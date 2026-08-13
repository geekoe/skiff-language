use skiff_runtime_model::vm_heap::VmHeapError;
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};

use crate::owner_inventory::{ChildOwnerLease, ChildOwnerRegistration, OwnerCreationError};

/// Failure to install another actual blocked-child owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnterChildError {
    CapacityExceeded,
    OwnerCreation(OwnerCreationError),
}

impl std::fmt::Display for EnterChildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CapacityExceeded => formatter.write_str("blocked child capacity is exhausted"),
            Self::OwnerCreation(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EnterChildError {}

/// One parent scheduler unit blocked on its active child.
#[derive(Debug)]
pub struct BlockedUnit<U, R> {
    parent: U,
    resume: R,
    owner_lease: Option<ChildOwnerLease>,
}

impl<U, R> BlockedUnit<U, R> {
    pub fn parent(&self) -> &U {
        &self.parent
    }

    pub fn resume(&self) -> &R {
        &self.resume
    }
}

impl<U, R> VmRootSource for BlockedUnit<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.parent.visit_roots(visitor)
    }
}

/// A flat stack of VM and adapter units.
///
/// Entering a child moves the current unit into `blocked` and installs the
/// child as the next active unit. Completing a child restores exactly one
/// parent. Neither operation invokes user code or recursively polls a unit.
#[derive(Debug)]
pub struct FlatTrampoline<U, R> {
    active: U,
    blocked: Vec<BlockedUnit<U, R>>,
    child_owners: ChildOwnerRegistration,
}

impl<U, R> FlatTrampoline<U, R> {
    pub fn new(root: U, child_owners: ChildOwnerRegistration) -> Self {
        Self {
            active: root,
            blocked: Vec::new(),
            child_owners,
        }
    }

    pub fn active(&self) -> &U {
        &self.active
    }

    pub fn active_mut(&mut self) -> &mut U {
        &mut self.active
    }

    pub fn blocked_depth(&self) -> usize {
        self.blocked.len()
    }

    pub fn enter_child(&mut self, child: U, resume: R) -> Result<(), EnterChildError> {
        self.blocked
            .try_reserve(1)
            .map_err(|_| EnterChildError::CapacityExceeded)?;
        let guard = self
            .child_owners
            .prepare()
            .map_err(EnterChildError::OwnerCreation)?;
        // The inventory lock is held from `prepare` until `commit`. Between the
        // two, only an unarmed placeholder is pushed: no caller code runs and
        // the guard's commit is infallible.
        let parent = std::mem::replace(&mut self.active, child);
        self.blocked.push(BlockedUnit {
            parent,
            resume,
            owner_lease: None,
        });
        let lease = guard.commit();
        self.blocked
            .last_mut()
            .expect("a blocked unit was just pushed")
            .owner_lease = Some(lease);
        Ok(())
    }

    pub fn complete_active<O>(mut self, outcome: O) -> TrampolineCompletion<U, R, O> {
        if let Some(BlockedUnit {
            parent,
            resume,
            owner_lease,
        }) = self.blocked.pop()
        {
            self.active = parent;
            let completion = TrampolineCompletion::ResumeParent(ParentResume {
                trampoline: self,
                resume,
                outcome,
            });
            drop(owner_lease);
            completion
        } else {
            TrampolineCompletion::RootComplete(outcome)
        }
    }

    pub fn suspend(self) -> SuspendedTrampoline<U, R> {
        SuspendedTrampoline {
            active: self.active,
            blocked: self.blocked,
            child_owners: self.child_owners,
        }
    }
}

impl<U, R> VmRootSource for FlatTrampoline<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.active.visit_roots(visitor)?;
        for blocked in &self.blocked {
            blocked.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// Result of completing exactly one active scheduler unit.
#[derive(Debug)]
pub enum TrampolineCompletion<U, R, O> {
    ResumeParent(ParentResume<U, R, O>),
    RootComplete(O),
}

/// Typed continuation and outcome to inject into the restored parent unit.
#[derive(Debug)]
pub struct ParentResume<U, R, O> {
    trampoline: FlatTrampoline<U, R>,
    resume: R,
    outcome: O,
}

impl<U, R, O> ParentResume<U, R, O> {
    pub fn trampoline(&self) -> &FlatTrampoline<U, R> {
        &self.trampoline
    }

    pub fn trampoline_mut(&mut self) -> &mut FlatTrampoline<U, R> {
        &mut self.trampoline
    }

    pub fn into_parts(self) -> (FlatTrampoline<U, R>, R, O) {
        (self.trampoline, self.resume, self.outcome)
    }
}

/// A trampoline moved into a pending owner.
///
/// This type is intentionally neither `Clone` nor `Copy`: there can be only
/// one runnable owner for an invocation chain.
#[must_use = "a suspended trampoline must be resumed or terminated"]
#[derive(Debug)]
pub struct SuspendedTrampoline<U, R> {
    active: U,
    blocked: Vec<BlockedUnit<U, R>>,
    child_owners: ChildOwnerRegistration,
}

impl<U, R> SuspendedTrampoline<U, R> {
    pub fn blocked_depth(&self) -> usize {
        self.blocked.len()
    }

    pub fn resume(self) -> FlatTrampoline<U, R> {
        FlatTrampoline {
            active: self.active,
            blocked: self.blocked,
            child_owners: self.child_owners,
        }
    }
}

impl<U, R> VmRootSource for SuspendedTrampoline<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.active.visit_roots(visitor)?;
        for blocked in &self.blocked {
            blocked.visit_roots(visitor)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{EnterChildError, FlatTrampoline, TrampolineCompletion};
    use crate::owner_inventory::{
        OwnerCreationErrorKind, OwnerDomain, RequestExecutionOwnerInventory,
    };

    #[test]
    fn deep_child_chain_uses_a_flat_vector() {
        const DEPTH: usize = 100_000;
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new(0usize, registrations.child());

        for child in 1..=DEPTH {
            trampoline.enter_child(child, child).unwrap();
        }
        assert_eq!(trampoline.blocked_depth(), DEPTH);

        for expected_parent in (0..DEPTH).rev() {
            let completion = trampoline.complete_active(());
            let TrampolineCompletion::ResumeParent(resume) = completion else {
                panic!("child must restore its parent");
            };
            let (next, resume, ()) = resume.into_parts();
            assert_eq!(resume, expected_parent + 1);
            trampoline = next;
            assert_eq!(*trampoline.active(), expected_parent);
        }

        assert!(matches!(
            trampoline.complete_active(()),
            TrampolineCompletion::RootComplete(())
        ));
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.child.current, 0);
        assert!(snapshot.child.ever_created);
    }

    #[test]
    fn suspension_moves_the_whole_chain_once() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new("root", registrations.child());
        trampoline.enter_child("child", "resume-root").unwrap();

        let suspended = trampoline.suspend();
        assert_eq!(suspended.blocked_depth(), 1);
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.child.current, 1);
        assert!(snapshot.child.ever_created);
        let resumed = suspended.resume();
        assert_eq!(resumed.active(), &"child");
        assert_eq!(resumed.blocked_depth(), 1);
    }

    #[test]
    fn frozen_inventory_rejects_child_without_installing_a_blocked_unit() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new("root", registrations.child());
        let snapshot = freeze.freeze();

        let error = match trampoline.enter_child("child", "resume-root") {
            Err(EnterChildError::OwnerCreation(error)) => error,
            other => panic!("expected an owner creation rejection, got {other:?}"),
        };
        assert_eq!(error.domain(), OwnerDomain::Child);
        assert_eq!(error.kind(), OwnerCreationErrorKind::InventoryFrozen);
        assert_eq!(trampoline.active(), &"root");
        assert_eq!(trampoline.blocked_depth(), 0);
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);
    }
}

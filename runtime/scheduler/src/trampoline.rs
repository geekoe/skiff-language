use skiff_runtime_model::vm_heap::VmHeapError;
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};

/// One parent scheduler unit blocked on its active child.
#[derive(Debug, PartialEq, Eq)]
pub struct BlockedUnit<U, R> {
    parent: U,
    resume: R,
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
#[derive(Debug, PartialEq, Eq)]
pub struct FlatTrampoline<U, R> {
    active: U,
    blocked: Vec<BlockedUnit<U, R>>,
}

impl<U, R> FlatTrampoline<U, R> {
    pub fn new(root: U) -> Self {
        Self {
            active: root,
            blocked: Vec::new(),
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

    pub fn enter_child(&mut self, child: U, resume: R) {
        let parent = std::mem::replace(&mut self.active, child);
        self.blocked.push(BlockedUnit { parent, resume });
    }

    pub fn complete_active<O>(mut self, outcome: O) -> TrampolineCompletion<U, R, O> {
        if let Some(BlockedUnit { parent, resume }) = self.blocked.pop() {
            self.active = parent;
            TrampolineCompletion::ResumeParent(ParentResume {
                trampoline: self,
                resume,
                outcome,
            })
        } else {
            TrampolineCompletion::RootComplete(outcome)
        }
    }

    pub fn suspend(self) -> SuspendedTrampoline<U, R> {
        SuspendedTrampoline {
            active: self.active,
            blocked: self.blocked,
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
#[derive(Debug, PartialEq, Eq)]
pub enum TrampolineCompletion<U, R, O> {
    ResumeParent(ParentResume<U, R, O>),
    RootComplete(O),
}

/// Typed continuation and outcome to inject into the restored parent unit.
#[derive(Debug, PartialEq, Eq)]
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
#[derive(Debug, PartialEq, Eq)]
pub struct SuspendedTrampoline<U, R> {
    active: U,
    blocked: Vec<BlockedUnit<U, R>>,
}

impl<U, R> SuspendedTrampoline<U, R> {
    pub fn blocked_depth(&self) -> usize {
        self.blocked.len()
    }

    pub fn resume(self) -> FlatTrampoline<U, R> {
        FlatTrampoline {
            active: self.active,
            blocked: self.blocked,
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
    use super::{FlatTrampoline, TrampolineCompletion};

    #[test]
    fn deep_child_chain_uses_a_flat_vector() {
        const DEPTH: usize = 100_000;
        let mut trampoline = FlatTrampoline::new(0usize);

        for child in 1..=DEPTH {
            trampoline.enter_child(child, child);
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
    }

    #[test]
    fn suspension_moves_the_whole_chain_once() {
        let mut trampoline = FlatTrampoline::new("root");
        trampoline.enter_child("child", "resume-root");

        let suspended = trampoline.suspend();
        assert_eq!(suspended.blocked_depth(), 1);
        let resumed = suspended.resume();
        assert_eq!(resumed.active(), &"child");
        assert_eq!(resumed.blocked_depth(), 1);
    }
}

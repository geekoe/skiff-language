use std::fmt;

use skiff_runtime_model::{
    memory_ledger::MemoryLease,
    vm_heap::{HeapDomainId, HeapEpoch, VmHeap, VmHeapError, VmHeapOperation},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueKind, ValueSlot},
};
use skiff_runtime_vm::VmBudget;

use crate::bytecode::{
    BytecodeSchedulerError, BytecodeUnit, BytecodeUnitControl, ChildFinish, ChildFinishError,
};
use crate::owner_inventory::{
    ChildHeapOwnerLease, ChildOwnerLease, ChildOwnerRegistration, OwnerCreationError,
};

/// Rejection from attaching a second pending cleanup to one owner bundle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChildHeapCleanupError;

impl fmt::Display for ChildHeapCleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("child heap carrier already owns a pending cleanup")
    }
}

impl std::error::Error for ChildHeapCleanupError {}

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

/// An owner-bearing rejection from [`FlatTrampoline::enter_child_with_finish`].
#[must_use = "a rejected child entry must be returned with its child bundle"]
pub struct EnterChildWithFinishError<U: BytecodeUnit, R> {
    error: EnterChildError,
    child: U,
    resume: R,
    child_heap: ChildHeapCarrier,
    finish: Box<dyn ChildFinish<U, R>>,
}

impl<U: BytecodeUnit, R> EnterChildWithFinishError<U, R> {
    fn capacity(
        child: U,
        resume: R,
        child_heap: ChildHeapCarrier,
        finish: Box<dyn ChildFinish<U, R>>,
    ) -> Self {
        Self {
            error: EnterChildError::CapacityExceeded,
            child,
            resume,
            child_heap,
            finish,
        }
    }

    fn owner_creation(
        error: OwnerCreationError,
        child: U,
        resume: R,
        child_heap: ChildHeapCarrier,
        finish: Box<dyn ChildFinish<U, R>>,
    ) -> Self {
        Self {
            error: EnterChildError::OwnerCreation(error),
            child,
            resume,
            child_heap,
            finish,
        }
    }

    pub const fn error(&self) -> EnterChildError {
        self.error
    }

    pub fn into_parts(
        self,
    ) -> (
        EnterChildError,
        U,
        R,
        ChildHeapCarrier,
        Box<dyn ChildFinish<U, R>>,
    ) {
        (
            self.error,
            self.child,
            self.resume,
            self.child_heap,
            self.finish,
        )
    }
}

impl<U: BytecodeUnit, R> fmt::Debug for EnterChildWithFinishError<U, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EnterChildWithFinishError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl<U: BytecodeUnit, R> fmt::Display for EnterChildWithFinishError<U, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl<U: BytecodeUnit, R> std::error::Error for EnterChildWithFinishError<U, R> {}

/// Observable lifecycle phase of one child heap carrier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildHeapState {
    Prepared,
    Staging,
    Terminal,
    Released,
}

/// Staging/terminal roots retained by a child heap carrier.
///
/// The carrier publishes destination roots here before they become visible to
/// the parent. Terminal roots remain enumerable through the carrier until the
/// request terminal path explicitly releases them against this carrier's own
/// heap.
struct BoundaryStaging {
    state: ChildHeapState,
    roots: Vec<ValueSlot>,
}

impl BoundaryStaging {
    fn new() -> Self {
        Self {
            state: ChildHeapState::Prepared,
            roots: Vec::new(),
        }
    }

    fn publish(&mut self, root: ValueSlot, heap: &mut dyn VmHeap) -> Result<(), VmHeapError> {
        if self.state == ChildHeapState::Released {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ValidateLive,
                message: "child heap staging is already released".to_string(),
            });
        }
        if root.as_handle().is_some() {
            heap.validate_live(&root)?;
        }
        if self.state == ChildHeapState::Prepared {
            self.state = ChildHeapState::Staging;
        }
        self.roots.push(root);
        Ok(())
    }

    fn mark_terminal(&mut self) {
        if matches!(
            self.state,
            ChildHeapState::Prepared | ChildHeapState::Staging
        ) {
            self.state = ChildHeapState::Terminal;
        }
    }

    fn release_all(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmHeapError> {
        while let Some(root) = self.roots.last().copied() {
            let result = match root.kind() {
                Some(ValueKind::RequestHeapRef) => heap.release_snapshot(&root),
                Some(ValueKind::ResourceRef) => heap.release_resource(&root),
                _ => Ok(()),
            };
            if result.is_err() {
                return result;
            }
            self.roots.pop();
        }
        self.state = ChildHeapState::Released;
        Ok(())
    }
}

impl fmt::Debug for BoundaryStaging {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BoundaryStaging")
            .field("state", &self.state)
            .field("root_count", &self.roots.len())
            .finish()
    }
}

/// One execution owner's heap, domain/epoch identity and memory/owner leases.
///
/// The carrier is the K6 owner-bundle primitive for the flat scheduler. It
/// keeps a concrete owner-local heap, its request-scoped domain/epoch, its
/// committed request memory lease and its child heap owner inventory lease in
/// one affine carrier. Published staging and terminal roots stay enumerable
/// as a `VmRootSource` and are released against this carrier's own heap before
/// the heap is dropped.
#[must_use = "a child heap carrier owns a heap, memory lease and owner lease"]
pub struct ChildHeapCarrier {
    heap: Box<dyn VmHeap + Send>,
    domain: HeapDomainId,
    epoch: HeapEpoch,
    memory_lease: MemoryLease,
    heap_owner_lease: ChildHeapOwnerLease,
    staging: BoundaryStaging,
    pending_cleanup: Option<Box<dyn FnOnce() + Send>>,
}

impl ChildHeapCarrier {
    pub fn new(
        heap: Box<dyn VmHeap + Send>,
        domain: HeapDomainId,
        epoch: HeapEpoch,
        memory_lease: MemoryLease,
        heap_owner_lease: ChildHeapOwnerLease,
    ) -> Self {
        Self {
            heap,
            domain,
            epoch,
            memory_lease,
            heap_owner_lease,
            staging: BoundaryStaging::new(),
            pending_cleanup: None,
        }
    }

    pub fn heap(&self) -> &dyn VmHeap {
        self.heap.as_ref()
    }

    pub fn heap_mut(&mut self) -> &mut dyn VmHeap {
        self.heap.as_mut()
    }

    pub const fn domain(&self) -> HeapDomainId {
        self.domain
    }

    pub const fn epoch(&self) -> HeapEpoch {
        self.epoch
    }

    pub const fn memory_lease(&self) -> &MemoryLease {
        &self.memory_lease
    }

    pub const fn state(&self) -> ChildHeapState {
        self.staging.state
    }

    pub fn staging_roots(&self) -> &[ValueSlot] {
        &self.staging.roots
    }

    pub fn publish_staging_root(&mut self, root: ValueSlot) -> Result<(), VmHeapError> {
        self.staging.publish(root, self.heap.as_mut())
    }

    pub fn mark_terminal(&mut self) {
        self.staging.mark_terminal();
    }

    /// Releases every published staging/terminal root exactly against this
    /// carrier's own heap.
    pub fn release_published_roots(&mut self) -> Result<(), VmHeapError> {
        self.staging.release_all(self.heap.as_mut())
    }

    /// Attaches the single pending cleanup authority for this owner bundle.
    ///
    /// D6R moves its transaction token cleanup here so a suspended/pending
    /// child keeps the cleanup with the same owner graph as its heap, memory
    /// lease and child owner lease. A second attachment fails closed.
    pub fn attach_pending_cleanup(
        &mut self,
        cleanup: Box<dyn FnOnce() + Send>,
    ) -> Result<(), ChildHeapCleanupError> {
        if self.pending_cleanup.is_some() {
            return Err(ChildHeapCleanupError);
        }
        self.pending_cleanup = Some(cleanup);
        Ok(())
    }

    /// Removes the pending cleanup for an exact successful commit/abort path.
    pub fn take_pending_cleanup(&mut self) -> Option<Box<dyn FnOnce() + Send>> {
        self.pending_cleanup.take()
    }
}

impl Drop for ChildHeapCarrier {
    fn drop(&mut self) {
        // Cleanup order is explicit here: staging/terminal roots are released
        // while the heap is still alive, then the heap drops, then the memory
        // lease releases its committed amount, then the child heap owner lease
        // releases its inventory count.
        let _ = self.release_published_roots();
        if let Some(cleanup) = self.pending_cleanup.take() {
            cleanup();
        }
    }
}

impl VmRootSource for ChildHeapCarrier {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for root in &self.staging.roots {
            visitor.visit_root(root)?;
        }
        Ok(())
    }
}

impl fmt::Debug for ChildHeapCarrier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildHeapCarrier")
            .field("domain", &self.domain)
            .field("epoch", &self.epoch)
            .field("memory_lease", &self.memory_lease)
            .field("staging", &self.staging)
            .field("pending_cleanup", &self.pending_cleanup.is_some())
            .finish_non_exhaustive()
    }
}

/// One parent scheduler unit blocked on its active child.
pub struct BlockedUnit<U: BytecodeUnit, R> {
    parent: U,
    resume: R,
    owner_lease: Option<ChildOwnerLease>,
    parent_heap: Option<ChildHeapCarrier>,
    finish: Option<Box<dyn ChildFinish<U, R>>>,
}

impl<U: BytecodeUnit, R: fmt::Debug> fmt::Debug for BlockedUnit<U, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BlockedUnit")
            .field("resume", &self.resume)
            .field("parent_heap", &self.parent_heap)
            .finish_non_exhaustive()
    }
}

impl<U: BytecodeUnit, R> BlockedUnit<U, R> {
    pub fn parent(&self) -> &U {
        &self.parent
    }

    pub fn resume(&self) -> &R {
        &self.resume
    }

    pub fn parent_heap(&self) -> Option<&ChildHeapCarrier> {
        self.parent_heap.as_ref()
    }
}

impl<U: BytecodeUnit, R> VmRootSource for BlockedUnit<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.parent.visit_roots(visitor)?;
        if let Some(heap) = &self.parent_heap {
            heap.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// A flat stack of VM and adapter units.
///
/// Entering a child moves the current unit into `blocked` and installs the
/// child as the next active unit. Completing a child restores exactly one
/// parent. Neither operation invokes user code or recursively polls a unit.
#[derive(Debug)]
pub struct FlatTrampoline<U: BytecodeUnit, R> {
    active: U,
    active_heap: Option<ChildHeapCarrier>,
    blocked: Vec<BlockedUnit<U, R>>,
    child_owners: ChildOwnerRegistration,
}

impl<U: BytecodeUnit, R> FlatTrampoline<U, R> {
    pub fn new(root: U, child_owners: ChildOwnerRegistration) -> Self {
        Self {
            active: root,
            active_heap: None,
            blocked: Vec::new(),
            child_owners,
        }
    }

    /// Builds a trampoline whose root execution unit owns a child heap carrier.
    ///
    /// The existing [`Self::new`] path remains available for single-heap
    /// callers that drive a request-owned heap outside the trampoline. Carrier
    /// callers use this constructor and never hand the parent heap to a child.
    pub(crate) fn with_child_heap(
        root: U,
        active_heap: ChildHeapCarrier,
        child_owners: ChildOwnerRegistration,
    ) -> Self {
        Self {
            active: root,
            active_heap: Some(active_heap),
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

    pub fn active_heap(&self) -> Option<&ChildHeapCarrier> {
        self.active_heap.as_ref()
    }

    pub fn active_heap_mut(&mut self) -> Option<&mut ChildHeapCarrier> {
        self.active_heap.as_mut()
    }

    pub fn blocked_depth(&self) -> usize {
        self.blocked.len()
    }

    /// Runs the active unit against its own child heap carrier when present.
    ///
    /// Single-heap callers continue to drive through `heap`; carrier callers
    /// never hand the parent heap to an active child.
    pub fn run_active_segment(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> BytecodeUnitControl<U> {
        if let Some(carrier) = self.active_heap.as_mut() {
            self.active.run_segment(carrier.heap_mut(), budget)
        } else {
            self.active.run_segment(heap, budget)
        }
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
        let parent_heap = self.active_heap.take();
        self.blocked.push(BlockedUnit {
            parent,
            resume,
            owner_lease: None,
            parent_heap,
            finish: None,
        });
        let lease = guard.commit();
        self.blocked
            .last_mut()
            .expect("a blocked unit was just pushed")
            .owner_lease = Some(lease);
        Ok(())
    }

    /// Enters a child after moving the current unit's heap carrier into the
    /// blocked parent slot.
    ///
    /// This is the owner-bundle publish point for the carrier lane: the parent
    /// heap is retained in the suspended chain, never handed to the child.
    pub fn enter_child_with_heap(
        &mut self,
        child: U,
        resume: R,
        child_heap: ChildHeapCarrier,
    ) -> Result<(), EnterChildError> {
        self.enter_child(child, resume)?;
        self.active_heap = Some(child_heap);
        Ok(())
    }

    /// Enters a child and keeps its exact finish continuation with the blocked
    /// parent.
    pub fn enter_child_with_finish(
        &mut self,
        child: U,
        resume: R,
        child_heap: ChildHeapCarrier,
        finish: Box<dyn ChildFinish<U, R>>,
    ) -> Result<(), EnterChildWithFinishError<U, R>> {
        if let Err(_) = self.blocked.try_reserve(1) {
            return Err(EnterChildWithFinishError::capacity(
                child, resume, child_heap, finish,
            ));
        }
        let guard = match self.child_owners.prepare() {
            Ok(guard) => guard,
            Err(error) => {
                return Err(EnterChildWithFinishError::owner_creation(
                    error, child, resume, child_heap, finish,
                ));
            }
        };
        let lease = guard.commit();
        let parent = std::mem::replace(&mut self.active, child);
        let parent_heap = self.active_heap.take();
        self.blocked.push(BlockedUnit {
            parent,
            resume,
            owner_lease: Some(lease),
            parent_heap,
            finish: Some(finish),
        });
        self.active_heap = Some(child_heap);
        Ok(())
    }

    /// Completes the active child after `ChildFinish` has materialized its
    /// result into the parent heap.
    ///
    /// The child heap remains active while `finish` runs, then is dropped when
    /// the parent heap is restored. A finish error returns the unchanged
    /// trampoline so the child heap and continuation stay in the same failure
    /// owner.
    pub fn complete_active_child(
        mut self,
        child_result: U::RootResult,
        budget: &mut dyn VmBudget,
        fallback_heap: &mut dyn VmHeap,
    ) -> Result<TrampolineCompletion<U, R, U::ResumeOutcome>, (ChildFinishError<U>, Self)> {
        let Some(mut blocked) = self.blocked.pop() else {
            unreachable!("child completion requires a blocked parent");
        };
        let Some(finish) = blocked.finish.take() else {
            self.blocked.push(blocked);
            return Err((
                ChildFinishError::result_retained(
                    BytecodeSchedulerError::UnsupportedChild,
                    child_result,
                ),
                self,
            ));
        };
        let BlockedUnit {
            parent: _,
            resume,
            owner_lease: _,
            parent_heap,
            finish: _,
        } = &mut blocked;
        let parent_heap = parent_heap
            .as_mut()
            .map(ChildHeapCarrier::heap_mut)
            .unwrap_or(fallback_heap);
        let child_heap = self
            .active_heap
            .as_mut()
            .expect("a child completion owns its active heap carrier");
        let outcome = match finish.finish(resume, child_result, child_heap, parent_heap, budget) {
            Ok(outcome) => outcome,
            Err(error) => {
                blocked.finish = Some(finish);
                self.blocked.push(blocked);
                return Err((error, self));
            }
        };

        drop(finish);
        let BlockedUnit {
            parent,
            resume,
            owner_lease,
            parent_heap,
            finish: _,
        } = blocked;
        self.active = parent;
        self.active_heap = parent_heap;
        let completion = TrampolineCompletion::ResumeParent(ParentResume {
            trampoline: self,
            resume,
            outcome,
        });
        drop(owner_lease);
        Ok(completion)
    }

    pub fn complete_active<O>(mut self, outcome: O) -> TrampolineCompletion<U, R, O> {
        if let Some(BlockedUnit {
            parent,
            resume,
            owner_lease,
            parent_heap,
            ..
        }) = self.blocked.pop()
        {
            self.active = parent;
            self.active_heap = parent_heap;
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
            active_heap: self.active_heap,
            blocked: self.blocked,
            child_owners: self.child_owners,
        }
    }
}

impl<U: BytecodeUnit, R> VmRootSource for FlatTrampoline<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.active.visit_roots(visitor)?;
        if let Some(heap) = &self.active_heap {
            heap.visit_roots(visitor)?;
        }
        for blocked in &self.blocked {
            blocked.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// Result of completing exactly one active scheduler unit.
#[derive(Debug)]
pub enum TrampolineCompletion<U: BytecodeUnit, R, O> {
    ResumeParent(ParentResume<U, R, O>),
    RootComplete(O),
}

/// Typed continuation and outcome to inject into the restored parent unit.
#[derive(Debug)]
pub struct ParentResume<U: BytecodeUnit, R, O> {
    trampoline: FlatTrampoline<U, R>,
    resume: R,
    outcome: O,
}

impl<U: BytecodeUnit, R, O> ParentResume<U, R, O> {
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
pub struct SuspendedTrampoline<U: BytecodeUnit, R> {
    active: U,
    active_heap: Option<ChildHeapCarrier>,
    blocked: Vec<BlockedUnit<U, R>>,
    child_owners: ChildOwnerRegistration,
}

impl<U: BytecodeUnit, R> SuspendedTrampoline<U, R> {
    pub fn blocked_depth(&self) -> usize {
        self.blocked.len()
    }

    pub fn active_heap(&self) -> Option<&ChildHeapCarrier> {
        self.active_heap.as_ref()
    }

    pub fn resume(self) -> FlatTrampoline<U, R> {
        FlatTrampoline {
            active: self.active,
            active_heap: self.active_heap,
            blocked: self.blocked,
            child_owners: self.child_owners,
        }
    }
}

impl<U: BytecodeUnit, R> VmRootSource for SuspendedTrampoline<U, R>
where
    U: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.active.visit_roots(visitor)?;
        if let Some(heap) = &self.active_heap {
            heap.visit_roots(visitor)?;
        }
        for blocked in &self.blocked {
            blocked.visit_roots(visitor)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{num::NonZeroU64, sync::Arc};

    use skiff_runtime_model::{
        memory_ledger::{MemoryLease, MemoryLeaseHost, MemoryLeaseToken},
        vm_heap::{HeapDomainId, HeapEpoch, VmHeap, VmHeapError},
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::{ValueKind, ValueSlot},
    };

    use super::{ChildHeapCarrier, EnterChildError, FlatTrampoline, TrampolineCompletion};
    use crate::bytecode::{
        BytecodeControl, BytecodeResumeFailure, BytecodeUnit, BytecodeUnitControl,
    };
    use crate::owner_inventory::{
        OwnerCreationErrorKind, OwnerDomain, RequestExecutionOwnerInventory,
    };
    use skiff_runtime_vm::VmBudget;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestUnit<T>(T);

    impl<T> VmRootSource for TestUnit<T> {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl<T> BytecodeUnit for TestUnit<T> {
        type ResumeToken = ();
        type ResumeOutcome = ();
        type RootResult = ();
        type ChildInvocation = ();
        type AdapterInvocation = ();
        type StreamItem = ();
        type PendingOperation = ();

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            BytecodeControl::Complete(())
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct TestHeap;

    impl VmHeap for TestHeap {
        fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
            if value.kind() == Some(ValueKind::RequestHeapRef)
                || value.kind() == Some(ValueKind::ResourceRef)
            {
                return Err(VmHeapError::HeapOperationFailed {
                    operation: skiff_runtime_model::vm_heap::VmHeapOperation::ValidateLive,
                    message: "test heap has no live physical handles".to_string(),
                });
            }
            Ok(())
        }

        fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    struct LeaseHost;

    impl MemoryLeaseHost for LeaseHost {
        fn release_lease(&self, _token: MemoryLeaseToken, _amount: usize) {}
    }

    fn lease(amount: usize) -> MemoryLease {
        MemoryLease::new(
            Arc::new(LeaseHost),
            MemoryLeaseToken::new(NonZeroU64::new(1).unwrap()),
            amount,
        )
    }

    fn carrier(
        domain: u64,
        heap_owner_lease: crate::owner_inventory::ChildHeapOwnerLease,
        amount: usize,
    ) -> ChildHeapCarrier {
        ChildHeapCarrier::new(
            Box::new(TestHeap),
            HeapDomainId::try_new(domain).unwrap(),
            HeapEpoch::new(0),
            lease(amount),
            heap_owner_lease,
        )
    }

    fn owner_lease() -> crate::owner_inventory::ChildHeapOwnerLease {
        let (registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.child_heap().mint_lease().unwrap()
    }

    #[test]
    fn child_heap_carrier_publishes_marks_terminal_and_releases_staging() {
        let mut heap = carrier(1, owner_lease(), 16);
        assert_eq!(heap.state(), super::ChildHeapState::Prepared);
        assert_eq!(heap.memory_lease().amount(), 16);
        assert_eq!(heap.domain().get(), 1);
        assert_eq!(heap.epoch().get(), 0);

        heap.publish_staging_root(ValueSlot::integer(7)).unwrap();
        assert_eq!(heap.state(), super::ChildHeapState::Staging);
        assert_eq!(
            heap.staging_roots()
                .iter()
                .map(ValueSlot::as_integer)
                .collect::<Vec<_>>(),
            vec![Some(7)]
        );

        struct CollectRoots(Vec<ValueSlot>);

        impl VmRootVisitor for CollectRoots {
            fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
                self.0.push(*root);
                Ok(())
            }
        }

        let mut roots = CollectRoots(Vec::new());
        heap.visit_roots(&mut roots).unwrap();
        assert_eq!(
            roots
                .0
                .iter()
                .map(ValueSlot::as_integer)
                .collect::<Vec<_>>(),
            vec![Some(7)]
        );

        heap.mark_terminal();
        assert_eq!(heap.state(), super::ChildHeapState::Terminal);
        heap.release_published_roots().unwrap();
        assert_eq!(heap.state(), super::ChildHeapState::Released);
        assert!(heap.staging_roots().is_empty());
    }

    #[test]
    fn trampoline_retains_active_and_blocked_child_heap_carriers() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, _freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::with_child_heap(
            TestUnit("root"),
            carrier(1, owner_lease(), 8),
            registrations.child(),
        );
        trampoline
            .enter_child_with_heap(
                TestUnit("child"),
                "resume-root",
                carrier(2, owner_lease(), 12),
            )
            .unwrap();

        assert_eq!(trampoline.active(), &TestUnit("child"));
        assert_eq!(trampoline.active_heap().unwrap().domain().get(), 2);
        assert_eq!(
            trampoline.blocked[0]
                .parent_heap
                .as_ref()
                .unwrap()
                .domain()
                .get(),
            1
        );

        let suspended = trampoline.suspend();
        assert_eq!(suspended.active_heap().unwrap().domain().get(), 2);
        assert_eq!(
            suspended.blocked[0]
                .parent_heap
                .as_ref()
                .unwrap()
                .domain()
                .get(),
            1
        );

        let mut resumed = suspended.resume();
        assert_eq!(resumed.active_heap().unwrap().domain().get(), 2);
        let completion = resumed.complete_active(());
        let TrampolineCompletion::ResumeParent(resume) = completion else {
            panic!("child must restore its parent");
        };
        let (restored, resume, outcome) = resume.into_parts();
        assert_eq!(resume, "resume-root");
        assert!(matches!(outcome, ()));
        assert_eq!(restored.active(), &TestUnit("root"));
        assert_eq!(restored.active_heap().unwrap().domain().get(), 1);
    }

    #[test]
    fn deep_child_chain_uses_a_flat_vector() {
        const DEPTH: usize = 100_000;
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new(TestUnit(0usize), registrations.child());

        for child in 1..=DEPTH {
            trampoline.enter_child(TestUnit(child), child).unwrap();
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
            assert_eq!(trampoline.active().0, expected_parent);
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
        let mut trampoline = FlatTrampoline::new(TestUnit("root"), registrations.child());
        trampoline
            .enter_child(TestUnit("child"), "resume-root")
            .unwrap();

        let suspended = trampoline.suspend();
        assert_eq!(suspended.blocked_depth(), 1);
        let snapshot = freeze.freeze();
        assert_eq!(snapshot.child.current, 1);
        assert!(snapshot.child.ever_created);
        let resumed = suspended.resume();
        assert_eq!(resumed.active(), &TestUnit("child"));
        assert_eq!(resumed.blocked_depth(), 1);
    }

    #[test]
    fn frozen_inventory_rejects_child_without_installing_a_blocked_unit() {
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new(TestUnit("root"), registrations.child());
        let snapshot = freeze.freeze();

        let error = match trampoline.enter_child(TestUnit("child"), "resume-root") {
            Err(EnterChildError::OwnerCreation(error)) => error,
            other => panic!("expected an owner creation rejection, got {other:?}"),
        };
        assert_eq!(error.domain(), OwnerDomain::Child);
        assert_eq!(error.kind(), OwnerCreationErrorKind::InventoryFrozen);
        assert_eq!(trampoline.active(), &TestUnit("root"));
        assert_eq!(trampoline.blocked_depth(), 0);
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);
    }
}

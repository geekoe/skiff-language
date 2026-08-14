//! Flat scheduler driver over one or more bytecode execution units.

use std::{fmt, sync::Arc};

use skiff_runtime_model::vm_heap::{VmHeap, VmHeapError};
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};
use skiff_runtime_vm::{
    AdapterInvocation as VmAdapterInvocation, ChildInvocation as VmChildInvocation, ChildTarget,
    PendingOperation as VmPendingOperation, ResumeOutcome, StreamItem as VmStreamItem,
    StreamItemReleaseError, VmBudget, VmControl, VmError, VmFiber, VmResult, VmResumeToken,
};

use crate::{
    owner_inventory::{ChildOwnerRegistration, OwnerCreationError},
    ClaimedPendingWakeGuard, EnterChildError, FlatTrampoline, PendingWake, RequestResourceRootPin,
    SuspendedTrampoline, TrampolineCompletion,
};

/// Failure modes owned by the bytecode scheduler.
#[derive(Debug)]
pub enum BytecodeSchedulerError {
    UnsupportedChild,
    UnsupportedAdapter,
    UnsupportedStream,
    UnsupportedPark,
    ChildCapacityExceeded,
    ChildOwnerCreation(OwnerCreationError),
    /// The scheduler rejected an emitted VM item, but its exact release
    /// failed. This variant retains the unique item carrier until the request
    /// terminal path can transfer its values into cleanup escrow.
    StreamItemRelease(StreamItemReleaseError),
    Vm(VmError),
    Port(String),
}

impl fmt::Display for BytecodeSchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedChild => formatter.write_str("bytecode child executor port is absent"),
            Self::UnsupportedAdapter => {
                formatter.write_str("bytecode adapter executor port is absent")
            }
            Self::UnsupportedStream => {
                formatter.write_str("bytecode stream supervisor port is absent")
            }
            Self::UnsupportedPark => formatter.write_str("bytecode park supervisor port is absent"),
            Self::ChildCapacityExceeded => {
                formatter.write_str("bytecode blocked child capacity is exhausted")
            }
            Self::ChildOwnerCreation(error) => error.fmt(formatter),
            Self::StreamItemRelease(error) => {
                write!(formatter, "bytecode stream item release failed: {error}")
            }
            Self::Vm(error) => write!(formatter, "bytecode VM unit failed: {error}"),
            Self::Port(message) => write!(formatter, "bytecode scheduler port failed: {message}"),
        }
    }
}

impl std::error::Error for BytecodeSchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Vm(error) => Some(error),
            Self::StreamItemRelease(error) => Some(error),
            Self::ChildOwnerCreation(error) => Some(error),
            _ => None,
        }
    }
}

impl VmRootSource for BytecodeSchedulerError {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::StreamItemRelease(error) => error.visit_roots(visitor),
            _ => Ok(()),
        }
    }
}

impl From<VmError> for BytecodeSchedulerError {
    fn from(error: VmError) -> Self {
        Self::Vm(error)
    }
}

impl From<String> for BytecodeSchedulerError {
    fn from(message: String) -> Self {
        Self::Port(message)
    }
}

impl From<EnterChildError> for BytecodeSchedulerError {
    fn from(error: EnterChildError) -> Self {
        match error {
            EnterChildError::CapacityExceeded => Self::ChildCapacityExceeded,
            EnterChildError::OwnerCreation(error) => Self::ChildOwnerCreation(error),
        }
    }
}

/// One control result returned by a bytecode execution unit.
#[derive(Debug, PartialEq, Eq)]
pub enum BytecodeControl<R, C, A, S, P> {
    Continue,
    Complete(R),
    EnterChild(C),
    EnterAdapter(A),
    EmitStream(S),
    Park(P),
}

/// Runtime-neutral unit contract implemented by `VmFiber` and by scheduler
/// fixtures. The unit controls remain typed so adapters can later implement
/// the same scheduler without introducing a second loop.
pub trait BytecodeUnit: VmRootSource {
    type ResumeToken;
    type ResumeOutcome;
    type RootResult;
    type ChildInvocation;
    type AdapterInvocation;
    type StreamItem;
    type PendingOperation;

    fn run_segment(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> BytecodeUnitControl<Self>;

    fn resume(
        &mut self,
        token: Self::ResumeToken,
        outcome: Self::ResumeOutcome,
    ) -> Result<(), BytecodeSchedulerError>;

    fn child_completion_to_resume_outcome(completed: Self::RootResult) -> Self::ResumeOutcome;

    /// Releases arguments from an adapter invocation rejected before any
    /// executor can consume it. Rootless scheduler fixtures use the default;
    /// the VM implementation routes exact linked plans through its lifecycle
    /// executor on the current heap thread.
    fn release_rejected_adapter_arguments(
        invocation: Self::AdapterInvocation,
        _heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeSchedulerError> {
        drop(invocation);
        Ok(())
    }

    /// Releases an emitted item rejected because no stream supervisor owns
    /// the request. VM units use the exact linked item plan on the current
    /// heap thread before the scheduler reports the missing port.
    fn release_rejected_stream_item(
        item: Self::StreamItem,
        _heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeSchedulerError> {
        drop(item);
        Ok(())
    }

    /// Reports whether a child invocation is a `StreamNext` consumer poll.
    ///
    /// Scheduler-neutral fixtures can override this with their own invocation
    /// vocabulary; the VM implementation identifies `ChildTarget::StreamNext`.
    fn is_stream_next_child(invocation: &Self::ChildInvocation) -> bool {
        let _ = invocation;
        false
    }
}

/// Typed control returned by one bytecode execution unit.
pub type BytecodeUnitControl<U> = BytecodeControl<
    <U as BytecodeUnit>::RootResult,
    <U as BytecodeUnit>::ChildInvocation,
    <U as BytecodeUnit>::AdapterInvocation,
    <U as BytecodeUnit>::StreamItem,
    <U as BytecodeUnit>::PendingOperation,
>;

/// A child unit and the unique continuation that restores its parent.
#[derive(Debug)]
pub struct BytecodeChildStart<U: BytecodeUnit> {
    pub unit: U,
    pub resume: U::ResumeToken,
}

/// One completed handoff plus the continuation that resumes the active unit.
#[derive(Debug)]
pub struct BytecodeHandoff<U: BytecodeUnit> {
    pub resume: U::ResumeToken,
    pub outcome: U::ResumeOutcome,
}

/// Result of one adapter invocation.
///
/// `Ready` gives the scheduler a continuation handoff to inject immediately.
/// `Pending` means the adapter owns a real host operation and must publish the
/// returned actual-`Pending` operation through [`BytecodeChildExecutor::park_adapter`].
pub enum BytecodeAdapterHandoff<U: BytecodeUnit> {
    Ready(BytecodeHandoff<U>),
    Pending(U::PendingOperation),
}

/// Result of handing one stream item to the supervisor.
///
/// `Ready` gives the scheduler a continuation handoff to inject immediately.
/// `Pending` means the item is supervisor-owned and the active producer must
/// park with the returned actual-`Pending` operation.
pub enum BytecodeStreamHandoff<U: BytecodeUnit> {
    Ready(BytecodeHandoff<U>),
    Pending(U::PendingOperation),
}

/// Port used to start child fibers and run host adapters once.
pub trait BytecodeChildExecutor<U: BytecodeUnit>: Send + Sync + 'static {
    fn execute_child(
        &self,
        invocation: U::ChildInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<U>, BytecodeSchedulerError>;

    fn execute_adapter(
        &self,
        invocation: U::AdapterInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<U>, BytecodeSchedulerError>;

    /// Publishes an adapter-owned actual-`Pending` operation.
    ///
    /// Implementations that never return `BytecodeAdapterHandoff::Pending`
    /// may leave this default in place.
    fn park_adapter(
        &self,
        operation: U::PendingOperation,
        suspended: SuspendedTrampoline<U, U::ResumeToken>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let _ = (operation, suspended, heap, budget);
        Err(BytecodeSchedulerError::UnsupportedPark)
    }

    /// Polls one `StreamNext` consumer child.
    ///
    /// `Ready` resumes the parent with the consumer's item, natural end or
    /// error. `Pending` preserves the item continuation in an actual pending
    /// operation so a later consumer wake can settle with the correct outcome.
    /// Implementations that do not handle stream consumers must return
    /// `UnsupportedChild`.
    fn execute_stream_next(
        &self,
        invocation: U::ChildInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<U>, BytecodeSchedulerError> {
        let _ = (invocation, heap, budget);
        Err(BytecodeSchedulerError::UnsupportedChild)
    }

    /// Publishes the stream-consumer pending owner produced by
    /// [`Self::execute_stream_next`].
    fn park_stream_next(
        &self,
        operation: U::PendingOperation,
        suspended: SuspendedTrampoline<U, U::ResumeToken>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let _ = (operation, suspended, heap, budget);
        Err(BytecodeSchedulerError::UnsupportedPark)
    }
}

/// Port used for stream emission and actual-Pending parking.
pub trait BytecodeStreamSupervisor<U: BytecodeUnit>: Send + Sync + 'static {
    /// Synchronous handoff retained for supervisors that do not yet model
    /// backpressure. The scheduler uses [`Self::emit_stream_handoff`].
    fn emit_stream(
        &self,
        item: U::StreamItem,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeHandoff<U>, BytecodeSchedulerError> {
        let _ = (item, heap, budget);
        Err(BytecodeSchedulerError::UnsupportedStream)
    }

    /// Emits one item and reports whether the producer can continue immediately
    /// or must park with a real backpressure operation.
    fn emit_stream_handoff(
        &self,
        item: U::StreamItem,
        _depth: usize,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<U>, BytecodeSchedulerError> {
        self.emit_stream(item, heap, budget)
            .map(BytecodeStreamHandoff::Ready)
    }

    fn park(
        &self,
        operation: U::PendingOperation,
        suspended: SuspendedTrampoline<U, U::ResumeToken>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError>;

    /// Notifies the supervisor that the unit at `depth` completed.
    ///
    /// A producer that emitted at this depth normally transitions to a stream
    /// terminal here. Implementations that do not supervise the completed unit
    /// must return `Ok(())` without changing state.
    fn finish_stream(
        &self,
        depth: usize,
        result: &U::RootResult,
    ) -> Result<(), BytecodeSchedulerError> {
        let _ = (depth, result);
        Ok(())
    }
}

/// Execution ports supplied to a [`BytecodeScheduler`].
///
/// An absent port fails closed before any child, adapter, stream or park
/// invocation can run.
pub struct BytecodeSchedulerPorts<U: BytecodeUnit> {
    pub child_executor: Option<Arc<dyn BytecodeChildExecutor<U>>>,
    pub stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<U>>>,
}

impl<U: BytecodeUnit> Clone for BytecodeSchedulerPorts<U> {
    fn clone(&self) -> Self {
        Self {
            child_executor: self.child_executor.clone(),
            stream_supervisor: self.stream_supervisor.clone(),
        }
    }
}

impl<U: BytecodeUnit> Default for BytecodeSchedulerPorts<U> {
    fn default() -> Self {
        Self {
            child_executor: None,
            stream_supervisor: None,
        }
    }
}

/// Terminal result of a scheduler drive.
pub enum BytecodeSchedulerOutcome<U: BytecodeUnit> {
    Complete(U::RootResult),
    Parked,
}

impl<U: BytecodeUnit> fmt::Debug for BytecodeSchedulerOutcome<U> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Complete(_) => formatter.write_str("Complete(..)"),
            Self::Parked => formatter.write_str("Parked"),
        }
    }
}

/// Flat bytecode scheduler over a `FlatTrampoline` of execution units.
pub struct BytecodeScheduler<U: BytecodeUnit> {
    trampoline: FlatTrampoline<U, U::ResumeToken>,
    ports: BytecodeSchedulerPorts<U>,
    resource_roots: Option<RequestResourceRootPin>,
}

impl<U> BytecodeScheduler<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    pub(crate) fn new(
        root: U,
        ports: BytecodeSchedulerPorts<U>,
        child_owners: ChildOwnerRegistration,
    ) -> Self {
        Self {
            trampoline: FlatTrampoline::new(root, child_owners),
            ports,
            resource_roots: None,
        }
    }

    pub(crate) fn new_with_resource_roots(
        root: U,
        ports: BytecodeSchedulerPorts<U>,
        child_owners: ChildOwnerRegistration,
        resource_roots: RequestResourceRootPin,
    ) -> Self {
        Self {
            trampoline: FlatTrampoline::new(root, child_owners),
            ports,
            resource_roots: Some(resource_roots),
        }
    }

    pub fn blocked_depth(&self) -> usize {
        self.trampoline.blocked_depth()
    }

    pub fn active(&self) -> &U {
        self.trampoline.active()
    }

    pub fn active_mut(&mut self) -> &mut U {
        self.trampoline.active_mut()
    }

    pub(crate) fn bind_request_resource_roots(
        mut self,
        resource_roots: RequestResourceRootPin,
    ) -> Result<Self, BytecodeSchedulerError> {
        if self
            .resource_roots
            .as_ref()
            .is_some_and(|current| !current.is_same_table(&resource_roots))
        {
            return Err(BytecodeSchedulerError::Port(
                "pending scheduler belongs to a different request resource table".to_string(),
            ));
        }
        self.resource_roots = Some(resource_roots);
        Ok(self)
    }

    /// Drives the active unit until a root completion or a real park.
    ///
    /// Child completion restores exactly one parent. The scheduler never calls
    /// `run_segment` or `resume` recursively.
    pub fn run(
        mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeSchedulerOutcome<U>, BytecodeSchedulerError> {
        loop {
            let control = self.trampoline.active_mut().run_segment(heap, budget);
            match control {
                BytecodeControl::Continue => {}
                BytecodeControl::Complete(result) => {
                    let depth = self.trampoline.blocked_depth();
                    if let Some(supervisor) = self.ports.stream_supervisor.as_ref() {
                        supervisor.finish_stream(depth, &result)?;
                    }
                    if self.trampoline.blocked_depth() == 0 {
                        return Ok(BytecodeSchedulerOutcome::Complete(result));
                    }
                    let outcome = U::child_completion_to_resume_outcome(result);
                    let completion = self.trampoline.complete_active(outcome);
                    let TrampolineCompletion::ResumeParent(parent) = completion else {
                        unreachable!("depth check guarantees a parent unit exists");
                    };
                    let (mut trampoline, resume, outcome) = parent.into_parts();
                    trampoline.active_mut().resume(resume, outcome)?;
                    self.trampoline = trampoline;
                }
                BytecodeControl::EnterChild(invocation) => {
                    let executor = self
                        .ports
                        .child_executor
                        .as_ref()
                        .ok_or(BytecodeSchedulerError::UnsupportedChild)?;
                    if U::is_stream_next_child(&invocation) {
                        match executor.execute_stream_next(invocation, heap, budget)? {
                            BytecodeStreamHandoff::Ready(handoff) => {
                                self.trampoline
                                    .active_mut()
                                    .resume(handoff.resume, handoff.outcome)?;
                            }
                            BytecodeStreamHandoff::Pending(operation) => {
                                let suspended = self.trampoline.suspend();
                                executor.park_stream_next(operation, suspended, heap, budget)?;
                                return Ok(BytecodeSchedulerOutcome::Parked);
                            }
                        }
                        continue;
                    }
                    let start = executor.execute_child(invocation, heap, budget)?;
                    self.trampoline.enter_child(start.unit, start.resume)?;
                }
                BytecodeControl::EnterAdapter(invocation) => {
                    let Some(executor) = self.ports.child_executor.as_ref() else {
                        U::release_rejected_adapter_arguments(invocation, heap)?;
                        return Err(BytecodeSchedulerError::UnsupportedAdapter);
                    };
                    match executor.execute_adapter(invocation, heap, budget)? {
                        BytecodeAdapterHandoff::Ready(handoff) => {
                            self.trampoline
                                .active_mut()
                                .resume(handoff.resume, handoff.outcome)?;
                        }
                        BytecodeAdapterHandoff::Pending(operation) => {
                            let suspended = self.trampoline.suspend();
                            executor.park_adapter(operation, suspended, heap, budget)?;
                            return Ok(BytecodeSchedulerOutcome::Parked);
                        }
                    }
                }
                BytecodeControl::EmitStream(item) => {
                    let Some(supervisor) = self.ports.stream_supervisor.as_ref() else {
                        U::release_rejected_stream_item(item, heap)?;
                        return Err(BytecodeSchedulerError::UnsupportedStream);
                    };
                    let depth = self.trampoline.blocked_depth();
                    match supervisor.emit_stream_handoff(item, depth, heap, budget)? {
                        BytecodeStreamHandoff::Ready(handoff) => {
                            self.trampoline
                                .active_mut()
                                .resume(handoff.resume, handoff.outcome)?;
                        }
                        BytecodeStreamHandoff::Pending(operation) => {
                            let suspended = self.trampoline.suspend();
                            supervisor.park(operation, suspended, heap, budget)?;
                            return Ok(BytecodeSchedulerOutcome::Parked);
                        }
                    }
                }
                BytecodeControl::Park(operation) => {
                    let supervisor = self
                        .ports
                        .stream_supervisor
                        .as_ref()
                        .ok_or(BytecodeSchedulerError::UnsupportedPark)?;
                    let suspended = self.trampoline.suspend();
                    supervisor.park(operation, suspended, heap, budget)?;
                    return Ok(BytecodeSchedulerOutcome::Parked);
                }
            }
        }
    }

    /// Restores a scheduler from a completed `PendingWake`.
    ///
    /// This consumes the wake and restores its escrowed roots exactly once
    /// before resuming the leaf unit with the winning settlement.
    pub fn resume_from_pending_wake(
        wake: PendingWake<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, U::ResumeOutcome>,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<Self, BytecodeSchedulerError>
    where
        U::ResumeOutcome: VmRootSource,
    {
        Self::resume_from_claimed_pending_wake_with(wake.claim(), ports, |_, outcome| outcome)
    }

    /// Restores a scheduler from a claimed wake whose heap-free settlement
    /// must first be materialized on the request resume thread.
    ///
    /// The pending owner lease stays inside scheduler authority until after
    /// the mapped outcome has restored the original VM site. The mapper may
    /// inspect the non-forgeable resume token but cannot access or duplicate
    /// the private inventory lease.
    pub fn resume_from_pending_wake_with<O>(
        wake: PendingWake<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, O>,
        ports: BytecodeSchedulerPorts<U>,
        map: impl FnOnce(&U::ResumeToken, O) -> U::ResumeOutcome,
    ) -> Result<Self, BytecodeSchedulerError>
    where
        O: VmRootSource,
        U::ResumeOutcome: VmRootSource,
    {
        Self::resume_from_claimed_pending_wake_with(wake.claim(), ports, map)
    }

    /// Maps and resumes a queue-claimed wake without decomposing its root
    /// graph across the heap materialization callback.
    pub fn resume_from_claimed_pending_wake_with<O>(
        wake: ClaimedPendingWakeGuard<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, O>,
        ports: BytecodeSchedulerPorts<U>,
        map: impl FnOnce(&U::ResumeToken, O) -> U::ResumeOutcome,
    ) -> Result<Self, BytecodeSchedulerError>
    where
        O: VmRootSource,
        U::ResumeOutcome: VmRootSource,
    {
        let mapped = wake
            .map(|resume, outcome, _roots| map(resume, outcome))
            .map_suspended(SuspendedTrampoline::resume);
        mapped.resume_and_commit(
            |trampoline, resume, outcome| trampoline.active_mut().resume(resume, outcome),
            |trampoline, resource_roots| Self {
                trampoline,
                ports,
                resource_roots,
            },
        )
    }
}

impl<U> VmRootSource for BytecodeScheduler<U>
where
    U: BytecodeUnit + VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.trampoline.visit_roots(visitor)?;
        if let Some(resource_roots) = &self.resource_roots {
            resource_roots.visit_roots(visitor)?;
        }
        Ok(())
    }
}

impl BytecodeUnit for VmFiber {
    type ResumeToken = VmResumeToken;
    type ResumeOutcome = ResumeOutcome;
    type RootResult = VmResult;
    type ChildInvocation = VmChildInvocation;
    type AdapterInvocation = VmAdapterInvocation;
    type StreamItem = VmStreamItem;
    type PendingOperation = VmPendingOperation;

    fn run_segment(
        &mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> BytecodeUnitControl<VmFiber> {
        match VmFiber::run_segment(self, heap, budget) {
            VmControl::Continue => BytecodeControl::Continue,
            VmControl::Complete(result) => BytecodeControl::Complete(result),
            VmControl::EnterChild(invocation) => BytecodeControl::EnterChild(invocation),
            VmControl::EnterAdapter(invocation) => BytecodeControl::EnterAdapter(invocation),
            VmControl::EmitStream(item) => BytecodeControl::EmitStream(item),
            VmControl::Park(operation) => BytecodeControl::Park(operation),
        }
    }

    fn resume(
        &mut self,
        token: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> Result<(), BytecodeSchedulerError> {
        VmFiber::resume(self, token, outcome).map_err(BytecodeSchedulerError::from)
    }

    fn child_completion_to_resume_outcome(completed: VmResult) -> ResumeOutcome {
        match completed {
            Ok(values) => ResumeOutcome::Values(values),
            // A child's ordinary throw is not a terminal failure: the exact
            // opaque envelope crosses the child boundary so the parent can
            // resume its own unwind with the unchanged identity.
            Err(VmError::Thrown(envelope)) => ResumeOutcome::Throw(envelope),
            Err(error) => ResumeOutcome::Failure(error),
        }
    }

    fn release_rejected_adapter_arguments(
        invocation: VmAdapterInvocation,
        heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeSchedulerError> {
        let (_, arguments, _) = invocation.into_parts();
        arguments
            .release(heap)
            .map_err(BytecodeSchedulerError::from)
    }

    fn release_rejected_stream_item(
        item: VmStreamItem,
        heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeSchedulerError> {
        item.release(heap)
            .map(|_resume| ())
            .map_err(BytecodeSchedulerError::StreamItemRelease)
    }

    fn is_stream_next_child(invocation: &VmChildInvocation) -> bool {
        invocation.target() == ChildTarget::StreamNext
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use skiff_runtime_model::{
        vm_heap::{VmHeap, VmHeapError},
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };
    use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmSemanticCharge};

    use super::*;
    use crate::{
        owner_inventory::{
            ChildOwnerRegistration, OwnerCreationErrorKind, OwnerDomain, PendingOwnerRegistration,
            RequestExecutionOwnerInventory,
        },
        PendingOwnerDraft, PendingPublication, PendingRegistry, PendingWake, PendingWakeQueue,
        RequestByteStreamPullFuture, RequestByteStreamPullStartError, RequestByteStreamSource,
        RequestExecutionContext, RequestResourceTermination, RootDisposition, RootEscrow,
        RootEscrowBacking, SettleDisposition, SettlementSource,
    };

    fn pending_registration() -> PendingOwnerRegistration {
        let (mut registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.take_pending().unwrap()
    }

    fn child_registration() -> ChildOwnerRegistration {
        let (registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.child()
    }

    struct NoopHeap;

    impl VmHeap for NoopHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
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

    struct NoopBudget;

    impl VmBudget for NoopBudget {
        fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
            Ok(())
        }

        fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
            Ok(())
        }
    }

    type TestControl = BytecodeControl<usize, usize, usize, usize, usize>;
    type TestSuspended = SuspendedTrampoline<TestUnit, usize>;
    type TestWake = PendingWake<usize, TestSuspended, TestResumeOutcome>;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TestResumeOutcome(usize);

    impl VmRootSource for TestResumeOutcome {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    struct RejectedAdapterProbe {
        release_count: Arc<AtomicUsize>,
        emitted: bool,
    }

    impl VmRootSource for RejectedAdapterProbe {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for RejectedAdapterProbe {
        type ResumeToken = usize;
        type ResumeOutcome = TestResumeOutcome;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = Arc<AtomicUsize>;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            if self.emitted {
                BytecodeControl::Complete(0)
            } else {
                self.emitted = true;
                BytecodeControl::EnterAdapter(Arc::clone(&self.release_count))
            }
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: Self::RootResult) -> Self::ResumeOutcome {
            TestResumeOutcome(completed)
        }

        fn release_rejected_adapter_arguments(
            invocation: Self::AdapterInvocation,
            _heap: &mut dyn VmHeap,
        ) -> Result<(), BytecodeSchedulerError> {
            invocation.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct RejectedStreamProbe {
        release_count: Arc<AtomicUsize>,
        emitted: bool,
    }

    impl VmRootSource for RejectedStreamProbe {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for RejectedStreamProbe {
        type ResumeToken = usize;
        type ResumeOutcome = TestResumeOutcome;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = Arc<AtomicUsize>;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            if self.emitted {
                BytecodeControl::Complete(0)
            } else {
                self.emitted = true;
                BytecodeControl::EmitStream(Arc::clone(&self.release_count))
            }
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: Self::RootResult) -> Self::ResumeOutcome {
            TestResumeOutcome(completed)
        }

        fn release_rejected_stream_item(
            item: Self::StreamItem,
            _heap: &mut dyn VmHeap,
        ) -> Result<(), BytecodeSchedulerError> {
            item.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestUnit {
        control: Option<TestControl>,
        resumed: Option<(usize, TestResumeOutcome)>,
        finish_after_resume: Option<usize>,
    }

    impl TestUnit {
        fn parked(operation: usize) -> Self {
            Self {
                control: Some(TestControl::Park(operation)),
                resumed: None,
                finish_after_resume: None,
            }
        }

        fn emit(item: usize, finish: usize) -> Self {
            Self {
                control: Some(TestControl::EmitStream(item)),
                resumed: None,
                finish_after_resume: Some(finish),
            }
        }
    }

    impl VmRootSource for TestUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for TestUnit {
        type ResumeToken = usize;
        type ResumeOutcome = TestResumeOutcome;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> TestControl {
            if let Some((_, outcome)) = self.resumed.take() {
                TestControl::Complete(self.finish_after_resume.take().unwrap_or(outcome.0))
            } else if let Some(control) = self.control.take() {
                control
            } else {
                TestControl::Complete(0)
            }
        }

        fn resume(
            &mut self,
            token: usize,
            outcome: TestResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: usize) -> TestResumeOutcome {
            TestResumeOutcome(completed)
        }
    }

    struct TestStreamSupervisor {
        parked: Mutex<Option<(usize, TestSuspended)>>,
        emitted: Mutex<Vec<usize>>,
    }

    impl BytecodeStreamSupervisor<TestUnit> for TestStreamSupervisor {
        fn emit_stream_handoff(
            &self,
            item: usize,
            _depth: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<TestUnit>, BytecodeSchedulerError> {
            self.emitted.lock().unwrap().push(item);
            Ok(BytecodeStreamHandoff::Pending(item))
        }

        fn park(
            &self,
            operation: usize,
            suspended: TestSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.parked.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct EmptyRoots;

    impl VmRootSource for EmptyRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for EmptyRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {}

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
    }

    struct TestWakeQueue(Mutex<Vec<TestWake>>);

    impl PendingWakeQueue<usize, TestSuspended, TestResumeOutcome> for TestWakeQueue {
        fn enqueue(&self, wake: TestWake) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[derive(Debug)]
    struct ResumeThenChildUnit {
        state: ResumeThenChildState,
    }

    #[derive(Clone, Copy, Debug)]
    enum ResumeThenChildState {
        Park,
        EnterChild,
        Complete,
    }

    impl VmRootSource for ResumeThenChildUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for ResumeThenChildUnit {
        type ResumeToken = usize;
        type ResumeOutcome = TestResumeOutcome;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            match std::mem::replace(&mut self.state, ResumeThenChildState::Complete) {
                ResumeThenChildState::Park => BytecodeControl::Park(7),
                ResumeThenChildState::EnterChild => BytecodeControl::EnterChild(0),
                ResumeThenChildState::Complete => BytecodeControl::Complete(0),
            }
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.state = ResumeThenChildState::EnterChild;
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: usize) -> TestResumeOutcome {
            TestResumeOutcome(completed)
        }
    }

    type ResumeThenChildSuspended = SuspendedTrampoline<ResumeThenChildUnit, usize>;
    type ResumeThenChildWake = PendingWake<usize, ResumeThenChildSuspended, TestResumeOutcome>;

    struct ResumeThenChildSupervisor(Mutex<Option<(usize, ResumeThenChildSuspended)>>);

    impl BytecodeStreamSupervisor<ResumeThenChildUnit> for ResumeThenChildSupervisor {
        fn park(
            &self,
            operation: usize,
            suspended: ResumeThenChildSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.0.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct ResumeThenChildQueue(Mutex<Vec<ResumeThenChildWake>>);

    impl PendingWakeQueue<usize, ResumeThenChildSuspended, TestResumeOutcome> for ResumeThenChildQueue {
        fn enqueue(&self, wake: ResumeThenChildWake) {
            self.0.lock().unwrap().push(wake);
        }
    }

    struct ResumeThenChildExecutor(AtomicUsize);

    impl BytecodeChildExecutor<ResumeThenChildUnit> for ResumeThenChildExecutor {
        fn execute_child(
            &self,
            invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildStart<ResumeThenChildUnit>, BytecodeSchedulerError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(BytecodeChildStart {
                unit: ResumeThenChildUnit {
                    state: ResumeThenChildState::Complete,
                },
                resume: invocation,
            })
        }

        fn execute_adapter(
            &self,
            _invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<ResumeThenChildUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum PendingStreamOutcome {
        End,
    }

    impl VmRootSource for PendingStreamOutcome {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct PendingStreamNextUnit {
        entered: bool,
        resumed: Option<PendingStreamOutcome>,
    }

    impl VmRootSource for PendingStreamNextUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for PendingStreamNextUnit {
        type ResumeToken = usize;
        type ResumeOutcome = PendingStreamOutcome;
        type RootResult = PendingStreamOutcome;
        type ChildInvocation = ();
        type AdapterInvocation = ();
        type StreamItem = ();
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            if let Some(outcome) = self.resumed.take() {
                BytecodeControl::Complete(outcome)
            } else if !self.entered {
                self.entered = true;
                BytecodeControl::EnterChild(())
            } else {
                BytecodeControl::Complete(PendingStreamOutcome::End)
            }
        }

        fn resume(
            &mut self,
            _token: usize,
            outcome: PendingStreamOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some(outcome);
            Ok(())
        }

        fn child_completion_to_resume_outcome(
            completed: PendingStreamOutcome,
        ) -> PendingStreamOutcome {
            completed
        }

        fn is_stream_next_child(_invocation: &()) -> bool {
            true
        }
    }

    type PendingStreamSuspended = SuspendedTrampoline<PendingStreamNextUnit, usize>;
    type PendingStreamWake = PendingWake<usize, PendingStreamSuspended, PendingStreamOutcome>;

    struct PendingStreamExecutor(Mutex<Option<(usize, PendingStreamSuspended)>>);

    impl BytecodeChildExecutor<PendingStreamNextUnit> for PendingStreamExecutor {
        fn execute_child(
            &self,
            _invocation: (),
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildStart<PendingStreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedChild)
        }

        fn execute_adapter(
            &self,
            _invocation: (),
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<PendingStreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        }

        fn execute_stream_next(
            &self,
            _invocation: (),
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<PendingStreamNextUnit>, BytecodeSchedulerError> {
            Ok(BytecodeStreamHandoff::Pending(13))
        }

        fn park_stream_next(
            &self,
            operation: usize,
            suspended: PendingStreamSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.0.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct PendingStreamQueue(Mutex<Vec<PendingStreamWake>>);

    impl PendingWakeQueue<usize, PendingStreamSuspended, PendingStreamOutcome> for PendingStreamQueue {
        fn enqueue(&self, wake: PendingStreamWake) {
            self.0.lock().unwrap().push(wake);
        }
    }

    #[derive(Debug)]
    struct RejectResumeUnit;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct RejectResumeOutcome(usize);

    impl VmRootSource for RejectResumeOutcome {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl VmRootSource for RejectResumeUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for RejectResumeUnit {
        type ResumeToken = usize;
        type ResumeOutcome = RejectResumeOutcome;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            BytecodeControl::Complete(0)
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::Port(
                "intentional resume rejection".to_string(),
            ))
        }

        fn child_completion_to_resume_outcome(completed: Self::RootResult) -> Self::ResumeOutcome {
            RejectResumeOutcome(completed)
        }
    }

    type RejectWake =
        PendingWake<usize, SuspendedTrampoline<RejectResumeUnit, usize>, RejectResumeOutcome>;

    struct RejectWakeQueue(Mutex<Vec<RejectWake>>);

    impl PendingWakeQueue<usize, SuspendedTrampoline<RejectResumeUnit, usize>, RejectResumeOutcome>
        for RejectWakeQueue
    {
        fn enqueue(&self, wake: RejectWake) {
            self.0.lock().unwrap().push(wake);
        }
    }

    struct DispositionRoots(Arc<Mutex<Vec<RootDisposition>>>);

    struct TerminatingByteStream {
        root: ValueSlot,
        terminations: Arc<Mutex<Vec<RequestResourceTermination>>>,
    }

    impl VmRootSource for TerminatingByteStream {
        fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            visitor.visit_root(&self.root)
        }
    }

    impl RequestByteStreamSource for TerminatingByteStream {
        fn start_pull(
            &self,
        ) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
            Ok(Box::pin(std::future::pending()))
        }

        fn terminate(self: Box<Self>, termination: RequestResourceTermination) {
            self.terminations.lock().unwrap().push(termination);
        }
    }

    impl VmRootSource for DispositionRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for DispositionRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {
            panic!("terminal settlement must not restore transferred roots")
        }

        fn drop_roots(self: Box<Self>, disposition: RootDisposition) {
            self.0.lock().unwrap().push(disposition);
        }
    }

    fn assert_terminal_root_disposition(source: SettlementSource, expected: RootDisposition) {
        let suspended = FlatTrampoline::new(RejectResumeUnit, child_registration()).suspend();
        let registry = PendingRegistry::<
            usize,
            SuspendedTrampoline<RejectResumeUnit, usize>,
            RejectResumeOutcome,
        >::new(pending_registration());
        let events = Arc::new(Mutex::new(Vec::new()));
        let completion = registry
            .begin(RootEscrow::new(Box::new(DispositionRoots(Arc::clone(
                &events,
            )))))
            .unwrap();
        let queue = Arc::new(RejectWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<
            dyn PendingWakeQueue<
                usize,
                SuspendedTrampoline<RejectResumeUnit, usize>,
                RejectResumeOutcome,
            >,
        > = queue.clone();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(9, suspended),
                wake_queue,
            )
            .unwrap();
        match source {
            SettlementSource::Cancellation => {
                let _ = completion.cancel(RejectResumeOutcome(0));
            }
            SettlementSource::Deadline => {
                let _ = completion.deadline(RejectResumeOutcome(0));
            }
            SettlementSource::InternalStop => {
                let _ = completion.internal_stop(RejectResumeOutcome(0));
            }
            SettlementSource::HostCompletion => unreachable!(),
        }
        let claimed = queue.0.lock().unwrap().pop().unwrap().claim();
        assert!(
            BytecodeScheduler::<RejectResumeUnit>::resume_from_claimed_pending_wake_with(
                claimed,
                BytecodeSchedulerPorts::default(),
                |_, outcome| outcome,
            )
            .is_err()
        );
        assert_eq!(*events.lock().unwrap(), [expected]);
    }

    #[test]
    fn phase_5_resource_terminal_winners_discard_transferred_roots_before_resume_error() {
        assert_terminal_root_disposition(
            SettlementSource::Cancellation,
            RootDisposition::Cancelled,
        );
        assert_terminal_root_disposition(SettlementSource::Deadline, RootDisposition::Deadline);
        assert_terminal_root_disposition(
            SettlementSource::InternalStop,
            RootDisposition::InternalStop,
        );
    }

    #[test]
    fn phase_5_resource_retained_provider_closes_once_after_resume_error() {
        let mut context =
            RequestExecutionContext::<RejectResumeUnit>::create(BytecodeSchedulerPorts::default());
        let table = context.resource_table();
        let terminations = Arc::new(Mutex::new(Vec::new()));
        let _handle = table
            .register_byte_stream(Box::new(TerminatingByteStream {
                root: ValueSlot::integer(91),
                terminations: Arc::clone(&terminations),
            }))
            .unwrap();
        let registry = PendingRegistry::<
            usize,
            SuspendedTrampoline<RejectResumeUnit, usize>,
            RejectResumeOutcome,
        >::new(context.take_pending_registration().unwrap());
        let completion = registry
            .begin_with_resource_roots(RootEscrow::empty(), table.root_pin())
            .unwrap();
        let queue = Arc::new(RejectWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<
            dyn PendingWakeQueue<
                usize,
                SuspendedTrampoline<RejectResumeUnit, usize>,
                RejectResumeOutcome,
            >,
        > = queue.clone();
        let suspended = FlatTrampoline::new(RejectResumeUnit, child_registration()).suspend();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(4, suspended),
                wake_queue,
            )
            .unwrap();
        assert!(matches!(
            completion.complete(RejectResumeOutcome(0)),
            SettleDisposition::Enqueued
        ));

        let wake = queue.0.lock().unwrap().pop().unwrap();
        assert!(
            BytecodeScheduler::<RejectResumeUnit>::resume_from_pending_wake(
                wake,
                BytecodeSchedulerPorts::default(),
            )
            .is_err()
        );
        assert_eq!(table.snapshot().live, 1);
        assert!(terminations.lock().unwrap().is_empty());

        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
        assert_eq!(
            *terminations.lock().unwrap(),
            [RequestResourceTermination::RequestNotStarted]
        );
    }

    #[test]
    fn phase_5_first_poll_missing_adapter_executor_releases_arguments_once() {
        let release_count = Arc::new(AtomicUsize::new(0));
        let scheduler = BytecodeScheduler::new(
            RejectedAdapterProbe {
                release_count: Arc::clone(&release_count),
                emitted: false,
            },
            BytecodeSchedulerPorts::default(),
            child_registration(),
        );

        assert!(matches!(
            scheduler.run(&mut NoopHeap, &mut NoopBudget),
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        ));
        assert_eq!(release_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn phase_5_stream_missing_supervisor_releases_emitted_item_once() {
        let release_count = Arc::new(AtomicUsize::new(0));
        let scheduler = BytecodeScheduler::new(
            RejectedStreamProbe {
                release_count: Arc::clone(&release_count),
                emitted: false,
            },
            BytecodeSchedulerPorts::default(),
            child_registration(),
        );

        assert!(matches!(
            scheduler.run(&mut NoopHeap, &mut NoopBudget),
            Err(BytecodeSchedulerError::UnsupportedStream)
        ));
        assert_eq!(release_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn pending_park_and_resume_round_trip() {
        let mut heap = NoopHeap;
        let mut budget = NoopBudget;
        let supervisor = Arc::new(TestStreamSupervisor {
            parked: Mutex::new(None),
            emitted: Mutex::new(Vec::new()),
        });
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(
                supervisor.clone() as Arc<dyn BytecodeStreamSupervisor<TestUnit>>
            ),
        };

        let outcome = BytecodeScheduler::new(TestUnit::parked(7), ports, child_registration())
            .run(&mut heap, &mut budget)
            .unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Parked));

        let (operation, suspended) = supervisor.parked.lock().unwrap().take().unwrap();
        assert_eq!(operation, 7);

        let registry =
            PendingRegistry::<usize, TestSuspended, TestResumeOutcome>::new(pending_registration());
        let completion = registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .unwrap();
        let queue = Arc::new(TestWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<dyn PendingWakeQueue<usize, TestSuspended, TestResumeOutcome>> =
            queue.clone();
        let publication = registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(11, suspended),
                wake_queue,
            )
            .unwrap();
        assert_eq!(publication, PendingPublication::Waiting);
        assert!(matches!(
            completion.complete(TestResumeOutcome(42)),
            SettleDisposition::Enqueued
        ));

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let scheduler = BytecodeScheduler::<TestUnit>::resume_from_pending_wake(
            wake,
            BytecodeSchedulerPorts::default(),
        )
        .unwrap();
        let outcome = scheduler.run(&mut heap, &mut budget).unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Complete(42)));
    }

    #[test]
    fn frozen_inventory_rejects_a_started_child_without_installing_it() {
        let executor = Arc::new(ResumeThenChildExecutor(AtomicUsize::new(0)));
        let supervisor = Arc::new(ResumeThenChildSupervisor(Mutex::new(None)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(
                executor.clone() as Arc<dyn BytecodeChildExecutor<ResumeThenChildUnit>>
            ),
            stream_supervisor: Some(
                supervisor.clone() as Arc<dyn BytecodeStreamSupervisor<ResumeThenChildUnit>>
            ),
        };
        let mut context = RequestExecutionContext::create(ports);
        let registry = PendingRegistry::<usize, ResumeThenChildSuspended, TestResumeOutcome>::new(
            context.take_pending_registration().unwrap(),
        );
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let queue = Arc::new(ResumeThenChildQueue(Mutex::new(Vec::new())));
        context.install_root(ResumeThenChildUnit {
            state: ResumeThenChildState::Park,
        });
        let (parked, snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        assert!(matches!(parked, Ok(BytecodeSchedulerOutcome::Parked)));
        assert_eq!(snapshot.child.current, 0);
        assert!(!snapshot.child.ever_created);

        let (operation, suspended) = supervisor.0.lock().unwrap().take().unwrap();
        let wake_queue: Arc<
            dyn PendingWakeQueue<usize, ResumeThenChildSuspended, TestResumeOutcome>,
        > = queue.clone();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(operation, suspended),
                wake_queue,
            )
            .unwrap();
        assert!(matches!(
            completion.complete(TestResumeOutcome(0)),
            SettleDisposition::Enqueued
        ));
        let wake = queue.0.lock().unwrap().pop().unwrap();
        let scheduler = BytecodeScheduler::<ResumeThenChildUnit>::resume_from_pending_wake(
            wake,
            BytecodeSchedulerPorts {
                child_executor: Some(
                    executor.clone() as Arc<dyn BytecodeChildExecutor<ResumeThenChildUnit>>
                ),
                stream_supervisor: None,
            },
        )
        .unwrap();
        let error = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap_err();
        let BytecodeSchedulerError::ChildOwnerCreation(error) = error else {
            panic!("expected frozen child inventory rejection")
        };
        assert_eq!(error.kind(), OwnerCreationErrorKind::InventoryFrozen);
        assert_eq!(error.domain(), OwnerDomain::Child);
        assert_eq!(executor.0.load(Ordering::SeqCst), 1);
        assert_eq!(snapshot.child.current, 0);
    }

    #[test]
    fn phase_5_stream_next_pending_wake_end_does_not_produce_an_item() {
        let executor = Arc::new(PendingStreamExecutor(Mutex::new(None)));
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(
                executor.clone() as Arc<dyn BytecodeChildExecutor<PendingStreamNextUnit>>
            ),
            stream_supervisor: None,
        };
        let mut context = RequestExecutionContext::create(ports);
        let registry = PendingRegistry::<usize, PendingStreamSuspended, PendingStreamOutcome>::new(
            context.take_pending_registration().unwrap(),
        );
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let queue = Arc::new(PendingStreamQueue(Mutex::new(Vec::new())));
        context.install_root(PendingStreamNextUnit {
            entered: false,
            resumed: None,
        });
        let (parked, _snapshot) = context.drive(&mut NoopHeap, &mut NoopBudget);
        assert!(matches!(parked, Ok(BytecodeSchedulerOutcome::Parked)));

        let (operation, suspended) = executor.0.lock().unwrap().take().unwrap();
        assert_eq!(operation, 13);
        let wake_queue: Arc<
            dyn PendingWakeQueue<usize, PendingStreamSuspended, PendingStreamOutcome>,
        > = queue.clone();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(operation, suspended),
                wake_queue,
            )
            .unwrap();
        assert!(matches!(
            completion.complete(PendingStreamOutcome::End),
            SettleDisposition::Enqueued
        ));
        let wake = queue.0.lock().unwrap().pop().unwrap();
        let scheduler = BytecodeScheduler::<PendingStreamNextUnit>::resume_from_pending_wake(
            wake,
            BytecodeSchedulerPorts {
                child_executor: Some(
                    executor as Arc<dyn BytecodeChildExecutor<PendingStreamNextUnit>>,
                ),
                stream_supervisor: None,
            },
        )
        .unwrap();
        let outcome = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap();
        assert!(matches!(
            outcome,
            BytecodeSchedulerOutcome::Complete(PendingStreamOutcome::End)
        ));
    }

    #[test]
    fn emit_stream_backpressure_parks_then_zero_result_wake_continues() {
        let mut heap = NoopHeap;
        let mut budget = NoopBudget;
        let supervisor = Arc::new(TestStreamSupervisor {
            parked: Mutex::new(None),
            emitted: Mutex::new(Vec::new()),
        });
        let ports = BytecodeSchedulerPorts {
            child_executor: None,
            stream_supervisor: Some(
                supervisor.clone() as Arc<dyn BytecodeStreamSupervisor<TestUnit>>
            ),
        };

        let outcome = BytecodeScheduler::new(TestUnit::emit(7, 99), ports, child_registration())
            .run(&mut heap, &mut budget)
            .unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Parked));
        assert_eq!(*supervisor.emitted.lock().unwrap(), [7]);

        let (operation, suspended) = supervisor.parked.lock().unwrap().take().unwrap();
        assert_eq!(operation, 7);

        let registry =
            PendingRegistry::<usize, TestSuspended, TestResumeOutcome>::new(pending_registration());
        let completion = registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .unwrap();
        let queue = Arc::new(TestWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<dyn PendingWakeQueue<usize, TestSuspended, TestResumeOutcome>> =
            queue.clone();
        assert_eq!(
            registry
                .publish(
                    completion.ticket(),
                    PendingOwnerDraft::new(11, suspended),
                    wake_queue
                )
                .unwrap(),
            PendingPublication::Waiting
        );
        assert!(matches!(
            completion.complete(TestResumeOutcome(0)),
            SettleDisposition::Enqueued
        ));

        let wake = queue.0.lock().unwrap().pop().unwrap();
        let scheduler = BytecodeScheduler::<TestUnit>::resume_from_pending_wake(
            wake,
            BytecodeSchedulerPorts::default(),
        )
        .unwrap();
        let outcome = scheduler.run(&mut heap, &mut budget).unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Complete(99)));
    }

    #[test]
    fn adapter_pending_parks_then_resume_completes() {
        struct AdapterExecutor {
            parked: Mutex<Option<(usize, TestSuspended)>>,
        }

        impl BytecodeChildExecutor<TestUnit> for AdapterExecutor {
            fn execute_child(
                &self,
                _invocation: usize,
                _heap: &mut dyn VmHeap,
                _budget: &mut dyn VmBudget,
            ) -> Result<BytecodeChildStart<TestUnit>, BytecodeSchedulerError> {
                Err(BytecodeSchedulerError::UnsupportedChild)
            }

            fn execute_adapter(
                &self,
                _invocation: usize,
                _heap: &mut dyn VmHeap,
                _budget: &mut dyn VmBudget,
            ) -> Result<BytecodeAdapterHandoff<TestUnit>, BytecodeSchedulerError> {
                Ok(BytecodeAdapterHandoff::Pending(7))
            }

            fn park_adapter(
                &self,
                operation: usize,
                suspended: TestSuspended,
                _heap: &mut dyn VmHeap,
                _budget: &mut dyn VmBudget,
            ) -> Result<(), BytecodeSchedulerError> {
                *self.parked.lock().unwrap() = Some((operation, suspended));
                Ok(())
            }
        }

        let mut heap = NoopHeap;
        let mut budget = NoopBudget;
        let executor = Arc::new(AdapterExecutor {
            parked: Mutex::new(None),
        });
        let ports = BytecodeSchedulerPorts {
            child_executor: Some(executor.clone() as Arc<dyn BytecodeChildExecutor<TestUnit>>),
            stream_supervisor: None,
        };
        let outcome = BytecodeScheduler::new(
            TestUnit {
                control: Some(TestControl::EnterAdapter(7)),
                resumed: None,
                finish_after_resume: Some(99),
            },
            ports,
            child_registration(),
        )
        .run(&mut heap, &mut budget)
        .unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Parked));

        let (operation, suspended) = executor.parked.lock().unwrap().take().unwrap();
        assert_eq!(operation, 7);
        let registry =
            PendingRegistry::<usize, TestSuspended, TestResumeOutcome>::new(pending_registration());
        let completion = registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .unwrap();
        let queue = Arc::new(TestWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<dyn PendingWakeQueue<usize, TestSuspended, TestResumeOutcome>> =
            queue.clone();
        registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(operation, suspended),
                wake_queue,
            )
            .unwrap();
        assert!(matches!(
            completion.complete(TestResumeOutcome(42)),
            SettleDisposition::Enqueued
        ));
        let wake = queue.0.lock().unwrap().pop().unwrap();
        let scheduler = BytecodeScheduler::<TestUnit>::resume_from_pending_wake(
            wake,
            BytecodeSchedulerPorts {
                child_executor: Some(executor.clone() as Arc<dyn BytecodeChildExecutor<TestUnit>>),
                stream_supervisor: None,
            },
        )
        .unwrap();
        let outcome = scheduler.run(&mut heap, &mut budget).unwrap();
        assert!(matches!(outcome, BytecodeSchedulerOutcome::Complete(99)));
    }
}

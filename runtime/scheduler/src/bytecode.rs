//! Flat scheduler driver over one or more bytecode execution units.

use std::{fmt, sync::Arc};

use skiff_runtime_model::vm_heap::{VmHeap, VmHeapError};
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};
use skiff_runtime_vm::{
    AdapterInvocation as VmAdapterInvocation, ChildInvocation as VmChildInvocation, ChildTarget,
    PendingOperation as VmPendingOperation, ResumeOutcome, StreamItem as VmStreamItem, VmBudget,
    VmControl, VmError, VmFiber, VmResult, VmResumeToken,
};

use crate::{
    owner_inventory::{ChildOwnerRegistration, OwnerCreationError},
    EnterChildError, FlatTrampoline, PendingWake, RootEscrow, SuspendedTrampoline,
    TrampolineCompletion,
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
            Self::Vm(error) => write!(formatter, "bytecode VM unit failed: {error}"),
            Self::Port(message) => write!(formatter, "bytecode scheduler port failed: {message}"),
        }
    }
}

impl std::error::Error for BytecodeSchedulerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Vm(error) => Some(error),
            Self::ChildOwnerCreation(error) => Some(error),
            _ => None,
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
        }
    }

    pub(crate) fn from_parts(
        trampoline: FlatTrampoline<U, U::ResumeToken>,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Self {
        Self { trampoline, ports }
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

    pub fn into_trampoline(self) -> FlatTrampoline<U, U::ResumeToken> {
        self.trampoline
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
                    let executor = self
                        .ports
                        .child_executor
                        .as_ref()
                        .ok_or(BytecodeSchedulerError::UnsupportedAdapter)?;
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
                    let supervisor = self
                        .ports
                        .stream_supervisor
                        .as_ref()
                        .ok_or(BytecodeSchedulerError::UnsupportedStream)?;
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
    ) -> Result<Self, BytecodeSchedulerError> {
        let (owner, settlement) = wake.into_parts();
        let (resume, suspended, escrow, pending_owner) = owner.into_parts();
        let resumed = Self::resume_from_suspended(
            suspended,
            resume,
            settlement.into_outcome(),
            escrow,
            ports,
        );
        drop(pending_owner);
        resumed
    }

    /// Restores a scheduler from a suspended chain and its resume envelope.
    pub fn resume_from_suspended(
        suspended: SuspendedTrampoline<U, U::ResumeToken>,
        resume: U::ResumeToken,
        outcome: U::ResumeOutcome,
        escrow: RootEscrow,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<Self, BytecodeSchedulerError> {
        escrow.restore();
        let mut trampoline = suspended.resume();
        trampoline.active_mut().resume(resume, outcome)?;
        Ok(Self { trampoline, ports })
    }
}

impl<U> VmRootSource for BytecodeScheduler<U>
where
    U: BytecodeUnit + VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.trampoline.visit_roots(visitor)
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

    fn is_stream_next_child(invocation: &VmChildInvocation) -> bool {
        invocation.target() == ChildTarget::StreamNext
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use skiff_runtime_model::{
        vm_heap::{VmHeap, VmHeapError},
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };
    use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmSemanticCharge};

    use super::*;
    use crate::{
        owner_inventory::{
            ChildOwnerRegistration, PendingOwnerRegistration, RequestExecutionOwnerInventory,
        },
        PendingOwnerDraft, PendingPublication, PendingRegistry, PendingWake, PendingWakeQueue,
        RootDisposition, RootEscrow, RootEscrowBacking, SettleDisposition,
    };

    fn pending_registration() -> PendingOwnerRegistration {
        let (registrations, _freeze) = RequestExecutionOwnerInventory::open().into_parts();
        registrations.pending()
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
    type TestWake = PendingWake<usize, TestSuspended, usize>;

    #[derive(Debug)]
    struct TestUnit {
        control: Option<TestControl>,
        resumed: Option<(usize, usize)>,
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
        type ResumeOutcome = usize;
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
                TestControl::Complete(self.finish_after_resume.take().unwrap_or(outcome))
            } else if let Some(control) = self.control.take() {
                control
            } else {
                TestControl::Complete(0)
            }
        }

        fn resume(&mut self, token: usize, outcome: usize) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: usize) -> usize {
            completed
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

    impl PendingWakeQueue<usize, TestSuspended, usize> for TestWakeQueue {
        fn enqueue(&self, wake: TestWake) {
            self.0.lock().unwrap().push(wake);
        }
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

        let registry = PendingRegistry::<usize, TestSuspended, usize>::new(pending_registration());
        let completion = registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .unwrap();
        let queue = Arc::new(TestWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<dyn PendingWakeQueue<usize, TestSuspended, usize>> = queue.clone();
        let publication = registry
            .publish(
                completion.ticket(),
                PendingOwnerDraft::new(11, suspended),
                wake_queue,
            )
            .unwrap();
        assert_eq!(publication, PendingPublication::Waiting);
        assert!(matches!(
            completion.complete(42),
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

        let registry = PendingRegistry::<usize, TestSuspended, usize>::new(pending_registration());
        let completion = registry
            .begin(RootEscrow::new(Box::new(EmptyRoots)))
            .unwrap();
        let queue = Arc::new(TestWakeQueue(Mutex::new(Vec::new())));
        let wake_queue: Arc<dyn PendingWakeQueue<usize, TestSuspended, usize>> = queue.clone();
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
            completion.complete(0),
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
        let scheduler = BytecodeScheduler::<TestUnit>::resume_from_suspended(
            suspended,
            operation,
            42,
            RootEscrow::new(Box::new(EmptyRoots)),
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

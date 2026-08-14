//! Flat scheduler driver over one or more bytecode execution units.

use std::{fmt, sync::Arc};

use skiff_runtime_model::vm_heap::{VmHeap, VmHeapError};
use skiff_runtime_model::vm_root::{VmRootSource, VmRootVisitor};
use skiff_runtime_vm::{
    AdapterInvocation as VmAdapterInvocation, ChildInvocation as VmChildInvocation, ChildTarget,
    PendingOperation as VmPendingOperation, ResumeOutcome, StreamItem as VmStreamItem,
    StreamItemReleaseError, VmBudget, VmCompletion, VmControl, VmError, VmFiber, VmResumeToken,
    VmTerminalEscrow,
};

use crate::{
    owner_inventory::{ChildOwnerRegistration, OwnerCreationError},
    pending::{MappedPendingWakeGuard, PendingResumeFailure},
    ClaimedPendingWakeGuard, EnterChildError, FlatTrampoline, PendingOwnerDraft, PendingWake,
    RequestResourceRootPin, SuspendedTrampoline,
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

impl VmRootSource for BytecodeSchedulerError {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
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

/// Sealed exact owner for an unrecoverable terminal scheduler handoff.
///
/// The only constructor consumes a VM-owned failure carrier and converts it
/// into terminal escrow. Callers cannot manufacture an ownerless terminal
/// port failure from an ordinary diagnostic.
pub struct BytecodeTerminalOwner {
    escrow: VmTerminalEscrow,
}

impl BytecodeTerminalOwner {
    fn from_stream_item_release(failure: StreamItemReleaseError) -> (BytecodeSchedulerError, Self) {
        let (escrow, error) = failure.into_terminal_escrow();
        (BytecodeSchedulerError::Vm(error), Self { escrow })
    }

    fn into_escrow(self) -> VmTerminalEscrow {
        self.escrow
    }
}

impl fmt::Debug for BytecodeTerminalOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeTerminalOwner")
            .field("root_count", &self.escrow.root_count())
            .finish_non_exhaustive()
    }
}

impl VmRootSource for BytecodeTerminalOwner {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.escrow.visit_roots(visitor)
    }
}

/// Owner-bearing terminalization failure returned by a [`BytecodeUnit`].
#[must_use = "a terminal failure must be routed with its exact escrow owner"]
pub struct BytecodeTerminalFailure {
    reason: BytecodeSchedulerError,
    owner: BytecodeTerminalOwner,
}

impl BytecodeTerminalFailure {
    /// Converts the still-unique failed stream item directly into exact
    /// terminal escrow; no raw item, tag or cleanup-kind projection escapes.
    pub fn stream_item_release(failure: StreamItemReleaseError) -> Self {
        let (reason, owner) = BytecodeTerminalOwner::from_stream_item_release(failure);
        Self { reason, owner }
    }

    pub const fn reason(&self) -> &BytecodeSchedulerError {
        &self.reason
    }

    fn into_parts(self) -> (BytecodeSchedulerError, BytecodeTerminalOwner) {
        (self.reason, self.owner)
    }
}

impl fmt::Debug for BytecodeTerminalFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeTerminalFailure")
            .field("reason", &self.reason)
            .field("owner", &self.owner)
            .finish()
    }
}

impl VmRootSource for BytecodeTerminalFailure {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.reason.visit_roots(visitor)?;
        self.owner.visit_roots(visitor)
    }
}

/// Owner returned when a scheduler port rejects a handoff.
///
/// A port either did not accept its original input or consumed that input and
/// reached the unique continuation. It must return whichever affine owner is
/// still live; neither variant may be silently discarded in order to report
/// the accompanying reason.
pub enum BytecodePortFailureOwner<I, R> {
    Input(I),
    Continuation(R),
    Terminal(BytecodeTerminalOwner),
}

/// Owner-returning failure from an adapter, stream-consumer or stream-emitter
/// port.
#[must_use = "a port failure must be routed with its affine owner"]
pub struct BytecodePortFailure<I, R> {
    reason: BytecodeSchedulerError,
    owner: BytecodePortFailureOwner<I, R>,
}

impl<I, R> BytecodePortFailure<I, R> {
    pub fn input(reason: BytecodeSchedulerError, input: I) -> Self {
        Self {
            reason,
            owner: BytecodePortFailureOwner::Input(input),
        }
    }

    pub fn continuation(reason: BytecodeSchedulerError, continuation: R) -> Self {
        Self {
            reason,
            owner: BytecodePortFailureOwner::Continuation(continuation),
        }
    }

    /// Constructs the sole terminal port failure by consuming the sealed VM
    /// carrier that still owns the rejected item and continuation.
    pub fn terminal_stream_release(failure: StreamItemReleaseError) -> Self {
        let (reason, owner) = BytecodeTerminalOwner::from_stream_item_release(failure);
        Self {
            reason,
            owner: BytecodePortFailureOwner::Terminal(owner),
        }
    }

    pub const fn reason(&self) -> &BytecodeSchedulerError {
        &self.reason
    }

    pub const fn owner(&self) -> &BytecodePortFailureOwner<I, R> {
        &self.owner
    }

    pub fn into_parts(self) -> (BytecodeSchedulerError, BytecodePortFailureOwner<I, R>) {
        (self.reason, self.owner)
    }
}

impl<I, R> fmt::Debug for BytecodePortFailure<I, R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodePortFailure")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

/// Failure to restore a unit at one exact continuation.
#[must_use = "a rejected resume must return its continuation and outcome"]
pub enum BytecodeResumeFailure<R, O> {
    Terminal(BytecodeSchedulerError),
    Rejected {
        reason: BytecodeSchedulerError,
        resume: R,
        outcome: O,
    },
}

impl<R, O> BytecodeResumeFailure<R, O> {
    pub const fn reason(&self) -> &BytecodeSchedulerError {
        match self {
            Self::Terminal(reason) | Self::Rejected { reason, .. } => reason,
        }
    }
}

impl<R, O> fmt::Debug for BytecodeResumeFailure<R, O> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeResumeFailure")
            .field("reason", self.reason())
            .finish_non_exhaustive()
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
    ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>>;

    /// Terminalizes an emitted item when no supervisor can accept it.
    ///
    /// The VM implementation performs the item's exact linked release. If
    /// that release fails, the returned owner-bearing error is converted into
    /// scheduler terminal escrow before this synchronous drive returns.
    fn release_rejected_stream_item(
        item: Self::StreamItem,
        heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeTerminalFailure> {
        let _ = (item, heap);
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

/// One complete request to publish an actual-pending operation.
///
/// The operation and suspended invocation chain remain sealed together until
/// a park port either accepts both or returns both in a
/// [`BytecodeParkFailure`].
#[must_use = "a park request must be accepted or returned in a failure"]
pub struct BytecodeParkRequest<U: BytecodeUnit> {
    operation: U::PendingOperation,
    suspended: SuspendedTrampoline<U, U::ResumeToken>,
}

impl<U: BytecodeUnit> BytecodeParkRequest<U> {
    pub fn new(
        operation: U::PendingOperation,
        suspended: SuspendedTrampoline<U, U::ResumeToken>,
    ) -> Self {
        Self {
            operation,
            suspended,
        }
    }

    pub const fn operation(&self) -> &U::PendingOperation {
        &self.operation
    }

    pub const fn suspended(&self) -> &SuspendedTrampoline<U, U::ResumeToken> {
        &self.suspended
    }

    pub fn into_parts(self) -> (U::PendingOperation, SuspendedTrampoline<U, U::ResumeToken>) {
        (self.operation, self.suspended)
    }
}

/// Affine owner returned by a failed park publication.
pub enum BytecodeParkFailureOwner<U: BytecodeUnit> {
    /// The port rejected the original sealed request without transforming it.
    Unaccepted(BytecodeParkRequest<U>),
    /// The registry consumed the operation envelope and returned its still
    /// unpublished resume/suspension draft.
    PendingDraft(PendingOwnerDraft<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>>),
}

/// Owner-returning failure from an actual-pending park port.
#[must_use = "a park failure must be routed with its suspended owner"]
pub struct BytecodeParkFailure<U: BytecodeUnit> {
    reason: BytecodeSchedulerError,
    owner: BytecodeParkFailureOwner<U>,
}

impl<U: BytecodeUnit> BytecodeParkFailure<U> {
    pub fn unaccepted(reason: BytecodeSchedulerError, request: BytecodeParkRequest<U>) -> Self {
        Self {
            reason,
            owner: BytecodeParkFailureOwner::Unaccepted(request),
        }
    }

    pub fn pending_draft(
        reason: BytecodeSchedulerError,
        draft: PendingOwnerDraft<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>>,
    ) -> Self {
        Self {
            reason,
            owner: BytecodeParkFailureOwner::PendingDraft(draft),
        }
    }

    pub const fn reason(&self) -> &BytecodeSchedulerError {
        &self.reason
    }

    pub const fn owner(&self) -> &BytecodeParkFailureOwner<U> {
        &self.owner
    }

    pub fn into_parts(self) -> (BytecodeSchedulerError, BytecodeParkFailureOwner<U>) {
        (self.reason, self.owner)
    }
}

impl<U: BytecodeUnit> fmt::Debug for BytecodeParkFailure<U> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeParkFailure")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
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
    ) -> Result<BytecodeAdapterHandoff<U>, BytecodePortFailure<U::AdapterInvocation, U::ResumeToken>>;

    /// Publishes an adapter-owned actual-`Pending` operation.
    ///
    /// Implementations that never return `BytecodeAdapterHandoff::Pending`
    /// may leave this default in place.
    fn park_adapter(
        &self,
        request: BytecodeParkRequest<U>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<U>> {
        let _ = (heap, budget);
        Err(BytecodeParkFailure::unaccepted(
            BytecodeSchedulerError::UnsupportedPark,
            request,
        ))
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
    ) -> Result<BytecodeStreamHandoff<U>, BytecodePortFailure<U::ChildInvocation, U::ResumeToken>>
    {
        let _ = (heap, budget);
        Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedChild,
            invocation,
        ))
    }

    /// Publishes the stream-consumer pending owner produced by
    /// [`Self::execute_stream_next`].
    fn park_stream_next(
        &self,
        request: BytecodeParkRequest<U>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<U>> {
        let _ = (heap, budget);
        Err(BytecodeParkFailure::unaccepted(
            BytecodeSchedulerError::UnsupportedPark,
            request,
        ))
    }
}

/// Port used for stream emission and actual-Pending parking.
pub trait BytecodeStreamSupervisor<U: BytecodeUnit>: Send + Sync + 'static {
    /// Emits one item and reports whether the producer can continue immediately
    /// or must park with a real backpressure operation.
    fn emit_stream_handoff(
        &self,
        item: U::StreamItem,
        _depth: usize,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<U>, BytecodePortFailure<U::StreamItem, U::ResumeToken>> {
        let _ = (heap, budget);
        Err(BytecodePortFailure::input(
            BytecodeSchedulerError::UnsupportedStream,
            item,
        ))
    }

    fn park(
        &self,
        request: BytecodeParkRequest<U>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<U>>;

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

#[allow(dead_code)] // Every field is intentionally held only for its Drop lifetime.
enum BytecodeSchedulerRetainedOwner<U: BytecodeUnit> {
    None,
    Complete(U::RootResult),
    ChildInput(U::ChildInvocation),
    AdapterInput(U::AdapterInvocation),
    StreamInput(U::StreamItem),
    PortContinuation(U::ResumeToken),
    ResumeRejected {
        resume: U::ResumeToken,
        outcome: U::ResumeOutcome,
    },
}

#[allow(dead_code)] // The public carrier is opaque; request retention only moves and drops it.
enum BytecodeSchedulerFailureOwnerKind<U: BytecodeUnit> {
    Scheduler {
        scheduler: BytecodeScheduler<U>,
        retained: BytecodeSchedulerRetainedOwner<U>,
    },
    Park {
        owner: BytecodeParkFailureOwner<U>,
        ports: BytecodeSchedulerPorts<U>,
        resource_roots: Option<RequestResourceRootPin>,
    },
    MappedWake {
        guard: MappedPendingWakeGuard<
            U::ResumeToken,
            FlatTrampoline<U, U::ResumeToken>,
            U::ResumeOutcome,
        >,
        ports: BytecodeSchedulerPorts<U>,
    },
}

/// Opaque affine owners retained by a [`BytecodeSchedulerFailure`].
///
/// Request drivers may move this carrier into their retention object so it is
/// dropped before the request heap. Its contents deliberately have no public
/// projection: only the scheduler can interpret and safely resume these
/// owners.
#[must_use = "a scheduler failure owner must be retained until request heap teardown"]
pub struct BytecodeSchedulerFailureOwner<U: BytecodeUnit> {
    #[allow(dead_code)] // The carrier intentionally exposes no owner projection.
    kind: BytecodeSchedulerFailureOwnerKind<U>,
    terminal_escrow: Option<VmTerminalEscrow>,
}

impl<U: BytecodeUnit> BytecodeSchedulerFailureOwner<U> {
    /// Exact terminal owners captured while normalizing a VM handoff failure.
    pub const fn terminal_escrow(&self) -> Option<&VmTerminalEscrow> {
        self.terminal_escrow.as_ref()
    }

    /// Releases the exact terminal owner suffix monotonically.
    ///
    /// A failed release leaves the same escrow in this carrier so request
    /// retention can keep it ahead of heap teardown and retry safely.
    pub fn release_terminal_escrow(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let Some(escrow) = self.terminal_escrow.as_mut() else {
            return Ok(());
        };
        escrow.release_all(heap)?;
        self.terminal_escrow = None;
        Ok(())
    }
}

impl<U: BytecodeUnit> fmt::Debug for BytecodeSchedulerFailureOwner<U> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeSchedulerFailureOwner")
            .field(
                "terminal_roots",
                &self
                    .terminal_escrow
                    .as_ref()
                    .map_or(0, VmTerminalEscrow::root_count),
            )
            .finish_non_exhaustive()
    }
}

impl VmRootSource for BytecodeSchedulerFailureOwner<VmFiber> {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match &self.kind {
            BytecodeSchedulerFailureOwnerKind::Scheduler {
                scheduler,
                retained,
            } => {
                scheduler.visit_roots(visitor)?;
                match retained {
                    BytecodeSchedulerRetainedOwner::None
                    | BytecodeSchedulerRetainedOwner::PortContinuation(_) => {}
                    BytecodeSchedulerRetainedOwner::Complete(completion) => {
                        completion.visit_roots(visitor)?;
                    }
                    BytecodeSchedulerRetainedOwner::ChildInput(invocation) => {
                        invocation.visit_roots(visitor)?;
                    }
                    BytecodeSchedulerRetainedOwner::AdapterInput(invocation) => {
                        invocation.visit_roots(visitor)?;
                    }
                    BytecodeSchedulerRetainedOwner::StreamInput(item) => {
                        item.visit_roots(visitor)?;
                    }
                    BytecodeSchedulerRetainedOwner::ResumeRejected { outcome, .. } => {
                        outcome.visit_roots(visitor)?;
                    }
                }
            }
            BytecodeSchedulerFailureOwnerKind::Park {
                owner,
                resource_roots,
                ..
            } => {
                match owner {
                    BytecodeParkFailureOwner::Unaccepted(request) => {
                        request.operation.visit_roots(visitor)?;
                        request.suspended.visit_roots(visitor)?;
                    }
                    BytecodeParkFailureOwner::PendingDraft(draft) => {
                        draft.visit_roots(visitor)?;
                    }
                }
                if let Some(resource_roots) = resource_roots {
                    resource_roots.visit_roots(visitor)?;
                }
            }
            BytecodeSchedulerFailureOwnerKind::MappedWake { guard, .. } => {
                guard.visit_roots(visitor)?;
            }
        }
        if let Some(escrow) = &self.terminal_escrow {
            escrow.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// Terminal scheduler failure plus every affine owner that was live when the
/// reason arose.
#[must_use = "a scheduler failure must be decomposed without discarding its owner"]
pub struct BytecodeSchedulerFailure<U: BytecodeUnit> {
    owner: BytecodeSchedulerFailureOwner<U>,
    reason: BytecodeSchedulerError,
}

impl<U: BytecodeUnit> BytecodeSchedulerFailure<U> {
    fn new(reason: BytecodeSchedulerError, kind: BytecodeSchedulerFailureOwnerKind<U>) -> Self {
        Self {
            owner: BytecodeSchedulerFailureOwner {
                kind,
                terminal_escrow: None,
            },
            reason,
        }
    }

    fn with_terminal_owner(
        reason: BytecodeSchedulerError,
        kind: BytecodeSchedulerFailureOwnerKind<U>,
        owner: BytecodeTerminalOwner,
    ) -> Self {
        Self {
            owner: BytecodeSchedulerFailureOwner {
                kind,
                terminal_escrow: Some(owner.into_escrow()),
            },
            reason,
        }
    }

    pub(crate) fn scheduler(
        reason: BytecodeSchedulerError,
        scheduler: BytecodeScheduler<U>,
    ) -> Self {
        Self::new(
            reason,
            BytecodeSchedulerFailureOwnerKind::Scheduler {
                scheduler,
                retained: BytecodeSchedulerRetainedOwner::None,
            },
        )
    }

    fn scheduler_with(
        reason: BytecodeSchedulerError,
        scheduler: BytecodeScheduler<U>,
        retained: BytecodeSchedulerRetainedOwner<U>,
    ) -> Self {
        Self::new(
            reason,
            BytecodeSchedulerFailureOwnerKind::Scheduler {
                scheduler,
                retained,
            },
        )
    }

    fn scheduler_with_terminal(
        reason: BytecodeSchedulerError,
        scheduler: BytecodeScheduler<U>,
        owner: BytecodeTerminalOwner,
    ) -> Self {
        Self::with_terminal_owner(
            reason,
            BytecodeSchedulerFailureOwnerKind::Scheduler {
                scheduler,
                retained: BytecodeSchedulerRetainedOwner::None,
            },
            owner,
        )
    }

    fn park(
        reason: BytecodeSchedulerError,
        owner: BytecodeParkFailureOwner<U>,
        ports: BytecodeSchedulerPorts<U>,
        resource_roots: Option<RequestResourceRootPin>,
    ) -> Self {
        Self::new(
            reason,
            BytecodeSchedulerFailureOwnerKind::Park {
                owner,
                ports,
                resource_roots,
            },
        )
    }

    fn mapped_wake(
        reason: BytecodeSchedulerError,
        guard: MappedPendingWakeGuard<
            U::ResumeToken,
            FlatTrampoline<U, U::ResumeToken>,
            U::ResumeOutcome,
        >,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Self {
        Self::new(
            reason,
            BytecodeSchedulerFailureOwnerKind::MappedWake { guard, ports },
        )
    }

    pub const fn reason(&self) -> &BytecodeSchedulerError {
        &self.reason
    }

    pub fn into_parts(self) -> (BytecodeSchedulerError, BytecodeSchedulerFailureOwner<U>) {
        (self.reason, self.owner)
    }
}

impl<U: BytecodeUnit> fmt::Debug for BytecodeSchedulerFailure<U> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BytecodeSchedulerFailure")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl<U: BytecodeUnit> fmt::Display for BytecodeSchedulerFailure<U> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.reason.fmt(formatter)
    }
}

impl<U: BytecodeUnit> std::error::Error for BytecodeSchedulerFailure<U> {}

impl VmRootSource for BytecodeSchedulerFailure<VmFiber> {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.reason.visit_roots(visitor)?;
        self.owner.visit_roots(visitor)
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
        &mut self,
        resource_roots: RequestResourceRootPin,
    ) -> Result<(), BytecodeSchedulerError> {
        if self
            .resource_roots
            .as_ref()
            .is_some_and(|current| !current.is_same_table(&resource_roots))
        {
            return Err(BytecodeSchedulerError::Port(
                "pending scheduler belongs to a different request resource table".to_string(),
            ));
        }
        if self.resource_roots.is_none() {
            self.resource_roots = Some(resource_roots);
        }
        Ok(())
    }

    fn with_retained_failure(
        self,
        reason: BytecodeSchedulerError,
        retained: BytecodeSchedulerRetainedOwner<U>,
    ) -> BytecodeSchedulerFailure<U> {
        BytecodeSchedulerFailure::scheduler_with(reason, self, retained)
    }

    fn with_resume_failure(
        self,
        failure: BytecodeResumeFailure<U::ResumeToken, U::ResumeOutcome>,
    ) -> BytecodeSchedulerFailure<U> {
        match failure {
            BytecodeResumeFailure::Terminal(reason) => {
                BytecodeSchedulerFailure::scheduler(reason, self)
            }
            BytecodeResumeFailure::Rejected {
                reason,
                resume,
                outcome,
            } => self.with_retained_failure(
                reason,
                BytecodeSchedulerRetainedOwner::ResumeRejected { resume, outcome },
            ),
        }
    }

    fn with_child_port_failure(
        self,
        failure: BytecodePortFailure<U::ChildInvocation, U::ResumeToken>,
    ) -> BytecodeSchedulerFailure<U> {
        let (reason, owner) = failure.into_parts();
        match owner {
            BytecodePortFailureOwner::Input(input) => self
                .with_retained_failure(reason, BytecodeSchedulerRetainedOwner::ChildInput(input)),
            BytecodePortFailureOwner::Continuation(resume) => self.with_retained_failure(
                reason,
                BytecodeSchedulerRetainedOwner::PortContinuation(resume),
            ),
            BytecodePortFailureOwner::Terminal(owner) => {
                BytecodeSchedulerFailure::scheduler_with_terminal(reason, self, owner)
            }
        }
    }

    fn with_adapter_port_failure(
        self,
        failure: BytecodePortFailure<U::AdapterInvocation, U::ResumeToken>,
    ) -> BytecodeSchedulerFailure<U> {
        let (reason, owner) = failure.into_parts();
        match owner {
            BytecodePortFailureOwner::Input(input) => self
                .with_retained_failure(reason, BytecodeSchedulerRetainedOwner::AdapterInput(input)),
            BytecodePortFailureOwner::Continuation(resume) => self.with_retained_failure(
                reason,
                BytecodeSchedulerRetainedOwner::PortContinuation(resume),
            ),
            BytecodePortFailureOwner::Terminal(owner) => {
                BytecodeSchedulerFailure::scheduler_with_terminal(reason, self, owner)
            }
        }
    }

    fn with_stream_port_failure(
        self,
        failure: BytecodePortFailure<U::StreamItem, U::ResumeToken>,
    ) -> BytecodeSchedulerFailure<U> {
        let (reason, owner) = failure.into_parts();
        match owner {
            BytecodePortFailureOwner::Input(input) => self
                .with_retained_failure(reason, BytecodeSchedulerRetainedOwner::StreamInput(input)),
            BytecodePortFailureOwner::Continuation(resume) => self.with_retained_failure(
                reason,
                BytecodeSchedulerRetainedOwner::PortContinuation(resume),
            ),
            BytecodePortFailureOwner::Terminal(owner) => {
                BytecodeSchedulerFailure::scheduler_with_terminal(reason, self, owner)
            }
        }
    }

    fn into_park_parts(
        self,
        operation: U::PendingOperation,
    ) -> (
        BytecodeParkRequest<U>,
        BytecodeSchedulerPorts<U>,
        Option<RequestResourceRootPin>,
    ) {
        let Self {
            trampoline,
            ports,
            resource_roots,
        } = self;
        (
            BytecodeParkRequest::new(operation, trampoline.suspend()),
            ports,
            resource_roots,
        )
    }

    fn failed_park(
        failure: BytecodeParkFailure<U>,
        ports: BytecodeSchedulerPorts<U>,
        resource_roots: Option<RequestResourceRootPin>,
    ) -> BytecodeSchedulerFailure<U> {
        let (reason, owner) = failure.into_parts();
        BytecodeSchedulerFailure::park(reason, owner, ports, resource_roots)
    }

    /// Drives the active unit until a root completion or a real park.
    ///
    /// Child completion restores exactly one parent. The scheduler never calls
    /// `run_segment` or `resume` recursively.
    pub fn run(
        mut self,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<BytecodeSchedulerOutcome<U>, BytecodeSchedulerFailure<U>> {
        loop {
            let control = self.trampoline.active_mut().run_segment(heap, budget);
            match control {
                BytecodeControl::Continue => {}
                BytecodeControl::Complete(result) => {
                    let depth = self.trampoline.blocked_depth();
                    if depth != 0 {
                        return Err(self.with_retained_failure(
                            BytecodeSchedulerError::UnsupportedChild,
                            BytecodeSchedulerRetainedOwner::Complete(result),
                        ));
                    }
                    if let Some(supervisor) = self.ports.stream_supervisor.clone() {
                        if let Err(reason) = supervisor.finish_stream(depth, &result) {
                            return Err(self.with_retained_failure(
                                reason,
                                BytecodeSchedulerRetainedOwner::Complete(result),
                            ));
                        }
                    }
                    return Ok(BytecodeSchedulerOutcome::Complete(result));
                }
                BytecodeControl::EnterChild(invocation) => {
                    if !U::is_stream_next_child(&invocation) {
                        return Err(self.with_retained_failure(
                            BytecodeSchedulerError::UnsupportedChild,
                            BytecodeSchedulerRetainedOwner::ChildInput(invocation),
                        ));
                    }
                    let Some(executor) = self.ports.child_executor.clone() else {
                        return Err(self.with_retained_failure(
                            BytecodeSchedulerError::UnsupportedChild,
                            BytecodeSchedulerRetainedOwner::ChildInput(invocation),
                        ));
                    };
                    let handoff = match executor.execute_stream_next(invocation, heap, budget) {
                        Ok(handoff) => handoff,
                        Err(failure) => return Err(self.with_child_port_failure(failure)),
                    };
                    match handoff {
                        BytecodeStreamHandoff::Ready(handoff) => {
                            if let Err(failure) = self
                                .trampoline
                                .active_mut()
                                .resume(handoff.resume, handoff.outcome)
                            {
                                return Err(self.with_resume_failure(failure));
                            }
                        }
                        BytecodeStreamHandoff::Pending(operation) => {
                            let (request, ports, resource_roots) = self.into_park_parts(operation);
                            if let Err(failure) = executor.park_stream_next(request, heap, budget) {
                                return Err(Self::failed_park(failure, ports, resource_roots));
                            }
                            return Ok(BytecodeSchedulerOutcome::Parked);
                        }
                    }
                }
                BytecodeControl::EnterAdapter(invocation) => {
                    let Some(executor) = self.ports.child_executor.clone() else {
                        return Err(self.with_retained_failure(
                            BytecodeSchedulerError::UnsupportedAdapter,
                            BytecodeSchedulerRetainedOwner::AdapterInput(invocation),
                        ));
                    };
                    let handoff = match executor.execute_adapter(invocation, heap, budget) {
                        Ok(handoff) => handoff,
                        Err(failure) => return Err(self.with_adapter_port_failure(failure)),
                    };
                    match handoff {
                        BytecodeAdapterHandoff::Ready(handoff) => {
                            if let Err(failure) = self
                                .trampoline
                                .active_mut()
                                .resume(handoff.resume, handoff.outcome)
                            {
                                return Err(self.with_resume_failure(failure));
                            }
                        }
                        BytecodeAdapterHandoff::Pending(operation) => {
                            let (request, ports, resource_roots) = self.into_park_parts(operation);
                            if let Err(failure) = executor.park_adapter(request, heap, budget) {
                                return Err(Self::failed_park(failure, ports, resource_roots));
                            }
                            return Ok(BytecodeSchedulerOutcome::Parked);
                        }
                    }
                }
                BytecodeControl::EmitStream(item) => {
                    let Some(supervisor) = self.ports.stream_supervisor.clone() else {
                        return match U::release_rejected_stream_item(item, heap) {
                            Ok(()) => Err(BytecodeSchedulerFailure::scheduler(
                                BytecodeSchedulerError::UnsupportedStream,
                                self,
                            )),
                            Err(failure) => {
                                let (reason, owner) = failure.into_parts();
                                Err(BytecodeSchedulerFailure::scheduler_with_terminal(
                                    reason, self, owner,
                                ))
                            }
                        };
                    };
                    let depth = self.trampoline.blocked_depth();
                    let handoff = match supervisor.emit_stream_handoff(item, depth, heap, budget) {
                        Ok(handoff) => handoff,
                        Err(failure) => return Err(self.with_stream_port_failure(failure)),
                    };
                    match handoff {
                        BytecodeStreamHandoff::Ready(handoff) => {
                            if let Err(failure) = self
                                .trampoline
                                .active_mut()
                                .resume(handoff.resume, handoff.outcome)
                            {
                                return Err(self.with_resume_failure(failure));
                            }
                        }
                        BytecodeStreamHandoff::Pending(operation) => {
                            let (request, ports, resource_roots) = self.into_park_parts(operation);
                            if let Err(failure) = supervisor.park(request, heap, budget) {
                                return Err(Self::failed_park(failure, ports, resource_roots));
                            }
                            return Ok(BytecodeSchedulerOutcome::Parked);
                        }
                    }
                }
                BytecodeControl::Park(operation) => {
                    let supervisor = self.ports.stream_supervisor.clone();
                    let (request, ports, resource_roots) = self.into_park_parts(operation);
                    let Some(supervisor) = supervisor else {
                        return Err(BytecodeSchedulerFailure::park(
                            BytecodeSchedulerError::UnsupportedPark,
                            BytecodeParkFailureOwner::Unaccepted(request),
                            ports,
                            resource_roots,
                        ));
                    };
                    if let Err(failure) = supervisor.park(request, heap, budget) {
                        return Err(Self::failed_park(failure, ports, resource_roots));
                    }
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
    ) -> Result<Self, BytecodeSchedulerFailure<U>>
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
    ) -> Result<Self, BytecodeSchedulerFailure<U>>
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
    ) -> Result<Self, BytecodeSchedulerFailure<U>>
    where
        O: VmRootSource,
        U::ResumeOutcome: VmRootSource,
    {
        let mapped = wake
            .map(|resume, outcome, _roots| map(resume, outcome))
            .map_suspended(SuspendedTrampoline::resume);
        let failure_ports = ports.clone();
        let resumed = mapped.resume_and_commit(
            |trampoline, resume, outcome| {
                trampoline
                    .active_mut()
                    .resume(resume, outcome)
                    .map_err(|failure| match failure {
                        BytecodeResumeFailure::Terminal(error) => {
                            PendingResumeFailure::Terminal(error)
                        }
                        BytecodeResumeFailure::Rejected {
                            reason,
                            resume,
                            outcome,
                        } => PendingResumeFailure::Rejected {
                            error: reason,
                            resume,
                            outcome,
                        },
                    })
            },
            move |trampoline, resource_roots| Self {
                trampoline,
                ports,
                resource_roots,
            },
        );
        match resumed {
            Ok(scheduler) => Ok(scheduler),
            Err(failure) => {
                let (reason, guard) = failure.into_parts();
                Err(BytecodeSchedulerFailure::mapped_wake(
                    reason,
                    guard,
                    failure_ports,
                ))
            }
        }
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
    type RootResult = VmCompletion;
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
    ) -> Result<(), BytecodeResumeFailure<VmResumeToken, ResumeOutcome>> {
        VmFiber::resume(self, token, outcome).map_err(|failure| {
            let (error, returned) = failure.into_parts();
            match returned {
                None => BytecodeResumeFailure::Terminal(BytecodeSchedulerError::Vm(error)),
                Some((resume, outcome)) => BytecodeResumeFailure::Rejected {
                    reason: BytecodeSchedulerError::Vm(error),
                    resume,
                    outcome,
                },
            }
        })
    }

    fn release_rejected_stream_item(
        item: VmStreamItem,
        heap: &mut dyn VmHeap,
    ) -> Result<(), BytecodeTerminalFailure> {
        item.release(heap)
            .map(|_resume| ())
            .map_err(BytecodeTerminalFailure::stream_item_release)
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
            ChildOwnerRegistration, PendingOwnerRegistration, RequestExecutionOwnerInventory,
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

    #[derive(Debug)]
    struct LinearCompletionProbe(Arc<AtomicUsize>);

    impl Drop for LinearCompletionProbe {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct LinearCompletionUnit {
        completion: Option<LinearCompletionProbe>,
    }

    impl VmRootSource for LinearCompletionUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for LinearCompletionUnit {
        type ResumeToken = ();
        type ResumeOutcome = ();
        type RootResult = LinearCompletionProbe;
        type ChildInvocation = ();
        type AdapterInvocation = ();
        type StreamItem = ();
        type PendingOperation = ();

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> BytecodeUnitControl<Self> {
            BytecodeControl::Complete(
                self.completion
                    .take()
                    .expect("linear completion is produced exactly once"),
            )
        }

        fn resume(
            &mut self,
            _token: Self::ResumeToken,
            _outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
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
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
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
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
            Ok(())
        }

        fn release_rejected_stream_item(
            item: Self::StreamItem,
            _heap: &mut dyn VmHeap,
        ) -> Result<(), BytecodeTerminalFailure> {
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
        ) -> Result<(), BytecodeResumeFailure<usize, TestResumeOutcome>> {
            self.resumed = Some((token, outcome));
            Ok(())
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
        ) -> Result<BytecodeStreamHandoff<TestUnit>, BytecodePortFailure<usize, usize>> {
            self.emitted.lock().unwrap().push(item);
            Ok(BytecodeStreamHandoff::Pending(item))
        }

        fn park(
            &self,
            request: BytecodeParkRequest<TestUnit>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeParkFailure<TestUnit>> {
            let (operation, suspended) = request.into_parts();
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
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
            self.state = ResumeThenChildState::EnterChild;
            Ok(())
        }
    }

    type ResumeThenChildSuspended = SuspendedTrampoline<ResumeThenChildUnit, usize>;
    type ResumeThenChildWake = PendingWake<usize, ResumeThenChildSuspended, TestResumeOutcome>;

    struct ResumeThenChildSupervisor(Mutex<Option<(usize, ResumeThenChildSuspended)>>);

    impl BytecodeStreamSupervisor<ResumeThenChildUnit> for ResumeThenChildSupervisor {
        fn park(
            &self,
            request: BytecodeParkRequest<ResumeThenChildUnit>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeParkFailure<ResumeThenChildUnit>> {
            let (operation, suspended) = request.into_parts();
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
            invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<ResumeThenChildUnit>, BytecodePortFailure<usize, usize>>
        {
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedAdapter,
                invocation,
            ))
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
        ) -> Result<(), BytecodeResumeFailure<usize, PendingStreamOutcome>> {
            self.resumed = Some(outcome);
            Ok(())
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
            invocation: (),
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeAdapterHandoff<PendingStreamNextUnit>, BytecodePortFailure<(), usize>>
        {
            Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedAdapter,
                invocation,
            ))
        }

        fn execute_stream_next(
            &self,
            _invocation: (),
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<PendingStreamNextUnit>, BytecodePortFailure<(), usize>>
        {
            Ok(BytecodeStreamHandoff::Pending(13))
        }

        fn park_stream_next(
            &self,
            request: BytecodeParkRequest<PendingStreamNextUnit>,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeParkFailure<PendingStreamNextUnit>> {
            let (operation, suspended) = request.into_parts();
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
            token: Self::ResumeToken,
            outcome: Self::ResumeOutcome,
        ) -> Result<(), BytecodeResumeFailure<Self::ResumeToken, Self::ResumeOutcome>> {
            Err(BytecodeResumeFailure::Rejected {
                reason: BytecodeSchedulerError::Port("intentional resume rejection".to_string()),
                resume: token,
                outcome,
            })
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
    fn phase_5_missing_adapter_executor_retains_input_until_failure_owner_drops() {
        let release_count = Arc::new(AtomicUsize::new(0));
        let scheduler = BytecodeScheduler::new(
            RejectedAdapterProbe {
                release_count: Arc::clone(&release_count),
                emitted: false,
            },
            BytecodeSchedulerPorts::default(),
            child_registration(),
        );

        let failure = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedAdapter
        ));
        assert_eq!(Arc::strong_count(&release_count), 3);
        let (_, owner) = failure.into_parts();
        drop(owner);
        assert_eq!(Arc::strong_count(&release_count), 1);
    }

    #[test]
    fn phase_5_missing_stream_supervisor_releases_emitted_item_once() {
        let release_count = Arc::new(AtomicUsize::new(0));
        let scheduler = BytecodeScheduler::new(
            RejectedStreamProbe {
                release_count: Arc::clone(&release_count),
                emitted: false,
            },
            BytecodeSchedulerPorts::default(),
            child_registration(),
        );

        let failure = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedStream
        ));
        assert_eq!(release_count.load(Ordering::SeqCst), 1);
        assert_eq!(Arc::strong_count(&release_count), 2);
        let (_, owner) = failure.into_parts();
        drop(owner);
        assert_eq!(Arc::strong_count(&release_count), 1);
        assert_eq!(release_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn vm_fiber_root_result_is_the_sealed_vm_completion_carrier() {
        fn preserve(completion: <VmFiber as BytecodeUnit>::RootResult) -> VmCompletion {
            completion
        }

        let _type_proof: fn(VmCompletion) -> VmCompletion = preserve;
    }

    #[test]
    fn depth_one_completion_fails_closed_and_retains_owner_and_child_lease() {
        let drops = Arc::new(AtomicUsize::new(0));
        let inventory = RequestExecutionOwnerInventory::open();
        let (registrations, freeze) = inventory.into_parts();
        let mut trampoline = FlatTrampoline::new(
            LinearCompletionUnit { completion: None },
            registrations.child(),
        );
        trampoline
            .enter_child(
                LinearCompletionUnit {
                    completion: Some(LinearCompletionProbe(Arc::clone(&drops))),
                },
                (),
            )
            .unwrap();
        let scheduler = BytecodeScheduler {
            trampoline,
            ports: BytecodeSchedulerPorts::default(),
            resource_roots: None,
        };

        let failure = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedChild
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let (_, owner) = failure.into_parts();
        let BytecodeSchedulerFailureOwnerKind::Scheduler {
            scheduler,
            retained: BytecodeSchedulerRetainedOwner::Complete(_),
        } = &owner.kind
        else {
            panic!("depth-one completion must remain in the typed failure owner")
        };
        assert_eq!(scheduler.blocked_depth(), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(owner);
        assert_eq!(drops.load(Ordering::SeqCst), 1);

        let snapshot = freeze.freeze();
        assert_eq!(snapshot.child.current, 0);
        assert!(snapshot.child.ever_created);
    }

    #[test]
    fn bind_mismatch_returns_the_unchanged_scheduler_owner() {
        let first_context =
            RequestExecutionContext::<TestUnit>::create(BytecodeSchedulerPorts::default());
        let first_pin = first_context.resource_table().root_pin();
        let scheduler = BytecodeScheduler::new_with_resource_roots(
            TestUnit {
                control: Some(TestControl::Complete(73)),
                resumed: None,
                finish_after_resume: None,
            },
            BytecodeSchedulerPorts::default(),
            child_registration(),
            first_pin,
        );
        let mut second_context =
            RequestExecutionContext::<TestUnit>::create(BytecodeSchedulerPorts::default());

        let failure = second_context
            .resume_drive(scheduler, &mut NoopHeap, &mut NoopBudget)
            .unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::Port(message)
                if message == "pending scheduler belongs to a different request resource table"
        ));
        let (_, owner) = failure.into_parts();
        let BytecodeSchedulerFailureOwnerKind::Scheduler { scheduler, .. } = owner.kind else {
            panic!("bind mismatch must return the unchanged scheduler")
        };
        assert!(matches!(
            scheduler.active().control.as_ref(),
            Some(TestControl::Complete(73))
        ));
        drop(first_context);
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
    fn non_stream_next_child_fails_closed_and_returns_invocation() {
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
        let failure = scheduler.run(&mut NoopHeap, &mut NoopBudget).unwrap_err();
        assert!(matches!(
            failure.reason(),
            BytecodeSchedulerError::UnsupportedChild
        ));
        let (_, owner) = failure.into_parts();
        let BytecodeSchedulerFailureOwnerKind::Scheduler {
            retained: BytecodeSchedulerRetainedOwner::ChildInput(invocation),
            ..
        } = owner.kind
        else {
            panic!("expected the rejected child invocation owner")
        };
        assert_eq!(invocation, 0);
        assert_eq!(executor.0.load(Ordering::SeqCst), 0);
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
            ) -> Result<BytecodeAdapterHandoff<TestUnit>, BytecodePortFailure<usize, usize>>
            {
                Ok(BytecodeAdapterHandoff::Pending(7))
            }

            fn park_adapter(
                &self,
                request: BytecodeParkRequest<TestUnit>,
                _heap: &mut dyn VmHeap,
                _budget: &mut dyn VmBudget,
            ) -> Result<(), BytecodeParkFailure<TestUnit>> {
                let (operation, suspended) = request.into_parts();
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

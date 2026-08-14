use std::{fmt, num::NonZeroU64, sync::Arc};

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    ActorMethodIndex, FunctionIndex, HostEffectAdapterIndex, InstructionIndex, InterfaceTableIndex,
    LinkedValueTransferPlan, ResumeSiteIndex, ServiceOperationIndex, ShapeIndex,
    SyntheticCallbackIndex, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueSlot, VmHandle},
};

use crate::{lifecycle::LifecycleExecutor, VmBudgetClosed, VmError};

pub type VmResult = Result<VmOwnedValues, VmError>;

pub(crate) use crate::terminal_ownership::{VmLifecycleSite, VmTerminalOwner};
pub use crate::terminal_ownership::{
    VmOwnedException, VmOwnedValues, VmOwnedValuesRejected, VmResumeFailure, VmTerminalCause,
    VmTerminalEscrow,
};

/// Arguments transferred out of the operand stack for one verified host
/// effect, paired with their exact linked lifecycle plans.
///
/// Construction is sealed inside the VM. The request executor may inspect the
/// values while producing a heap-free host DTO, then must consume this owner
/// through [`Self::release`] on the request heap thread before returning
/// `Ready` or publishing a pending cell.
#[must_use = "host-effect arguments must be released on the request heap thread"]
pub struct VmHostEffectArguments {
    values: VmOwnedValues,
    plans: Box<[LinkedValueTransferPlan]>,
    function: FunctionIndex,
    instruction: InstructionIndex,
}

impl VmHostEffectArguments {
    pub(crate) fn new(
        values: VmOwnedValues,
        plans: Box<[LinkedValueTransferPlan]>,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Self {
        debug_assert_eq!(values.len(), plans.len());
        Self {
            values,
            plans,
            function,
            instruction,
        }
    }

    pub fn values(&self) -> &[ValueSlot] {
        self.values.values()
    }

    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        self.values.image()
    }

    /// Releases every transferred owner through the VM's sole lifecycle
    /// executor. Unsupported plans are rejected before the first heap
    /// mutation. A heap failure terminates the request synchronously; the
    /// failing and later owners remain in the unique boundary-owned heap for
    /// terminal teardown and are never submitted a second time here.
    pub fn release(self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let mut executor = LifecycleExecutor::new(heap);
        executor
            .release_batch(self.values.values(), &self.plans)
            .map_err(|error| {
                error.into_vm_error(self.function, self.instruction, Opcode::InvokeHost)
            })
    }
}

impl VmRootSource for VmHostEffectArguments {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.values.visit_roots(visitor)
    }
}

/// Single-owner continuation token minted by a [`VmFiber`](crate::VmFiber).
///
/// This type intentionally implements neither `Clone` nor `Copy`. A child,
/// adapter, stream or pending owner must move the same token back to the
/// originating fiber, where its complete verified descriptor is checked.
#[derive(Debug)]
#[must_use = "a resume token is unique continuation authority"]
pub struct VmResumeToken {
    image: Arc<DeploymentExecutionImage>,
    sequence: u64,
    function: FunctionIndex,
    instruction: InstructionIndex,
    resume_instruction: InstructionIndex,
    end_resume_pc: Option<InstructionIndex>,
    resume_site: ResumeSiteIndex,
    expected_stack_height: u32,
    expected_result_count: u32,
    authority: VmResumeAuthority,
}

impl VmResumeToken {
    // Reserved for the first implemented external opcode. Keeping minting
    // crate-private prevents a scheduler/adapter from forging continuations.
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        image: Arc<DeploymentExecutionImage>,
        sequence: u64,
        function: FunctionIndex,
        instruction: InstructionIndex,
        resume_instruction: InstructionIndex,
        end_resume_pc: Option<InstructionIndex>,
        resume_site: ResumeSiteIndex,
        expected_stack_height: u32,
        expected_result_count: u32,
        authority: VmResumeAuthority,
    ) -> Self {
        Self {
            image,
            sequence,
            function,
            instruction,
            resume_instruction,
            end_resume_pc,
            resume_site,
            expected_stack_height,
            expected_result_count,
            authority,
        }
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub const fn resume_instruction(&self) -> InstructionIndex {
        self.resume_instruction
    }

    pub const fn end_resume_pc(&self) -> Option<InstructionIndex> {
        self.end_resume_pc
    }

    pub const fn resume_site(&self) -> ResumeSiteIndex {
        self.resume_site
    }

    pub const fn expected_stack_height(&self) -> u32 {
        self.expected_stack_height
    }

    pub const fn expected_result_count(&self) -> u32 {
        self.expected_result_count
    }

    pub const fn kind(&self) -> VmResumeKind {
        match self.authority {
            VmResumeAuthority::Child(_) => VmResumeKind::Child,
            VmResumeAuthority::Adapter(_) => VmResumeKind::Adapter,
            VmResumeAuthority::StreamChild(_) => VmResumeKind::StreamChild,
            VmResumeAuthority::StreamItem => VmResumeKind::StreamItem,
        }
    }

    pub(crate) const fn authority(&self) -> VmResumeAuthority {
        self.authority
    }

    /// Consumes this unforgeable continuation capability after the scheduler
    /// has established the completion cell and root escrow identified by
    /// `ticket`. The target/control authority remains sealed inside the token.
    pub fn into_pending(self, ticket: PendingTicket) -> PendingOperation {
        PendingOperation {
            ticket,
            resume: self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmResumeKind {
    Child,
    Adapter,
    StreamChild,
    StreamItem,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildTarget {
    Service(ServiceOperationIndex),
    Actor(ActorMethodIndex),
    Interface {
        table: InterfaceTableIndex,
        method_ordinal: u32,
    },
    Callback(SyntheticCallbackIndex),
    StreamNext,
}

/// Non-owning route to the exact affine endpoint borrowed by `StreamNext`.
///
/// This route deliberately owns no VM value and contributes no GC root. The
/// endpoint itself stays live in the originating frame slot while the child
/// invocation is outstanding; the scheduler may only use this opaque route to
/// select the matching request resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StreamEndpointRef(VmHandle);

impl StreamEndpointRef {
    pub(crate) const fn new(route: VmHandle) -> Self {
        Self(route)
    }

    pub const fn route(self) -> VmHandle {
        self.0
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VmResumeAuthority {
    Child(ChildTarget),
    Adapter(HostEffectAdapterIndex),
    StreamChild(ChildTarget),
    StreamItem,
}

/// Owned request for the flat scheduler to enter another execution owner.
#[must_use = "a child invocation owns arguments and unique continuation authority"]
pub struct ChildInvocation {
    target: ChildTarget,
    arguments: VmOwnedValues,
    stream_endpoint: Option<StreamEndpointRef>,
    resume: VmResumeToken,
}

impl ChildInvocation {
    #[allow(dead_code)]
    pub(crate) fn new(
        target: ChildTarget,
        arguments: VmOwnedValues,
        resume: VmResumeToken,
    ) -> Result<Self, VmError> {
        if target == ChildTarget::StreamNext
            || resume.authority != VmResumeAuthority::Child(target)
            || !Arc::ptr_eq(arguments.image(), resume.image())
        {
            return Err(VmError::ResumeTokenMismatch);
        }
        Ok(Self {
            target,
            arguments,
            stream_endpoint: None,
            resume,
        })
    }

    pub(crate) fn new_stream_next(
        endpoint: StreamEndpointRef,
        arguments: VmOwnedValues,
        resume: VmResumeToken,
    ) -> Result<Self, VmError> {
        if !arguments.is_empty()
            || resume.authority != VmResumeAuthority::Child(ChildTarget::StreamNext)
            || !Arc::ptr_eq(arguments.image(), resume.image())
        {
            return Err(VmError::ResumeTokenMismatch);
        }
        Ok(Self {
            target: ChildTarget::StreamNext,
            arguments,
            stream_endpoint: Some(endpoint),
            resume,
        })
    }

    pub const fn target(&self) -> ChildTarget {
        self.target
    }

    pub const fn arguments(&self) -> &VmOwnedValues {
        &self.arguments
    }

    pub const fn stream_endpoint(&self) -> Option<StreamEndpointRef> {
        self.stream_endpoint
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: all four parts remain one logical handoff and must
    /// not be exchanged with parts from another invocation.
    pub fn into_parts(
        self,
    ) -> (
        ChildTarget,
        VmOwnedValues,
        Option<StreamEndpointRef>,
        VmResumeToken,
    ) {
        (
            self.target,
            self.arguments,
            self.stream_endpoint,
            self.resume,
        )
    }
}

impl VmRootSource for ChildInvocation {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.arguments.visit_roots(visitor)
    }
}

/// Owned invocation of one verified host-effect adapter.
#[must_use = "an adapter invocation owns arguments and unique continuation authority"]
pub struct AdapterInvocation {
    adapter: HostEffectAdapterIndex,
    arguments: VmHostEffectArguments,
    resume: VmResumeToken,
}

impl AdapterInvocation {
    #[allow(dead_code)]
    pub(crate) fn new(
        adapter: HostEffectAdapterIndex,
        arguments: VmHostEffectArguments,
        resume: VmResumeToken,
    ) -> Self {
        debug_assert_eq!(resume.authority, VmResumeAuthority::Adapter(adapter));
        debug_assert!(Arc::ptr_eq(arguments.image(), resume.image()));
        Self {
            adapter,
            arguments,
            resume,
        }
    }

    pub const fn adapter(&self) -> HostEffectAdapterIndex {
        self.adapter
    }

    pub const fn arguments(&self) -> &VmHostEffectArguments {
        &self.arguments
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: all three parts remain one logical handoff and must
    /// not be exchanged with parts from another invocation.
    pub fn into_parts(self) -> (HostEffectAdapterIndex, VmHostEffectArguments, VmResumeToken) {
        (self.adapter, self.arguments, self.resume)
    }
}

impl VmRootSource for AdapterInvocation {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.arguments.visit_roots(visitor)
    }
}

/// Owned request to establish a supervised producer child.
#[must_use = "a stream invocation owns arguments and unique continuation authority"]
pub struct StreamInvocation {
    target: ChildTarget,
    arguments: VmOwnedValues,
    resume: VmResumeToken,
}

impl StreamInvocation {
    #[allow(dead_code)]
    pub(crate) fn new(
        target: ChildTarget,
        arguments: VmOwnedValues,
        resume: VmResumeToken,
    ) -> Result<Self, VmError> {
        if resume.authority != VmResumeAuthority::StreamChild(target)
            || !Arc::ptr_eq(arguments.image(), resume.image())
        {
            return Err(VmError::ResumeTokenMismatch);
        }
        Ok(Self {
            target,
            arguments,
            resume,
        })
    }

    pub const fn target(&self) -> ChildTarget {
        self.target
    }

    pub const fn arguments(&self) -> &VmOwnedValues {
        &self.arguments
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: all three parts remain one logical handoff and must
    /// not be exchanged with parts from another invocation.
    pub fn into_parts(self) -> (ChildTarget, VmOwnedValues, VmResumeToken) {
        (self.target, self.arguments, self.resume)
    }
}

impl VmRootSource for StreamInvocation {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.arguments.visit_roots(visitor)
    }
}

/// One owned stream item handed to the stream supervisor.
#[must_use = "a stream item owns its value and unique continuation authority"]
pub struct StreamItem {
    item: VmOwnedValues,
    item_type: TypeIndex,
    item_shape: ShapeIndex,
    plan: LinkedValueTransferPlan,
    function: FunctionIndex,
    instruction: InstructionIndex,
    resume: VmResumeToken,
}

/// A failed stream-item release together with the still-unique carrier.
///
/// Heap release is transactional, so `item` remains the logical owner and may
/// be visited or retried. A request owner that has already chosen a terminal
/// outcome may instead consume this error through [`Self::into_cleanup_roots`]
/// and transfer the values into its explicit terminal cleanup escrow.
#[must_use = "a failed stream-item release still owns its item and continuation"]
pub struct StreamItemReleaseError {
    item: StreamItem,
    error: VmError,
}

impl StreamItemReleaseError {
    pub const fn item(&self) -> &StreamItem {
        &self.item
    }

    pub const fn error(&self) -> &VmError {
        &self.error
    }

    /// Returns the intact carrier for a retry on the same request heap.
    pub fn into_parts(self) -> (StreamItem, VmError) {
        (self.item, self.error)
    }

    /// Transfers the failed value owner into terminal cleanup escrow.
    ///
    /// The continuation is deliberately abandoned only on this explicit
    /// terminal path; callers that may resume must use [`Self::into_parts`].
    pub fn into_cleanup_roots(self) -> (Box<[ValueSlot]>, VmError) {
        let Self { item, error } = self;
        let StreamItem { item, .. } = item;
        (item.values, error)
    }
}

impl fmt::Debug for StreamItemReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamItemReleaseError")
            .field("item_type", &self.item.item_type)
            .field("item_shape", &self.item.item_shape)
            .field("function", &self.item.function)
            .field("instruction", &self.item.instruction)
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for StreamItemReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for StreamItemReleaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl VmRootSource for StreamItemReleaseError {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.item.visit_roots(visitor)
    }
}

impl StreamItem {
    #[allow(dead_code)]
    pub(crate) fn new(
        item: VmOwnedValues,
        item_type: TypeIndex,
        item_shape: ShapeIndex,
        plan: LinkedValueTransferPlan,
        function: FunctionIndex,
        instruction: InstructionIndex,
        resume: VmResumeToken,
    ) -> Self {
        debug_assert_eq!(resume.authority, VmResumeAuthority::StreamItem);
        debug_assert!(Arc::ptr_eq(item.image(), resume.image()));
        debug_assert_eq!(item.len(), 1);
        Self {
            item,
            item_type,
            item_shape,
            plan,
            function,
            instruction,
            resume,
        }
    }

    pub const fn item(&self) -> &VmOwnedValues {
        &self.item
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Exact verified type of the sole item transferred from the producer's
    /// linked operand stack.
    pub const fn item_type(&self) -> TypeIndex {
        self.item_type
    }

    /// Exact linker-resolved dense shape emitted for this particular
    /// `EmitStream` site. This is site authority, not a function-wide nominal
    /// type lookup.
    pub const fn item_shape(&self) -> ShapeIndex {
        self.item_shape
    }

    /// Releases the emitted owner through its exact linked lifecycle plan on
    /// the request heap thread, then returns the unique zero-result resume.
    /// No VM value crosses an actual-Pending writer flush.
    pub fn release(self, heap: &mut dyn VmHeap) -> Result<VmResumeToken, StreamItemReleaseError> {
        let released = LifecycleExecutor::new(heap)
            .release_batch(self.item.values(), std::slice::from_ref(&self.plan))
            .map_err(|error| {
                error.into_vm_error(self.function, self.instruction, Opcode::EmitStream)
            });
        match released {
            Ok(()) => Ok(self.resume),
            Err(error) => Err(StreamItemReleaseError { item: self, error }),
        }
    }
}

impl VmRootSource for StreamItem {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.item.visit_roots(visitor)
    }
}

/// Opaque key for a scheduler-owned completion cell and pre-established root
/// escrow. The VM does not interpret this value or create the cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[must_use = "a pending ticket identifies scheduler-owned completion state"]
pub struct PendingTicket(NonZeroU64);

impl PendingTicket {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> NonZeroU64 {
        self.0
    }
}

/// Actual-Pending handoff. Root escrow must already exist under `ticket`
/// before this value can be returned by an adapter.
#[derive(Debug)]
#[must_use = "a pending operation owns unique continuation authority"]
pub struct PendingOperation {
    ticket: PendingTicket,
    resume: VmResumeToken,
}

impl PendingOperation {
    pub const fn ticket(&self) -> PendingTicket {
        self.ticket
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: the ticket and token remain one logical pending
    /// registration and must not be rebound independently.
    pub fn into_parts(self) -> (PendingTicket, VmResumeToken) {
        (self.ticket, self.resume)
    }
}

impl VmRootSource for PendingOperation {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        // Roots already live in the scheduler-owned escrow identified by the
        // ticket; copying them here would create a second logical owner.
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmInternalTerminal {
    Budget(VmBudgetClosed),
    OwnerStopped,
}

/// Result claimed exactly once for a continuation token.
#[must_use = "a resume outcome may own VM roots"]
pub enum ResumeOutcome {
    Values(VmOwnedValues),
    /// Verified zero-result resume for `EmitStream` backpressure wakes. Stream
    /// natural end must use [`ResumeOutcome::StreamEnd`], never this variant.
    Empty,
    /// Explicit stream producer natural end. Natural end uses an independent
    /// end resume PC and zero-result resume path, not `EmitStream` backpressure.
    StreamEnd,
    /// A child boundary delivered the exact opaque exception envelope. The
    /// parent reuses this same envelope (`resume_throw`) without re-wrapping,
    /// so the actual catch identity stays unchanged.
    Throw(VmOwnedException),
    Failure(VmError),
    InternalTerminal(VmInternalTerminal),
}

impl VmRootSource for ResumeOutcome {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Values(values) => values.visit_roots(visitor),
            Self::Throw(exception) => exception.visit_roots(visitor),
            Self::Failure(error) => visit_vm_error(error, visitor),
            Self::Empty | Self::StreamEnd | Self::InternalTerminal(_) => Ok(()),
        }
    }
}

/// Sole outward control surface of the synchronous VM core.
#[must_use = "VM control must be handled by the scheduler"]
pub enum VmControl {
    Continue,
    Complete(VmResult),
    EnterChild(ChildInvocation),
    EnterAdapter(AdapterInvocation),
    EmitStream(StreamItem),
    Park(PendingOperation),
}

impl VmRootSource for VmControl {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Complete(Ok(values)) => values.visit_roots(visitor),
            // Root uncaught throws remain owned by the originating fiber's
            // exact UnwindState until the scheduler consumes the completion
            // into VmOwnedException/VmTerminalCause. Visiting the diagnostic
            // alias here would enumerate a second root authority.
            Self::Complete(Err(VmError::Thrown(_))) => Ok(()),
            Self::Complete(Err(error)) => visit_vm_error(error, visitor),
            Self::EnterChild(invocation) => invocation.visit_roots(visitor),
            Self::EnterAdapter(invocation) => invocation.visit_roots(visitor),
            Self::EmitStream(item) => item.visit_roots(visitor),
            Self::Continue | Self::Park(_) => Ok(()),
        }
    }
}

/// First result of a host effect. Synchronous failure stays on the Ready path.
#[must_use = "an effect start result may own roots or continuation authority"]
pub enum EffectStart {
    Ready(VmResult),
    EnterAdapter(AdapterInvocation),
    Pending(PendingOperation),
}

impl VmRootSource for EffectStart {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Ready(Ok(values)) => values.visit_roots(visitor),
            Self::EnterAdapter(invocation) => invocation.visit_roots(visitor),
            Self::Ready(Err(error)) => visit_vm_error(error, visitor),
            Self::Pending(_) => Ok(()),
        }
    }
}

/// First result of a service, Actor or callback boundary.
#[must_use = "a boundary start result may own roots or continuation authority"]
pub enum BoundaryStart {
    Ready(VmResult),
    EnterChild(ChildInvocation),
    OpenStreamChild(StreamInvocation),
    Pending(PendingOperation),
}

impl VmRootSource for BoundaryStart {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Ready(Ok(values)) => values.visit_roots(visitor),
            Self::EnterChild(invocation) => invocation.visit_roots(visitor),
            Self::OpenStreamChild(invocation) => invocation.visit_roots(visitor),
            Self::Ready(Err(error)) => visit_vm_error(error, visitor),
            Self::Pending(_) => Ok(()),
        }
    }
}

/// Outward control from a resumable native adapter frame.
#[must_use = "adapter control must be handled by the scheduler"]
pub enum AdapterControl {
    Continue,
    EnterChild(ChildInvocation),
    Complete(VmResult),
    Park(PendingOperation),
}

impl VmRootSource for AdapterControl {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::EnterChild(invocation) => invocation.visit_roots(visitor),
            Self::Complete(Ok(values)) => values.visit_roots(visitor),
            Self::Complete(Err(error)) => visit_vm_error(error, visitor),
            Self::Continue | Self::Park(_) => Ok(()),
        }
    }
}

pub(crate) fn visit_vm_error(
    error: &VmError,
    visitor: &mut dyn VmRootVisitor,
) -> Result<(), VmHeapError> {
    if let VmError::Thrown(envelope) = error {
        if let Some(slot) = envelope.vm_local_slot() {
            visitor.visit_root(&slot)?;
        }
    }
    Ok(())
}

pub(crate) fn visit_values(
    values: &[ValueSlot],
    visitor: &mut dyn VmRootVisitor,
) -> Result<(), VmHeapError> {
    for value in values {
        visitor.visit_root(value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };

    use super::{
        PendingOperation, PendingTicket, ResumeOutcome, VmControl, VmError, VmOwnedValues,
        VmResumeToken,
    };

    #[test]
    fn continuation_and_ticket_remain_compact_value_envelopes() {
        assert!(size_of::<VmResumeToken>() <= 96);
        assert_eq!(size_of::<PendingTicket>(), 8);
    }

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn scheduler_envelopes_have_bounded_inline_layouts() {
        assert!(size_of::<VmOwnedValues>() <= 48);
        assert!(size_of::<VmControl>() <= 192);
    }

    #[test]
    fn pending_handoff_consumes_only_unforgeable_resume_authority() {
        let into_pending: fn(VmResumeToken, PendingTicket) -> PendingOperation =
            VmResumeToken::into_pending;

        let _ = into_pending;
    }

    #[test]
    fn empty_owned_values_constructor_is_a_zero_value_authority() {
        fn empty(
            image: std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        ) -> VmOwnedValues {
            VmOwnedValues::empty(image)
        }

        let _ = empty;
    }

    #[test]
    fn empty_resume_outcome_is_a_public_zero_result_path() {
        let _: fn() -> ResumeOutcome = || ResumeOutcome::Empty;
    }

    #[test]
    fn stream_end_resume_outcome_is_public_and_rootless() {
        let _: fn() -> ResumeOutcome = || ResumeOutcome::StreamEnd;

        struct UnreachableVisitor;

        impl VmRootVisitor for UnreachableVisitor {
            fn visit_root(&mut self, _root: &ValueSlot) -> Result<(), VmHeapError> {
                panic!("StreamEnd must not own VM roots")
            }
        }

        ResumeOutcome::StreamEnd
            .visit_roots(&mut UnreachableVisitor)
            .unwrap();
    }

    #[test]
    fn stream_end_constructor_is_distinct_from_values_empty_and_failure() {
        let _: fn(VmOwnedValues) -> ResumeOutcome = ResumeOutcome::Values;
        let _: fn() -> ResumeOutcome = || ResumeOutcome::Empty;
        let _: fn() -> ResumeOutcome = || ResumeOutcome::StreamEnd;
        let _: fn(VmError) -> ResumeOutcome = ResumeOutcome::Failure;
    }
}

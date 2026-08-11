use std::{num::NonZeroU64, sync::Arc};

use skiff_runtime_bytecode_verifier::VerifiedLinkedBytecodeImage;
use skiff_runtime_deployment_image::DeploymentOwnerIdentity;
use skiff_runtime_linked_bytecode::{
    ActorMethodIndex, FunctionIndex, HostEffectAdapterIndex, InstructionIndex, InterfaceTableIndex,
    ResumeSiteIndex, ServiceOperationIndex, SyntheticCallbackIndex,
};
use skiff_runtime_model::{
    vm_heap::VmHeapError,
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::ValueSlot,
};

use crate::{VmBudgetError, VmError};

pub type VmResult = Result<VmOwnedValues, VmError>;

/// Values whose image-local handles remain pinned to the exact verified image
/// that created them.
///
/// Construction is crate-private so downstream code cannot attach a raw
/// `ValueSlot` to an unrelated image pin.
#[must_use = "owned VM values retain roots and an exact verified-image pin"]
pub struct VmOwnedValues {
    image: Arc<VerifiedLinkedBytecodeImage>,
    values: Box<[ValueSlot]>,
}

impl VmOwnedValues {
    pub(crate) fn new(image: Arc<VerifiedLinkedBytecodeImage>, values: Box<[ValueSlot]>) -> Self {
        Self { image, values }
    }

    pub const fn image(&self) -> &Arc<VerifiedLinkedBytecodeImage> {
        &self.image
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.image.owner()
    }

    pub fn values(&self) -> &[ValueSlot] {
        &self.values
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl VmRootSource for VmOwnedValues {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        visit_values(&self.values, visitor)
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
    image: Arc<VerifiedLinkedBytecodeImage>,
    sequence: u64,
    function: FunctionIndex,
    instruction: InstructionIndex,
    resume_instruction: InstructionIndex,
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
        image: Arc<VerifiedLinkedBytecodeImage>,
        sequence: u64,
        function: FunctionIndex,
        instruction: InstructionIndex,
        resume_instruction: InstructionIndex,
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
            resume_site,
            expected_stack_height,
            expected_result_count,
            authority,
        }
    }

    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    pub const fn image(&self) -> &Arc<VerifiedLinkedBytecodeImage> {
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
    resume: VmResumeToken,
}

impl ChildInvocation {
    #[allow(dead_code)]
    pub(crate) fn new(
        target: ChildTarget,
        arguments: VmOwnedValues,
        resume: VmResumeToken,
    ) -> Result<Self, VmError> {
        if resume.authority != VmResumeAuthority::Child(target)
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

impl VmRootSource for ChildInvocation {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.arguments.visit_roots(visitor)
    }
}

/// Owned invocation of one verified host-effect adapter.
#[must_use = "an adapter invocation owns arguments and unique continuation authority"]
pub struct AdapterInvocation {
    adapter: HostEffectAdapterIndex,
    arguments: VmOwnedValues,
    resume: VmResumeToken,
}

impl AdapterInvocation {
    #[allow(dead_code)]
    pub(crate) fn new(
        adapter: HostEffectAdapterIndex,
        arguments: VmOwnedValues,
        resume: VmResumeToken,
    ) -> Result<Self, VmError> {
        if resume.authority != VmResumeAuthority::Adapter(adapter)
            || !Arc::ptr_eq(arguments.image(), resume.image())
        {
            return Err(VmError::ResumeTokenMismatch);
        }
        Ok(Self {
            adapter,
            arguments,
            resume,
        })
    }

    pub const fn adapter(&self) -> HostEffectAdapterIndex {
        self.adapter
    }

    pub const fn arguments(&self) -> &VmOwnedValues {
        &self.arguments
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: all three parts remain one logical handoff and must
    /// not be exchanged with parts from another invocation.
    pub fn into_parts(self) -> (HostEffectAdapterIndex, VmOwnedValues, VmResumeToken) {
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
    resume: VmResumeToken,
}

impl StreamItem {
    #[allow(dead_code)]
    pub(crate) fn new(item: VmOwnedValues, resume: VmResumeToken) -> Result<Self, VmError> {
        if resume.authority != VmResumeAuthority::StreamItem
            || !Arc::ptr_eq(item.image(), resume.image())
            || item.len() != 1
        {
            return Err(VmError::ResumeTokenMismatch);
        }
        Ok(Self { item, resume })
    }

    pub const fn item(&self) -> &VmOwnedValues {
        &self.item
    }

    pub const fn resume(&self) -> &VmResumeToken {
        &self.resume
    }

    /// Scheduler-TCB seam: the item and token remain one logical handoff.
    pub fn into_parts(self) -> (VmOwnedValues, VmResumeToken) {
        (self.item, self.resume)
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
    Budget(VmBudgetError),
    OwnerStopped,
}

/// Result claimed exactly once for a continuation token.
#[must_use = "a resume outcome may own VM roots"]
pub enum ResumeOutcome {
    Values(VmOwnedValues),
    Throw(VmOwnedValues),
    Failure(VmError),
    InternalTerminal(VmInternalTerminal),
}

impl VmRootSource for ResumeOutcome {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Values(values) | Self::Throw(values) => values.visit_roots(visitor),
            Self::Failure(_) | Self::InternalTerminal(_) => Ok(()),
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
            Self::EnterChild(invocation) => invocation.visit_roots(visitor),
            Self::EnterAdapter(invocation) => invocation.visit_roots(visitor),
            Self::EmitStream(item) => item.visit_roots(visitor),
            Self::Continue | Self::Complete(Err(_)) | Self::Park(_) => Ok(()),
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
            Self::Ready(Err(_)) | Self::Pending(_) => Ok(()),
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
            Self::Ready(Err(_)) | Self::Pending(_) => Ok(()),
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
            Self::Continue | Self::Complete(Err(_)) | Self::Park(_) => Ok(()),
        }
    }
}

fn visit_values(values: &[ValueSlot], visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
    for value in values {
        visitor.visit_root(value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use super::{PendingOperation, PendingTicket, VmControl, VmOwnedValues, VmResumeToken};

    #[test]
    fn continuation_and_ticket_remain_compact_value_envelopes() {
        assert!(size_of::<VmResumeToken>() <= 80);
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
}

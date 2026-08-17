use std::{
    fmt,
    num::{NonZeroU32, NonZeroU64},
    sync::Arc,
};

use skiff_artifact_model::Opcode;
use skiff_runtime_linked_bytecode::{
    ActorMethodIndex, FunctionIndex, HostEffectAdapterIndex, InstructionIndex, InterfaceTableIndex,
    IntrinsicIndex, LinkedCallableSignature, LinkedRemoteInterfaceMethod,
    LinkedRemoteInterfaceTable, LinkedTaskTarget, LinkedTaskTiming, LinkedValueTransferPlan,
    ResumeSiteIndex, ServiceOperationIndex, ShapeIndex, SyntheticCallbackIndex, TaskTargetIndex,
    TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueKind, ValueSlot, VmHandle},
};

use crate::{lifecycle::LifecycleExecutor, VmBudgetClosed, VmError};

pub type VmResult = Result<VmOwnedValues, VmError>;

pub(crate) use crate::terminal_ownership::VmTerminalOwner;
pub use crate::terminal_ownership::{
    VmCompletion, VmLifecycleSite, VmOwnedException, VmOwnedExceptionRejected, VmOwnedValues,
    VmOwnedValuesRejected, VmResumeFailure, VmTerminalCause, VmTerminalEscrow, VmThrownDiagnostic,
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
    /// mutation. A heap failure returns the failing owner and every later
    /// suffix owner in an explicit carrier, so request terminal retention can
    /// retry or keep those roots ahead of heap teardown.
    pub fn release(self, heap: &mut dyn VmHeap) -> Result<(), VmHostEffectArgumentsReleaseError> {
        let Self {
            values,
            plans,
            function,
            instruction,
        } = self;
        let image = Arc::clone(values.image());
        let site = VmLifecycleSite {
            function,
            instruction,
            opcode: Opcode::InvokeHost,
        };
        if !plans.iter().all(LifecycleExecutor::supports_release) {
            let escrow = VmTerminalEscrow::from_slots(
                image,
                values.values().to_vec(),
                plans.iter().cloned().map(Some).collect::<Vec<_>>(),
                site,
            );
            return Err(VmHostEffectArgumentsReleaseError {
                error: VmError::FullValueLifecyclePlanUnavailable {
                    function,
                    instruction,
                    opcode: Opcode::InvokeHost,
                },
                escrow,
            });
        }
        let mut executor = LifecycleExecutor::new(heap);
        for (index, (value, plan)) in values.values().iter().zip(plans.iter()).enumerate() {
            if let Err(error) = executor.release(value, plan) {
                let remaining = values.values()[index..].to_vec();
                let remaining_plans = plans[index..].iter().cloned().map(Some).collect::<Vec<_>>();
                let escrow = VmTerminalEscrow::from_slots(image, remaining, remaining_plans, site);
                return Err(VmHostEffectArgumentsReleaseError {
                    error: error.into_vm_error(function, instruction, Opcode::InvokeHost),
                    escrow,
                });
            }
        }
        Ok(())
    }
}

impl VmRootSource for VmHostEffectArguments {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.values.visit_roots(visitor)
    }
}

/// A failed host-argument release that still owns the failing suffix.
///
/// Successful prefix owners are removed as they are released; the failing
/// owner and every later owner remain in this exact terminal escrow. No
/// runtime-kind fallback is inferred from the remaining slots.
#[must_use = "a host-argument release failure still owns its exact suffix"]
pub struct VmHostEffectArgumentsReleaseError {
    error: VmError,
    escrow: VmTerminalEscrow,
}

impl VmHostEffectArgumentsReleaseError {
    pub const fn error(&self) -> &VmError {
        &self.error
    }

    pub fn into_terminal_escrow(self) -> (VmError, VmTerminalEscrow) {
        (self.error, self.escrow)
    }
}

impl fmt::Debug for VmHostEffectArgumentsReleaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmHostEffectArgumentsReleaseError")
            .field("error", &self.error)
            .field("suffix_roots", &self.escrow.root_count())
            .finish_non_exhaustive()
    }
}

impl VmRootSource for VmHostEffectArgumentsReleaseError {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.escrow.visit_roots(visitor)
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
    binding: Arc<VmResumeBinding>,
}

/// Per-mint continuation identity and its complete linked resume descriptor.
///
/// The descriptor lives behind one private `Arc` so the token, the blocked
/// fiber, and any value carrier derived from that exact token can prove they
/// refer to the same continuation by pointer identity. Equal numeric fields
/// from another fiber are deliberately insufficient.
#[derive(Debug)]
pub(crate) struct VmResumeBinding {
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
    interface_plan: Option<InterfaceCallPlan>,
    task_plan: Option<TaskIntrinsicResumePlan>,
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
        interface_plan: Option<InterfaceCallPlan>,
        task_plan: Option<TaskIntrinsicResumePlan>,
    ) -> Self {
        Self {
            binding: Arc::new(VmResumeBinding {
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
                interface_plan,
                task_plan,
            }),
        }
    }

    pub fn sequence(&self) -> u64 {
        self.binding.sequence
    }

    pub fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.binding.image
    }

    pub fn function(&self) -> FunctionIndex {
        self.binding.function
    }

    pub fn instruction(&self) -> InstructionIndex {
        self.binding.instruction
    }

    pub fn resume_instruction(&self) -> InstructionIndex {
        self.binding.resume_instruction
    }

    pub fn end_resume_pc(&self) -> Option<InstructionIndex> {
        self.binding.end_resume_pc
    }

    pub fn resume_site(&self) -> ResumeSiteIndex {
        self.binding.resume_site
    }

    pub fn expected_stack_height(&self) -> u32 {
        self.binding.expected_stack_height
    }

    pub fn expected_result_count(&self) -> u32 {
        self.binding.expected_result_count
    }

    pub fn kind(&self) -> VmResumeKind {
        match self.binding.authority {
            VmResumeAuthority::Child(_) => VmResumeKind::Child,
            VmResumeAuthority::Adapter(_) => VmResumeKind::Adapter,
            VmResumeAuthority::StreamChild(_) => VmResumeKind::StreamChild,
            VmResumeAuthority::StreamItem => VmResumeKind::StreamItem,
        }
    }

    pub(crate) fn authority(&self) -> VmResumeAuthority {
        self.binding.authority
    }

    /// Exact linked interface call facts carried by this continuation.
    ///
    /// Non-interface resumes carry `None`; interface resumes must carry a
    /// complete plan before leaving the VM core.
    pub fn interface_plan(&self) -> Option<&InterfaceCallPlan> {
        self.binding.interface_plan.as_ref()
    }

    /// Exact linked task intrinsic result facts carried by this continuation.
    ///
    /// Non-task resumes carry `None`; task resumes must carry a complete plan
    /// before leaving the VM core.
    pub(crate) fn task_plan(&self) -> Option<&TaskIntrinsicResumePlan> {
        self.binding.task_plan.as_ref()
    }

    pub(crate) const fn binding(&self) -> &Arc<VmResumeBinding> {
        &self.binding
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

impl VmResumeBinding {
    pub(crate) const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub(crate) const fn function(&self) -> FunctionIndex {
        self.function
    }

    pub(crate) const fn instruction(&self) -> InstructionIndex {
        self.instruction
    }

    pub(crate) const fn resume_instruction(&self) -> InstructionIndex {
        self.resume_instruction
    }

    pub(crate) const fn end_resume_pc(&self) -> Option<InstructionIndex> {
        self.end_resume_pc
    }

    pub(crate) const fn expected_stack_height(&self) -> u32 {
        self.expected_stack_height
    }

    pub(crate) const fn expected_result_count(&self) -> u32 {
        self.expected_result_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmResumeKind {
    Child,
    Adapter,
    StreamChild,
    StreamItem,
}

/// Exact task dispatch target index.
///
/// The index is opaque to the VM core. It is deliberately non-zero so a
/// missing/ambiguous plan cannot be represented by a default integer value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskDispatchIndex(NonZeroU32);

impl TaskDispatchIndex {
    pub const fn new(value: NonZeroU32) -> Self {
        Self(value)
    }

    pub fn try_new(value: u32) -> Option<Self> {
        Some(Self(NonZeroU32::new(value)?))
    }

    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Maps the linker's zero-based task target table into the opaque,
    /// deliberately non-zero scheduler dispatch index.
    pub fn from_task_target_index(index: TaskTargetIndex) -> Option<Self> {
        Self::try_new(index.get().checked_add(1)?)
    }

    /// Recovers the exact linked task target row selected by this dispatch
    /// index. Returns `None` only for the invalid zero dispatch index.
    pub fn task_target_index(self) -> Option<TaskTargetIndex> {
        Some(TaskTargetIndex::new(self.0.get().checked_sub(1)?))
    }
}

/// K6-resolved submission timing for one durable task dispatch.
///
/// `Immediate` is carried only when the compiler/linked facts explicitly say
/// immediate. `After` and `At` are never synthesized from an expression index
/// or a default: the VM task intrinsic must supply the exact evaluated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDispatchTiming {
    Immediate,
    After { duration_ms: u64 },
    At { utc_millis: i64 },
}

impl TaskDispatchTiming {
    pub fn try_from_linked(timing: LinkedTaskTiming) -> Result<Self, TaskDispatchTimingError> {
        match timing {
            LinkedTaskTiming::Immediate => Ok(Self::Immediate),
            LinkedTaskTiming::After { expression } => {
                Err(TaskDispatchTimingError::MissingOperand {
                    kind: "after",
                    expression,
                })
            }
            LinkedTaskTiming::At { expression } => Err(TaskDispatchTimingError::MissingOperand {
                kind: "at",
                expression,
            }),
        }
    }

    pub(crate) fn resolve_from_slot(
        timing: LinkedTaskTiming,
        operand: Option<ValueSlot>,
    ) -> Result<Self, TaskDispatchTimingError> {
        match timing {
            LinkedTaskTiming::Immediate => match operand {
                None => Ok(Self::Immediate),
                Some(slot) => Err(TaskDispatchTimingError::UnexpectedOperand {
                    kind: "immediate",
                    actual: slot.kind(),
                }),
            },
            LinkedTaskTiming::After { expression } => {
                let slot = operand.ok_or(TaskDispatchTimingError::MissingOperand {
                    kind: "after",
                    expression,
                })?;
                if let Some(value) = slot.as_number() {
                    if value.is_finite()
                        && value >= 0.0
                        && value.fract() == 0.0
                        && value <= u64::MAX as f64
                    {
                        return Ok(Self::After {
                            duration_ms: value as u64,
                        });
                    }
                }
                if let Some(value) = slot.as_integer() {
                    if value >= 0 {
                        return Ok(Self::After {
                            duration_ms: value as u64,
                        });
                    }
                }
                Err(TaskDispatchTimingError::InvalidAfterValue {
                    expression,
                    kind: slot.kind(),
                })
            }
            LinkedTaskTiming::At { expression } => {
                let slot = operand.ok_or(TaskDispatchTimingError::MissingOperand {
                    kind: "at",
                    expression,
                })?;
                slot.as_date().map_or(
                    Err(TaskDispatchTimingError::InvalidAtValue {
                        expression,
                        kind: slot.kind(),
                    }),
                    |utc_millis| Ok(Self::At { utc_millis }),
                )
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskDispatchTimingError {
    NotTaskChild,
    MissingOperand {
        kind: &'static str,
        expression: u32,
    },
    UnexpectedOperand {
        kind: &'static str,
        actual: Option<ValueKind>,
    },
    InvalidAfterValue {
        expression: u32,
        kind: Option<ValueKind>,
    },
    InvalidAtValue {
        expression: u32,
        kind: Option<ValueKind>,
    },
}

impl fmt::Display for TaskDispatchTimingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTaskChild => formatter.write_str(
                "task child invocation carries no K6-resolved dispatch timing; only task continuations expose exact timing",
            ),
            Self::MissingOperand { kind, expression } => write!(
                formatter,
                "task dispatch {kind} timing expression {expression} has no K6-resolved VM timing value; exact timing must be supplied by the VM task intrinsic"
            ),
            Self::UnexpectedOperand { kind, actual } => write!(
                formatter,
                "task dispatch {kind} timing must not carry an unexpected VM timing operand (received {actual:?})"
            ),
            Self::InvalidAfterValue { expression, kind } => write!(
                formatter,
                "K6 could not resolve task dispatch after timing expression {expression} to a non-negative Duration (received {kind:?})"
            ),
            Self::InvalidAtValue { expression, kind } => write!(
                formatter,
                "K6 could not resolve task dispatch at timing expression {expression} to an Instant/Date (received {kind:?})"
            ),
        }
    }
}

impl std::error::Error for TaskDispatchTimingError {}

/// Sealed exact task intrinsic facts retained by a task continuation.
///
/// The VM mints this plan from the linked intrinsic row so the task resume
/// binder can validate the exact target and result plan without reconstructing
/// either from a runtime tag or table name, and the exact K6 timing stays with
/// the same continuation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskIntrinsicResumePlan {
    task_target: LinkedTaskTarget,
    result_type: TypeIndex,
    result_plan: LinkedValueTransferPlan,
    timing: TaskDispatchTiming,
}

impl TaskIntrinsicResumePlan {
    pub(crate) fn new(
        task_target: LinkedTaskTarget,
        result_type: TypeIndex,
        result_plan: LinkedValueTransferPlan,
        timing: TaskDispatchTiming,
    ) -> Self {
        Self {
            task_target,
            result_type,
            result_plan,
            timing,
        }
    }

    pub(crate) const fn task_target(&self) -> &LinkedTaskTarget {
        &self.task_target
    }

    pub(crate) const fn result_type(&self) -> TypeIndex {
        self.result_type
    }

    pub(crate) const fn result_plan(&self) -> &LinkedValueTransferPlan {
        &self.result_plan
    }

    pub(crate) const fn timing(&self) -> TaskDispatchTiming {
        self.timing
    }
}

/// Exact linked interface call facts carried with a continuation.
///
/// This plan is sealed by the VM and travels in the resume token so the flat
/// scheduler/request mux cannot reconstruct a method signature or carrier
/// plan from a table name or request metadata. Remote calls additionally
/// carry the exact linked remote method and table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceCallPlan {
    signature: LinkedCallableSignature,
    carrier_plan: LinkedValueTransferPlan,
    remote: Option<RemoteInterfaceCallPlan>,
}

impl InterfaceCallPlan {
    pub fn new(
        signature: LinkedCallableSignature,
        carrier_plan: LinkedValueTransferPlan,
        remote: Option<RemoteInterfaceCallPlan>,
    ) -> Self {
        Self {
            signature,
            carrier_plan,
            remote,
        }
    }

    pub const fn signature(&self) -> &LinkedCallableSignature {
        &self.signature
    }

    pub const fn carrier_plan(&self) -> &LinkedValueTransferPlan {
        &self.carrier_plan
    }

    pub const fn remote(&self) -> Option<&RemoteInterfaceCallPlan> {
        self.remote.as_ref()
    }
}

/// Exact remote method facts for one interface call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteInterfaceCallPlan {
    table: LinkedRemoteInterfaceTable,
    method: LinkedRemoteInterfaceMethod,
}

impl RemoteInterfaceCallPlan {
    pub fn new(table: LinkedRemoteInterfaceTable, method: LinkedRemoteInterfaceMethod) -> Self {
        Self { table, method }
    }

    pub const fn table(&self) -> &LinkedRemoteInterfaceTable {
        &self.table
    }

    pub const fn method(&self) -> &LinkedRemoteInterfaceMethod {
        &self.method
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildTarget {
    Service(ServiceOperationIndex),
    Actor(ActorMethodIndex),
    Interface {
        table: InterfaceTableIndex,
        method_ordinal: u32,
    },
    Db(IntrinsicIndex),
    Callback(SyntheticCallbackIndex),
    Task(TaskDispatchIndex),
    StreamNext,
}

/// Owned task dispatch request with exact VM arguments and raw request
/// payload bytes.
///
/// This is the payload materialization seam for T6F: the target index is
/// opaque and checked, the VM arguments remain VM-owned roots, and the
/// recoverable task payload bytes travel beside them without being interpreted
/// by the VM core or stored only in a host-side sidecar. The exact
/// K6-resolved timing is mandatory; there is no default.
#[must_use = "a task dispatch request owns VM arguments and payload bytes"]
pub struct TaskDispatchRequest {
    target: TaskDispatchIndex,
    arguments: VmOwnedValues,
    payload: Box<[u8]>,
    timing: TaskDispatchTiming,
}

impl TaskDispatchRequest {
    pub fn new(
        target: TaskDispatchIndex,
        arguments: VmOwnedValues,
        payload: Box<[u8]>,
        timing: TaskDispatchTiming,
    ) -> Self {
        Self {
            target,
            arguments,
            payload,
            timing,
        }
    }

    pub const fn target(&self) -> TaskDispatchIndex {
        self.target
    }

    pub const fn arguments(&self) -> &VmOwnedValues {
        &self.arguments
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub const fn timing(&self) -> TaskDispatchTiming {
        self.timing
    }

    pub fn into_parts(
        self,
    ) -> (
        TaskDispatchIndex,
        VmOwnedValues,
        Box<[u8]>,
        TaskDispatchTiming,
    ) {
        (self.target, self.arguments, self.payload, self.timing)
    }
}

impl VmRootSource for TaskDispatchRequest {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.arguments.visit_roots(visitor)
    }
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
            || resume.authority() != VmResumeAuthority::Child(target)
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
            || resume.authority() != VmResumeAuthority::Child(ChildTarget::StreamNext)
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

    /// Exact K6-resolved timing retained by a task intrinsic continuation.
    ///
    /// Non-task children fail closed: no caller may infer or default timing
    /// from a non-task invocation.
    pub fn task_dispatch_timing(&self) -> Result<TaskDispatchTiming, TaskDispatchTimingError> {
        self.resume
            .task_plan()
            .map(TaskIntrinsicResumePlan::timing)
            .ok_or(TaskDispatchTimingError::NotTaskChild)
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
        debug_assert_eq!(resume.authority(), VmResumeAuthority::Adapter(adapter));
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
        if resume.authority() != VmResumeAuthority::StreamChild(target)
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
/// outcome may instead consume this error through [`Self::into_terminal_escrow`]
/// and transfer the values plus their captured lifecycle authority into
/// explicit terminal cleanup escrow.
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

    /// Transfers the failed value owner and its exact lifecycle plan into
    /// terminal cleanup escrow.
    ///
    /// The continuation is deliberately abandoned only on this explicit
    /// terminal path; callers that may resume must use [`Self::into_parts`].
    pub fn into_terminal_escrow(self) -> (VmTerminalEscrow, VmError) {
        let Self { item, error } = self;
        let StreamItem { item, .. } = item;
        (item.into_terminal_escrow(), error)
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
        debug_assert_eq!(resume.authority(), VmResumeAuthority::StreamItem);
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

    pub fn into_parts(self) -> (VmOwnedValues, VmResumeToken) {
        (self.item, self.resume)
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
    Complete(VmCompletion),
    EnterChild(ChildInvocation),
    EnterAdapter(AdapterInvocation),
    EmitStream(StreamItem),
    Park(PendingOperation),
}

impl VmRootSource for VmControl {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Complete(completion) => completion.visit_roots(visitor),
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
    _error: &VmError,
    _visitor: &mut dyn VmRootVisitor,
) -> Result<(), VmHeapError> {
    // `VmError` is diagnostic-only. Exact thrown payload authority can only
    // live in VmOwnedException/VmCompletion/VmTerminalCause.
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

    use skiff_artifact_model::{
        CallableEffectSummary, ContractOperationId, PackageBuildId, ParamModeIr,
        ServiceProtocolIdentity, ServiceRequirementKey,
    };
    use skiff_runtime_linked_bytecode::{
        LinkedCallableSignature, LinkedPublicInstanceKey, LinkedRemoteInterfaceMethod,
        LinkedRemoteInterfaceTable, LinkedTaskTiming, LinkedValueDropPlan, LinkedValueTransferPlan,
        TaskTargetIndex, TypeIndex,
    };
    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::{VmRootSource, VmRootVisitor},
        vm_value::ValueSlot,
    };

    use super::{
        ChildTarget, InterfaceCallPlan, PendingOperation, PendingTicket, ResumeOutcome,
        TaskDispatchIndex, TaskDispatchRequest, TaskDispatchTiming, TaskDispatchTimingError,
        VmControl, VmError, VmOwnedValues, VmResumeToken,
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

    #[test]
    fn task_dispatch_index_is_nonzero_and_opaque() {
        assert!(TaskDispatchIndex::try_new(0).is_none());
        let first = TaskDispatchIndex::try_new(1).expect("one is valid");
        let second = TaskDispatchIndex::try_new(2).expect("two is valid");
        assert_ne!(first, second);
        assert_eq!(
            first,
            TaskDispatchIndex::from_task_target_index(TaskTargetIndex::new(0))
                .expect("zero-based task target maps to one")
        );
        assert_eq!(first.task_target_index(), Some(TaskTargetIndex::new(0)));
        assert_eq!(
            TaskDispatchIndex::from_task_target_index(TaskTargetIndex::new(u32::MAX)),
            None
        );
        assert_eq!(
            ChildTarget::Task(first),
            ChildTarget::Task(TaskDispatchIndex::try_new(1).expect("one is valid"))
        );
        assert_ne!(ChildTarget::Task(first), ChildTarget::Task(second));
    }

    #[test]
    fn task_dispatch_request_exposes_arguments_and_raw_payload() {
        fn payload_materializer(
            request: &TaskDispatchRequest,
        ) -> (&VmOwnedValues, &[u8], TaskDispatchTiming) {
            (request.arguments(), request.payload(), request.timing())
        }
        fn assert_root_source<T: VmRootSource>() {}

        let _ = payload_materializer;
        assert_root_source::<TaskDispatchRequest>();
    }

    #[test]
    fn task_dispatch_timing_resolution_fails_closed_without_operand() {
        assert_eq!(
            TaskDispatchTiming::try_from_linked(LinkedTaskTiming::Immediate),
            Ok(TaskDispatchTiming::Immediate)
        );
        assert!(matches!(
            TaskDispatchTiming::try_from_linked(LinkedTaskTiming::After { expression: 4 }),
            Err(TaskDispatchTimingError::MissingOperand {
                kind: "after",
                expression: 4
            })
        ));
        assert!(matches!(
            TaskDispatchTiming::try_from_linked(LinkedTaskTiming::At { expression: 5 }),
            Err(TaskDispatchTimingError::MissingOperand {
                kind: "at",
                expression: 5
            })
        ));
    }

    #[test]
    fn task_dispatch_timing_rejects_invalid_resolved_values() {
        assert!(matches!(
            TaskDispatchTiming::resolve_from_slot(
                LinkedTaskTiming::Immediate,
                Some(ValueSlot::number(0.0)),
            ),
            Err(TaskDispatchTimingError::UnexpectedOperand { .. })
        ));
        assert!(TaskDispatchTiming::resolve_from_slot(
            LinkedTaskTiming::After { expression: 4 },
            Some(ValueSlot::number(-1.0)),
        )
        .is_err());
        assert!(TaskDispatchTiming::resolve_from_slot(
            LinkedTaskTiming::After { expression: 4 },
            Some(ValueSlot::number(f64::NAN)),
        )
        .is_err());
        assert!(TaskDispatchTiming::resolve_from_slot(
            LinkedTaskTiming::At { expression: 5 },
            Some(ValueSlot::number(0.0)),
        )
        .is_err());
    }

    #[test]
    fn interface_call_plan_keeps_exact_signature_and_carrier_plan() {
        let plan = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        };
        let signature = LinkedCallableSignature::new(
            Box::new([TypeIndex::new(0), TypeIndex::new(1)]),
            Box::new([ParamModeIr::Value, ParamModeIr::Value]),
            Box::new([plan.clone(), plan.clone()]),
            Box::new([TypeIndex::new(2)]),
            Box::new([plan.clone()]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("interface signature is canonical");
        let call = InterfaceCallPlan::new(signature.clone(), plan.clone(), None);

        assert_eq!(call.signature(), &signature);
        assert_eq!(call.carrier_plan(), &plan);
        assert!(call.remote().is_none());
    }

    #[test]
    fn remote_interface_call_plan_keeps_exact_remote_method_and_table() {
        let plan = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        };
        let signature = LinkedCallableSignature::new(
            Box::new([TypeIndex::new(0)]),
            Box::new([ParamModeIr::Value]),
            Box::new([plan.clone()]),
            Box::new([]),
            Box::new([]),
            CallableEffectSummary::analysis_pending(),
        )
        .expect("remote method signature is canonical");
        let method = LinkedRemoteInterfaceMethod::new(
            0,
            skiff_runtime_linked_bytecode::LinkedInterfaceMethodAbiId::parse("reader:method:read")
                .expect("method ABI id"),
            signature,
            ContractOperationId::new("operation:reader.read"),
        );
        let table = LinkedRemoteInterfaceTable::new(
            ServiceRequirementKey {
                caller_package_build_id: PackageBuildId::new("build:caller"),
                service_requirement_slot: 0,
            },
            LinkedPublicInstanceKey::parse("instance:reader").expect("public instance key"),
            Box::new([method.clone()]),
            ServiceProtocolIdentity::new("protocol:reader-v1"),
        )
        .expect("remote table is canonical");
        let call = InterfaceCallPlan::new(
            method.signature().clone(),
            plan,
            Some(super::RemoteInterfaceCallPlan::new(
                table.clone(),
                method.clone(),
            )),
        );
        let remote = call.remote().expect("remote plan");
        assert_eq!(remote.table(), &table);
        assert_eq!(remote.method(), &method);
    }
}

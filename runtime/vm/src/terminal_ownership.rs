use std::{fmt, sync::Arc};

use skiff_artifact_model::{InstructionSourceSite, Opcode, TypeRefIr};
use skiff_runtime_deployment_image::DeploymentOwnerIdentity;
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedDbOperation, LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_linker::DeploymentExecutionImage;
use skiff_runtime_model::{
    service_error::{CatchIdentity, ErrorCorrelation, ExceptionStackFrame, RequestException},
    vm_heap::{VmHeap, VmHeapError},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{ValueKind, ValueSlot},
};

use crate::{
    admission::is_discardable_root,
    control::{
        visit_values, visit_vm_error, ChildTarget, ResumeOutcome, VmResumeAuthority,
        VmResumeBinding, VmResumeKind, VmResumeToken,
    },
    fiber::runtime_leaf_catch_identity,
    lifecycle::LifecycleExecutor,
    VmError,
};

/// Values whose image-local handles remain pinned to the exact verified image
/// that created them.
///
/// Construction is crate-private so downstream code cannot attach a raw
/// `ValueSlot` to an unrelated image pin.
#[must_use = "owned VM values retain roots and an exact verified-image pin"]
pub struct VmOwnedValues {
    image: Arc<DeploymentExecutionImage>,
    values: Box<[ValueSlot]>,
    /// Exact producer-side lifecycle plans. `None` is reserved for an
    /// internally detected damaged invariant and can only become retained
    /// terminal ownership; it is never inferred again from a runtime tag.
    release_plans: Box<[Option<LinkedValueTransferPlan>]>,
    /// Present only for values materialized against one exact live resume
    /// token. Pointer identity is the unforgeable continuation binding; all
    /// descriptor fields and plans were validated before this Arc was stored.
    resume_binding: Option<Arc<VmResumeBinding>>,
}

impl VmOwnedValues {
    pub(crate) fn new_exact(
        image: Arc<DeploymentExecutionImage>,
        values: Box<[ValueSlot]>,
        plans: Box<[LinkedValueTransferPlan]>,
    ) -> Self {
        let release_plans = if values.len() == plans.len() {
            plans.into_vec().into_iter().map(Some).collect()
        } else {
            // A linked invariant mismatch is damaged state. Preserve every
            // owner and refuse to guess its cleanup primitive.
            vec![None; values.len()].into_boxed_slice()
        };
        Self {
            image,
            values,
            release_plans,
            resume_binding: None,
        }
    }

    /// Creates an owned, zero-result resume envelope pinned to `image`.
    ///
    /// The only externally constructible `VmOwnedValues` is empty: it can
    /// resume a verified zero-result site such as `EmitStream`, but cannot
    /// attach a raw `ValueSlot` to an unrelated image pin.
    pub(crate) fn empty(image: Arc<DeploymentExecutionImage>) -> Self {
        Self {
            image,
            values: Box::new([]),
            release_plans: Box::new([]),
            resume_binding: None,
        }
    }

    /// Binds externally materialized values to the exact result authority of
    /// one live resume token. The token is borrowed, so a rejection returns
    /// all values while the scheduler still retains the unique continuation.
    pub fn try_from_resume(
        resume: &VmResumeToken,
        values: Box<[ValueSlot]>,
    ) -> Result<Self, VmOwnedValuesRejected> {
        let image = Arc::clone(resume.image());
        let Some(site) = image
            .resume_sites()
            .get(resume.resume_site())
            .filter(|site| {
                site.index() == resume.resume_site()
                    && site.function() == resume.function()
                    && site.site() == resume.instruction()
                    && site.resume() == resume.resume_instruction()
                    && site.end_resume() == resume.end_resume_pc()
                    && site.expected_stack_height_before_result() == resume.expected_stack_height()
            })
        else {
            return Err(VmOwnedValuesRejected::new(
                image,
                VmError::ResumeTokenMismatch,
                values,
            ));
        };
        let expected = usize::try_from(resume.expected_result_count()).unwrap_or(usize::MAX);
        if values.len() != expected
            || site.result_types().len() != expected
            || site.result_plans().len() != expected
        {
            return Err(VmOwnedValuesRejected::new(
                image,
                VmError::ResumeShapeMismatch {
                    expected,
                    actual: values.len(),
                },
                values,
            ));
        }
        if values
            .iter()
            .zip(site.result_types())
            .zip(site.result_plans())
            .any(|((value, ty), plan)| !resume_value_matches(&image, value, *ty, plan))
        {
            return Err(VmOwnedValuesRejected::new(
                image,
                VmError::ResumeTokenMismatch,
                values,
            ));
        }
        let plans = site.result_plans().to_vec().into_boxed_slice();
        let mut owned = Self::new_exact(image, values, plans);
        owned.resume_binding = Some(Arc::clone(resume.binding()));
        Ok(owned)
    }

    /// Binds one DB intrinsic result to the exact continuation minted by the
    /// VM for the linked DB operation.
    ///
    /// The intrinsic opcode carries no image resume-site row, so this is the
    /// checked DB-specific binder. It still validates the caller image, the
    /// exact operation result type/plan, the value tag, and the same private
    /// binding pointer before any value can be delivered to the fiber.
    pub fn try_from_db_intrinsic_resume(
        resume: &VmResumeToken,
        values: Box<[ValueSlot]>,
        operation: &LinkedDbOperation,
    ) -> Result<Self, VmOwnedValuesRejected> {
        let image = Arc::clone(resume.image());
        if !matches!(
            resume.authority(),
            VmResumeAuthority::Child(ChildTarget::Db(_))
        ) || resume.expected_result_count() != 1
            || values.len() != 1
        {
            return Err(VmOwnedValuesRejected::new(
                image,
                VmError::ResumeTokenMismatch,
                values,
            ));
        }
        let result_type = operation.result_type();
        let result_plan = operation.result_plan();
        if image.type_plan(result_type) != Some(result_plan)
            || !resume_value_matches(&image, &values[0], result_type, result_plan)
        {
            return Err(VmOwnedValuesRejected::new(
                image,
                VmError::FullValueLifecyclePlanUnavailable {
                    function: resume.function(),
                    instruction: resume.instruction(),
                    opcode: Opcode::InvokeIntrinsic,
                },
                values,
            ));
        }
        let mut owned = Self::new_exact(image, values, Box::new([result_plan.clone()]));
        owned.resume_binding = Some(Arc::clone(resume.binding()));
        Ok(owned)
    }

    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
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

    pub(crate) fn is_bound_to(&self, binding: &Arc<VmResumeBinding>) -> bool {
        self.resume_binding
            .as_ref()
            .is_some_and(|owned| Arc::ptr_eq(owned, binding))
    }

    /// Consumes values that can no longer be delivered to their verified
    /// destination and moves every physical owner into terminal cleanup.
    /// Exact plans were captured at the producer or resume-token boundary;
    /// damaged entries remain explicitly retained without tag-based lookup.
    pub fn into_terminal_escrow(self) -> VmTerminalEscrow {
        let site = VmLifecycleSite {
            function: FunctionIndex::new(0),
            instruction: InstructionIndex::new(0),
            opcode: Opcode::Return,
        };
        VmTerminalEscrow::from_slots(
            self.image,
            self.values.into_vec(),
            self.release_plans.into_vec(),
            site,
        )
    }
}

impl VmRootSource for VmOwnedValues {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        visit_values(&self.values, visitor)
    }
}

/// Owner-returning rejection from [`VmOwnedValues::try_from_resume`].
/// Construction is sealed; callers may inspect the error and then consume
/// the carrier to recover the unchanged values without dropping roots.
#[must_use = "rejected resume values still own their original VM roots"]
pub struct VmOwnedValuesRejected {
    image: Arc<DeploymentExecutionImage>,
    error: VmError,
    values: Box<[ValueSlot]>,
}

impl VmOwnedValuesRejected {
    fn new(image: Arc<DeploymentExecutionImage>, error: VmError, values: Box<[ValueSlot]>) -> Self {
        Self {
            image,
            error,
            values,
        }
    }

    pub const fn error(&self) -> &VmError {
        &self.error
    }

    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub fn values(&self) -> &[ValueSlot] {
        &self.values
    }

    /// Converts a rejected, therefore untrusted, value batch into explicit
    /// damaged retention. No runtime tag is allowed to mint a cleanup plan.
    pub fn into_terminal_escrow(self) -> (VmError, VmTerminalEscrow) {
        let Self {
            image,
            error,
            values,
        } = self;
        let plans = vec![None; values.len()];
        let escrow = VmTerminalEscrow::from_slots(
            image,
            values.into_vec(),
            plans,
            VmLifecycleSite {
                function: FunctionIndex::new(0),
                instruction: InstructionIndex::new(0),
                opcode: Opcode::Return,
            },
        );
        (error, escrow)
    }
}

impl fmt::Debug for VmOwnedValuesRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmOwnedValuesRejected")
            .field("error", &self.error)
            .field("value_count", &self.values.len())
            .finish()
    }
}

impl VmRootSource for VmOwnedValuesRejected {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        visit_values(&self.values, visitor)
    }
}

fn resume_value_matches(
    image: &DeploymentExecutionImage,
    value: &ValueSlot,
    expected: TypeIndex,
    plan: &LinkedValueTransferPlan,
) -> bool {
    let Some(kind) = value.kind() else {
        return false;
    };
    match kind {
        ValueKind::Null
        | ValueKind::Bool
        | ValueKind::Number
        | ValueKind::Integer
        | ValueKind::Date => image
            .types()
            .get(expected.get() as usize)
            .filter(|entry| entry.index() == expected)
            .is_some_and(|entry| immediate_type_matches(entry.type_ref(), kind)),
        ValueKind::RequestHeapRef => {
            // The resume TCB binds these values to the request heap shared by
            // producer and receiver; the image-local tag and exact linked plan
            // then prove the destination carrier. Other reference kinds need
            // their own origin/table authority and are rejected below.
            runtime_tag_matches(value, expected)
                && matches!(
                    plan,
                    LinkedValueTransferPlan::SnapshotShare { .. }
                        | LinkedValueTransferPlan::MoveOnly { .. }
                )
        }
        ValueKind::ConstRef
        | ValueKind::ResourceRef
        | ValueKind::ActorStateRef
        | ValueKind::CallbackClosureRef => false,
    }
}

fn runtime_tag_matches(value: &ValueSlot, expected: TypeIndex) -> bool {
    value
        .compact_type_tag()
        .is_some_and(|tag| tag.type_index() == expected.get())
}

fn immediate_type_matches(expected: &TypeRefIr, actual: ValueKind) -> bool {
    let TypeRefIr::Builtin { name, args } = expected else {
        return false;
    };
    args.is_empty()
        && matches!(
            (name.as_str(), actual),
            ("null", ValueKind::Null)
                | ("bool", ValueKind::Bool)
                | ("number", ValueKind::Number)
                | ("integer", ValueKind::Integer)
                | ("Date", ValueKind::Date)
        )
}

/// Non-cloneable ownership carrier for roots left behind by a terminal VM
/// fiber. The scheduler must move this value across every terminal boundary
/// before dropping the corresponding fiber or trampoline. Request teardown
/// may release its owners synchronously, but a failed release leaves the
/// failing owner and the remaining suffix inside this same root source.
#[must_use = "terminal VM roots must be released or retained through request heap teardown"]
pub struct VmTerminalEscrow {
    images: Vec<Arc<DeploymentExecutionImage>>,
    owners: Vec<VmTerminalOwner>,
}

pub(crate) struct VmTerminalOwner {
    value: ValueSlot,
    release: VmTerminalReleaseAuthority,
    site: VmLifecycleSite,
    diagnostic_index: usize,
}

pub(crate) enum VmTerminalReleaseAuthority {
    /// An exact compiler-emitted plan resolved into the pinned execution
    /// image. Cleanup must consume this complete plan rather than infer a
    /// physical primitive from `ValueKind`.
    Exact(LinkedValueTransferPlan),
    /// An owned value observed only after linked state was found damaged.
    /// Keeping it explicit prevents an absent plan from silently becoming a
    /// kind-based release. The request retains this owner until the concrete
    /// heap/resource authority is torn down.
    DamagedRetained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VmLifecycleSite {
    pub function: FunctionIndex,
    pub instruction: InstructionIndex,
    pub opcode: Opcode,
}

/// An opaque exception envelope bound to the execution image that produced
/// its VM-local payload. Construction stays inside the VM crate; scheduler
/// code can only move, inspect, root-walk, or consume this carrier. The exact
/// linked plan is captured at construction, before the image-local TypeIndex
/// can cross a continuation boundary.
#[must_use = "an owned VM exception must be resumed, released, or retained"]
pub struct VmOwnedException {
    image: Arc<DeploymentExecutionImage>,
    exception: Arc<RequestException>,
    diagnostic: VmThrownDiagnostic,
    owner: Option<VmTerminalOwner>,
    resume_binding: Option<Arc<VmResumeBinding>>,
}

/// Rootless metadata for one VM-local exception.
///
/// This snapshot deliberately contains neither `RequestException` nor
/// `ValueSlot`. It may be cloned or retained after the exact payload owner is
/// transferred or released without exposing a stale heap handle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmThrownDiagnostic {
    identity: Option<CatchIdentity>,
    source: InstructionSourceSite,
    stack: Box<[ExceptionStackFrame]>,
    correlation: ErrorCorrelation,
}

impl VmThrownDiagnostic {
    fn from_exception(exception: &RequestException) -> Self {
        Self {
            identity: exception.actual_catch_identity().cloned(),
            source: exception.source().clone(),
            stack: exception.stack().to_vec().into_boxed_slice(),
            correlation: exception.correlation().clone(),
        }
    }

    pub const fn identity(&self) -> Option<&CatchIdentity> {
        self.identity.as_ref()
    }

    pub const fn source(&self) -> &InstructionSourceSite {
        &self.source
    }

    pub const fn stack(&self) -> &[ExceptionStackFrame] {
        &self.stack
    }

    pub const fn correlation(&self) -> &ErrorCorrelation {
        &self.correlation
    }
}

impl VmOwnedException {
    pub(crate) fn from_origin_authority(
        image: Arc<DeploymentExecutionImage>,
        exception: Arc<RequestException>,
        plan: Option<LinkedValueTransferPlan>,
        site: VmLifecycleSite,
    ) -> Self {
        let diagnostic = VmThrownDiagnostic::from_exception(&exception);
        let owner = exception.vm_local_slot().and_then(|value| {
            if is_discardable_root(&value) {
                return None;
            }
            Some(match plan.clone() {
                Some(plan) => VmTerminalOwner::exact(value, plan, site, 0),
                None => VmTerminalOwner::damaged_retained(value, site, 0),
            })
        });
        Self {
            image,
            exception,
            diagnostic,
            owner,
            resume_binding: None,
        }
    }

    /// Mints a caller-owned exception from a provider `VmOwnedException`
    /// diagnostic and a payload already materialized into the caller heap.
    ///
    /// The caller image and resume token must agree, the payload must be a
    /// live caller-heap reference whose caller-image leaf identity matches the
    /// diagnostic, and the linked caller plan must be exactly the image plan
    /// for that payload type. The returned envelope is bound to this exact
    /// resume token; an origin-owned envelope has no token binding and remains
    /// accepted for the same-image throw seam.
    pub fn try_from_caller_resume(
        caller_image: Arc<DeploymentExecutionImage>,
        resume: &VmResumeToken,
        caller_heap: &dyn VmHeap,
        payload: Option<ValueSlot>,
        diagnostic: &VmThrownDiagnostic,
        plan: LinkedValueTransferPlan,
        site: VmLifecycleSite,
    ) -> Result<Self, VmOwnedExceptionRejected> {
        if !Arc::ptr_eq(&caller_image, resume.image()) {
            return Err(VmOwnedExceptionRejected::new(
                Arc::clone(resume.image()),
                VmError::ResumeTokenMismatch,
                payload,
            ));
        }
        if !matches!(resume.kind(), VmResumeKind::Child)
            || site.function != resume.function()
            || site.instruction != resume.instruction()
        {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeTokenMismatch,
                payload,
            ));
        }
        let Some(value) = payload else {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                None,
            ));
        };
        if !matches!(value.kind(), Some(ValueKind::RequestHeapRef)) {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                Some(value),
            ));
        }
        if let Err(error) = caller_heap.validate_live(&value) {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::Heap(error),
                Some(value),
            ));
        }
        let Some(leaf) = value
            .compact_type_tag()
            .map(|tag| TypeIndex::new(tag.type_index()))
        else {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                Some(value),
            ));
        };
        let Some(actual_identity) = runtime_leaf_catch_identity(&caller_image, &value) else {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                Some(value),
            ));
        };
        let Some(caller_identity) = diagnostic.identity() else {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                Some(value),
            ));
        };
        if caller_identity != &actual_identity {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::ResumeThrowEnvelopeUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                },
                Some(value),
            ));
        }
        let Some(image_plan) = caller_image.type_plan(leaf) else {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::FullValueLifecyclePlanUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                    opcode: site.opcode,
                },
                Some(value),
            ));
        };
        if image_plan != &plan || !LifecycleExecutor::supports_release(&plan) {
            return Err(VmOwnedExceptionRejected::new(
                caller_image,
                VmError::FullValueLifecyclePlanUnavailable {
                    function: site.function,
                    instruction: site.instruction,
                    opcode: site.opcode,
                },
                Some(value),
            ));
        }
        let exception = match RequestException::local_vm(
            value,
            actual_identity.clone(),
            diagnostic.source().clone(),
            diagnostic.stack().to_vec(),
            diagnostic.correlation().clone(),
        ) {
            Ok(exception) => exception,
            Err(_) => {
                return Err(VmOwnedExceptionRejected::new(
                    caller_image,
                    VmError::ResumeThrowEnvelopeUnavailable {
                        function: site.function,
                        instruction: site.instruction,
                    },
                    Some(value),
                ));
            }
        };
        Ok(Self {
            image: caller_image,
            exception: Arc::new(exception),
            diagnostic: diagnostic.clone(),
            owner: Some(VmTerminalOwner::exact(value, plan, site, 0)),
            resume_binding: Some(Arc::clone(resume.binding())),
        })
    }

    pub fn origin_owner(&self) -> &DeploymentOwnerIdentity {
        self.image.owner()
    }

    pub const fn origin_image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub(crate) fn is_bound_to(&self, binding: &Arc<VmResumeBinding>) -> bool {
        self.resume_binding
            .as_ref()
            .map_or(true, |owned| Arc::ptr_eq(owned, binding))
    }

    pub const fn diagnostic(&self) -> &VmThrownDiagnostic {
        &self.diagnostic
    }

    /// Returns the exact VM-local payload owner retained by this exception.
    ///
    /// The payload is still owned by the exception and by its origin heap. A
    /// child boundary may read it for typed materialization before consuming
    /// this exception through `release_all` or terminal escrow.
    pub fn vm_local_payload(&self) -> Option<ValueSlot> {
        self.exception.vm_local_slot()
    }

    pub(crate) fn exception(&self) -> &RequestException {
        &self.exception
    }

    pub fn root_count(&self) -> usize {
        usize::from(self.owner.is_some())
    }

    pub fn unresolved_count(&self) -> usize {
        usize::from(self.owner.as_ref().is_some_and(|owner| {
            matches!(&owner.release, VmTerminalReleaseAuthority::DamagedRetained)
        }))
    }

    /// Releases the cause-owned payload only after request projection has
    /// borrowed it. A failed release leaves the exact same owner in this
    /// carrier for retention/retry; success makes future root visits empty.
    pub fn release_all(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let Some(owner) = self.owner.as_ref() else {
            return Ok(());
        };
        match &owner.release {
            VmTerminalReleaseAuthority::Exact(plan) => {
                LifecycleExecutor::new(heap)
                    .release(&owner.value, plan)
                    .map_err(|error| {
                        error.into_vm_error(
                            owner.site.function,
                            owner.site.instruction,
                            owner.site.opcode,
                        )
                    })?;
            }
            VmTerminalReleaseAuthority::DamagedRetained => {
                return Err(VmError::TerminalRootLifecycleUnavailable {
                    index: owner.diagnostic_index,
                    kind: owner.value.kind(),
                });
            }
        }
        self.owner = None;
        Ok(())
    }

    pub(crate) fn into_terminal_escrow(self) -> VmTerminalEscrow {
        let Self {
            image,
            exception: _,
            diagnostic: _,
            owner,
            resume_binding: _,
        } = self;
        VmTerminalEscrow::new(image, owner.into_iter().collect())
    }

    pub(crate) fn into_unwind_parts(
        self,
    ) -> (Arc<RequestException>, Option<LinkedValueTransferPlan>) {
        let plan = self.owner.and_then(|owner| match owner.release {
            VmTerminalReleaseAuthority::Exact(plan) => Some(plan),
            VmTerminalReleaseAuthority::DamagedRetained => None,
        });
        (self.exception, plan)
    }
}

impl fmt::Debug for VmOwnedException {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmOwnedException")
            .field("origin", self.origin_owner())
            .field("root_count", &self.root_count())
            .field("unresolved_count", &self.unresolved_count())
            .finish()
    }
}

impl VmRootSource for VmOwnedException {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        if let Some(owner) = &self.owner {
            visitor.visit_root(&owner.value)?;
        }
        Ok(())
    }
}

/// Owner-returning rejection from
/// [`VmOwnedException::try_from_caller_resume`].
#[must_use = "a rejected cross-image throw payload still owns its caller-heap root"]
pub struct VmOwnedExceptionRejected {
    image: Arc<DeploymentExecutionImage>,
    error: VmError,
    payload: Option<ValueSlot>,
}

impl VmOwnedExceptionRejected {
    fn new(
        image: Arc<DeploymentExecutionImage>,
        error: VmError,
        payload: Option<ValueSlot>,
    ) -> Self {
        Self {
            image,
            error,
            payload,
        }
    }

    pub const fn error(&self) -> &VmError {
        &self.error
    }

    pub const fn image(&self) -> &Arc<DeploymentExecutionImage> {
        &self.image
    }

    pub const fn payload(&self) -> Option<ValueSlot> {
        self.payload
    }

    pub fn into_parts(self) -> (VmError, Option<ValueSlot>) {
        (self.error, self.payload)
    }
}

impl fmt::Debug for VmOwnedExceptionRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmOwnedExceptionRejected")
            .field("error", &self.error)
            .field("payload_kind", &self.payload.and_then(|value| value.kind()))
            .finish()
    }
}

impl VmRootSource for VmOwnedExceptionRejected {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        if let Some(value) = self.payload {
            visitor.visit_root(&value)?;
        }
        Ok(())
    }
}

/// Linear completion of one VM fiber.
///
/// Construction is sealed in the VM. The primary result and every residual
/// terminal owner are moved into this value in the same heap-free commit, so
/// the producing fiber is rootless before control returns to the scheduler.
#[must_use = "a VM completion must be resumed or converted into terminal retention"]
pub struct VmCompletion {
    kind: Box<VmCompletionKind>,
    residual: VmTerminalEscrow,
}

enum VmCompletionKind {
    Returned(VmOwnedValues),
    Thrown(VmOwnedException),
    Failed {
        image: Arc<DeploymentExecutionImage>,
        diagnostic: VmFailureDiagnostic,
    },
}

enum VmFailureDiagnostic {
    Error(VmError),
    Thrown(VmThrownDiagnostic),
}

impl VmCompletion {
    pub(crate) fn returned(values: VmOwnedValues, residual: VmTerminalEscrow) -> Self {
        Self {
            kind: Box::new(VmCompletionKind::Returned(values)),
            residual,
        }
    }

    pub(crate) fn thrown(exception: VmOwnedException, residual: VmTerminalEscrow) -> Self {
        Self {
            kind: Box::new(VmCompletionKind::Thrown(exception)),
            residual,
        }
    }

    pub(crate) fn failed(
        image: Arc<DeploymentExecutionImage>,
        error: VmError,
        residual: VmTerminalEscrow,
    ) -> Self {
        let diagnostic = match error {
            VmError::Thrown(exception) => {
                VmFailureDiagnostic::Thrown(VmThrownDiagnostic::from_exception(&exception))
            }
            error => VmFailureDiagnostic::Error(error),
        };
        Self {
            kind: Box::new(VmCompletionKind::Failed { image, diagnostic }),
            residual,
        }
    }

    pub fn returned_values(&self) -> Option<&VmOwnedValues> {
        match self.kind.as_ref() {
            VmCompletionKind::Returned(values) => Some(values),
            VmCompletionKind::Thrown(_) | VmCompletionKind::Failed { .. } => None,
        }
    }

    pub fn thrown_diagnostic(&self) -> Option<&VmThrownDiagnostic> {
        match self.kind.as_ref() {
            VmCompletionKind::Thrown(exception) => Some(exception.diagnostic()),
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Thrown(diagnostic),
                ..
            } => Some(diagnostic),
            VmCompletionKind::Returned(_)
            | VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Error(_),
                ..
            } => None,
        }
    }

    pub fn failure(&self) -> Option<&VmError> {
        match self.kind.as_ref() {
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Error(error),
                ..
            } => Some(error),
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Thrown(_),
                ..
            } => None,
            VmCompletionKind::Returned(_) | VmCompletionKind::Thrown(_) => None,
        }
    }

    pub const fn residual(&self) -> &VmTerminalEscrow {
        &self.residual
    }

    /// Converts a child completion into the only owner-bearing resume
    /// variants. A failed rootless diagnostic remains a rootless Failure.
    pub fn into_resume(
        self,
    ) -> Result<(ResumeOutcome, VmTerminalEscrow), (VmTerminalCause, VmTerminalEscrow)> {
        let outcome = match *self.kind {
            VmCompletionKind::Returned(values) => ResumeOutcome::Values(values),
            VmCompletionKind::Thrown(exception) => ResumeOutcome::Throw(exception),
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Error(error),
                ..
            } => ResumeOutcome::Failure(error),
            VmCompletionKind::Failed {
                image,
                diagnostic: VmFailureDiagnostic::Thrown(diagnostic),
            } => {
                let cause = VmTerminalCause {
                    kind: VmTerminalCauseKind::ThrownDiagnostic { image, diagnostic },
                };
                return Err((cause, self.residual));
            }
        };
        Ok((outcome, self.residual))
    }

    /// Moves a completion that will not be delivered into terminal cause and
    /// cleanup carriers. No owner is inferred from a diagnostic error.
    pub fn into_terminal(mut self) -> (Option<VmTerminalCause>, VmTerminalEscrow) {
        match *self.kind {
            VmCompletionKind::Returned(values) => {
                self.residual.merge(values.into_terminal_escrow());
                (None, self.residual)
            }
            VmCompletionKind::Thrown(exception) => (
                Some(VmTerminalCause::from_owned_exception(exception)),
                self.residual,
            ),
            VmCompletionKind::Failed { image, diagnostic } => {
                let cause = match diagnostic {
                    VmFailureDiagnostic::Error(error) => VmTerminalCause::from_error(image, error),
                    VmFailureDiagnostic::Thrown(diagnostic) => VmTerminalCause {
                        kind: VmTerminalCauseKind::ThrownDiagnostic { image, diagnostic },
                    },
                };
                (Some(cause), self.residual)
            }
        }
    }
}

impl fmt::Debug for VmCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("VmCompletion");
        match self.kind.as_ref() {
            VmCompletionKind::Returned(values) => {
                debug.field("returned_values", &values.len());
            }
            VmCompletionKind::Thrown(exception) => {
                debug.field("thrown", exception.diagnostic());
            }
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Error(error),
                ..
            } => {
                debug.field("failed", error);
            }
            VmCompletionKind::Failed {
                diagnostic: VmFailureDiagnostic::Thrown(diagnostic),
                ..
            } => {
                debug.field("failed_throw", diagnostic);
            }
        }
        debug
            .field("residual_roots", &self.residual.root_count())
            .finish()
    }
}

impl VmRootSource for VmCompletion {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self.kind.as_ref() {
            VmCompletionKind::Returned(values) => values.visit_roots(visitor)?,
            VmCompletionKind::Thrown(exception) => exception.visit_roots(visitor)?,
            VmCompletionKind::Failed { .. } => {}
        }
        self.residual.visit_roots(visitor)
    }
}

/// Non-cloneable terminal primary cause. An uncaught VM exception keeps its
/// diagnostic envelope and exact release authority in this same carrier, so
/// request projection may borrow the live exception before monotonically
/// releasing its payload without creating a second root-bearing
/// `VmError::Thrown` copy.
#[must_use = "a terminal VM cause may own an uncaught exception payload"]
pub struct VmTerminalCause {
    kind: VmTerminalCauseKind,
}

enum VmTerminalCauseKind {
    Diagnostic {
        image: Arc<DeploymentExecutionImage>,
        error: VmError,
    },
    Thrown(VmOwnedException),
    /// A legacy/naked thrown error is only a cloneable diagnostic alias. It
    /// can never mint or retain physical payload authority.
    ThrownDiagnostic {
        image: Arc<DeploymentExecutionImage>,
        diagnostic: VmThrownDiagnostic,
    },
}

impl VmTerminalCause {
    pub(crate) fn from_error(image: Arc<DeploymentExecutionImage>, error: VmError) -> Self {
        let kind = match error {
            // A naked thrown error no longer carries its origin plan. It must
            // never mint the origin-bound `VmOwnedException`; only the
            // producing fiber's completion seam has that authority.
            VmError::Thrown(exception) => VmTerminalCauseKind::ThrownDiagnostic {
                image,
                diagnostic: VmThrownDiagnostic::from_exception(&exception),
            },
            error => VmTerminalCauseKind::Diagnostic { image, error },
        };
        Self { kind }
    }

    pub(crate) fn from_owned_exception(exception: VmOwnedException) -> Self {
        Self {
            kind: VmTerminalCauseKind::Thrown(exception),
        }
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { image, .. } => image.owner(),
            VmTerminalCauseKind::Thrown(exception) => exception.origin_owner(),
            VmTerminalCauseKind::ThrownDiagnostic { image, .. } => image.owner(),
        }
    }

    pub fn diagnostic(&self) -> Option<&VmError> {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { error, .. } => Some(error),
            VmTerminalCauseKind::Thrown(_) | VmTerminalCauseKind::ThrownDiagnostic { .. } => None,
        }
    }

    /// Borrows the live uncaught exception for request error projection. The
    /// caller must complete any heap-backed projection before `release_all`.
    pub fn thrown(&self) -> Option<&VmThrownDiagnostic> {
        match &self.kind {
            VmTerminalCauseKind::Thrown(exception) => Some(exception.diagnostic()),
            VmTerminalCauseKind::ThrownDiagnostic { diagnostic, .. } => Some(diagnostic),
            VmTerminalCauseKind::Diagnostic { .. } => None,
        }
    }

    pub fn root_count(&self) -> usize {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { .. } => 0,
            VmTerminalCauseKind::Thrown(exception) => exception.root_count(),
            VmTerminalCauseKind::ThrownDiagnostic { .. } => 0,
        }
    }

    pub fn unresolved_count(&self) -> usize {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { .. } => 0,
            VmTerminalCauseKind::Thrown(exception) => exception.unresolved_count(),
            VmTerminalCauseKind::ThrownDiagnostic { .. } => 0,
        }
    }

    pub fn release_all(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        match &mut self.kind {
            VmTerminalCauseKind::Diagnostic { .. } => Ok(()),
            VmTerminalCauseKind::Thrown(exception) => exception.release_all(heap),
            VmTerminalCauseKind::ThrownDiagnostic { .. } => Ok(()),
        }
    }
}

impl fmt::Debug for VmTerminalCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { error, .. } => formatter
                .debug_tuple("VmTerminalCause::Diagnostic")
                .field(error)
                .finish(),
            VmTerminalCauseKind::Thrown(exception) => exception.fmt(formatter),
            VmTerminalCauseKind::ThrownDiagnostic { diagnostic, .. } => formatter
                .debug_tuple("VmTerminalCause::ThrownDiagnostic")
                .field(diagnostic)
                .finish(),
        }
    }
}

impl fmt::Display for VmTerminalCause {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VmTerminalCauseKind::Diagnostic { error, .. } => fmt::Display::fmt(error, formatter),
            VmTerminalCauseKind::Thrown(_) | VmTerminalCauseKind::ThrownDiagnostic { .. } => {
                formatter.write_str("VM threw an uncaught request-local exception")
            }
        }
    }
}

impl VmRootSource for VmTerminalCause {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match &self.kind {
            VmTerminalCauseKind::Thrown(exception) => exception.visit_roots(visitor)?,
            VmTerminalCauseKind::Diagnostic { .. }
            | VmTerminalCauseKind::ThrownDiagnostic { .. } => {}
        }
        Ok(())
    }
}

impl VmTerminalOwner {
    pub(crate) fn exact(
        value: ValueSlot,
        plan: LinkedValueTransferPlan,
        site: VmLifecycleSite,
        diagnostic_index: usize,
    ) -> Self {
        Self {
            value,
            release: VmTerminalReleaseAuthority::Exact(plan),
            site,
            diagnostic_index,
        }
    }

    pub(crate) fn damaged_retained(
        value: ValueSlot,
        site: VmLifecycleSite,
        diagnostic_index: usize,
    ) -> Self {
        Self {
            value,
            release: VmTerminalReleaseAuthority::DamagedRetained,
            site,
            diagnostic_index,
        }
    }
}

impl VmTerminalEscrow {
    pub(crate) fn new(
        image: Arc<DeploymentExecutionImage>,
        mut owners: Vec<VmTerminalOwner>,
    ) -> Self {
        // Damaged retained owners cannot be released without guessing. Keep
        // them at the front so the exact suffix is still drained
        // monotonically before `release_all` reports the retained remainder.
        // Stable ordering preserves the deterministic extraction order inside
        // each authority class.
        owners.sort_by_key(|owner| matches!(&owner.release, VmTerminalReleaseAuthority::Exact(_)));
        Self {
            images: vec![image],
            owners,
        }
    }

    pub(crate) fn empty(image: Arc<DeploymentExecutionImage>) -> Self {
        Self::new(image, Vec::new())
    }

    pub fn owner(&self) -> &DeploymentOwnerIdentity {
        self.images[0].owner()
    }

    pub fn root_count(&self) -> usize {
        self.owners.len()
    }

    pub fn unresolved_count(&self) -> usize {
        self.owners
            .iter()
            .filter(|owner| matches!(&owner.release, VmTerminalReleaseAuthority::DamagedRetained))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    /// Consumes a root result that cannot be delivered (for example because a
    /// scheduler stream-finalization port failed after completion). The
    /// primary scheduler error remains separate; this carrier owns only the
    /// abandoned result roots.
    pub fn from_abandoned_result(
        image: Arc<DeploymentExecutionImage>,
        result: Result<VmOwnedValues, VmError>,
    ) -> (Option<VmTerminalCause>, Self) {
        match result {
            Ok(values) => (None, values.into_terminal_escrow()),
            Err(error) => {
                let escrow = Self::empty(Arc::clone(&image));
                (Some(VmTerminalCause::from_error(image, error)), escrow)
            }
        }
    }

    /// Consumes a resume outcome rejected before a VM unit could accept it.
    /// The outcome itself remains the sole ownership authority until this
    /// conversion; no scheduler error is inspected to infer cleanup.
    pub fn from_abandoned_resume(
        image: Arc<DeploymentExecutionImage>,
        outcome: ResumeOutcome,
    ) -> Self {
        match outcome {
            ResumeOutcome::Values(values) => values.into_terminal_escrow(),
            ResumeOutcome::Throw(exception) => exception.into_terminal_escrow(),
            ResumeOutcome::Empty
            | ResumeOutcome::StreamEnd
            | ResumeOutcome::Failure(_)
            | ResumeOutcome::InternalTerminal(_) => Self::empty(image),
        }
    }

    pub(crate) fn from_slots(
        image: Arc<DeploymentExecutionImage>,
        values: Vec<ValueSlot>,
        plans: Vec<Option<LinkedValueTransferPlan>>,
        site: VmLifecycleSite,
    ) -> Self {
        let mut owners = Vec::with_capacity(values.len());
        for (diagnostic_index, value) in values.into_iter().enumerate() {
            if is_discardable_root(&value) {
                continue;
            }
            let plan = plans.get(diagnostic_index).cloned().flatten();
            owners.push(match plan {
                Some(plan) => Self::exact_owner(value, plan, site, diagnostic_index),
                None => VmTerminalOwner::damaged_retained(value, site, diagnostic_index),
            });
        }
        Self::new(image, owners)
    }

    // The linear R2 scheduler join consumes this seam when it collects every
    // active/blocked fiber carrier. V1 keeps it crate-sealed until that join.
    #[allow(dead_code)]
    pub(crate) fn merge(&mut self, mut other: Self) {
        for image in other.images.drain(..) {
            if !self
                .images
                .iter()
                .any(|current| Arc::ptr_eq(current, &image))
            {
                self.images.push(image);
            }
        }
        self.owners.append(&mut other.owners);
        self.owners
            .sort_by_key(|owner| matches!(&owner.release, VmTerminalReleaseAuthority::Exact(_)));
    }

    fn exact_owner(
        value: ValueSlot,
        plan: LinkedValueTransferPlan,
        site: VmLifecycleSite,
        diagnostic_index: usize,
    ) -> VmTerminalOwner {
        VmTerminalOwner::exact(value, plan, site, diagnostic_index)
    }

    /// Releases a fixed suffix monotonically through exact linked plans.
    /// Successful owners are removed immediately. Any heap or plan failure
    /// returns without removing the failing owner, so the caller can retain
    /// this carrier and retry without re-releasing a completed suffix.
    pub fn release_all(&mut self, heap: &mut dyn VmHeap) -> Result<(), VmError> {
        let mut executor = LifecycleExecutor::new(heap);
        while let Some(owner) = self.owners.last() {
            match &owner.release {
                VmTerminalReleaseAuthority::Exact(plan) => {
                    executor.release(&owner.value, plan).map_err(|error| {
                        error.into_vm_error(
                            owner.site.function,
                            owner.site.instruction,
                            owner.site.opcode,
                        )
                    })?;
                }
                VmTerminalReleaseAuthority::DamagedRetained => {
                    return Err(VmError::TerminalRootLifecycleUnavailable {
                        index: owner.diagnostic_index,
                        kind: owner.value.kind(),
                    });
                }
            }
            self.owners.pop();
        }
        Ok(())
    }
}

impl VmRootSource for VmTerminalEscrow {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for owner in &self.owners {
            visitor.visit_root(&owner.value)?;
        }
        Ok(())
    }
}

/// Terminal result of attempting to bind one scheduler resume token.
///
/// A validation rejection returns both the exact token and its input without
/// consuming either. Only a matching, accepted rootless terminal outcome uses
/// `Terminal`; owned Values/Throw inputs can never be hidden in that variant.
#[must_use = "a failed resume may return an outcome that still owns VM roots"]
pub struct VmResumeFailure {
    kind: VmResumeFailureKind,
}

enum VmResumeFailureKind {
    Terminal(VmError),
    Rejected {
        error: VmError,
        resume: VmResumeToken,
        outcome: ResumeOutcome,
    },
}

impl fmt::Debug for VmResumeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            VmResumeFailureKind::Terminal(error) => formatter
                .debug_tuple("VmResumeFailure::Terminal")
                .field(error)
                .finish(),
            VmResumeFailureKind::Rejected { error, .. } => formatter
                .debug_struct("VmResumeFailure::Rejected")
                .field("error", error)
                .field("resume", &"<owned resume token>")
                .field("outcome", &"<owned resume outcome>")
                .finish(),
        }
    }
}

impl VmResumeFailure {
    pub(crate) fn terminal(error: VmError) -> Self {
        debug_assert!(
            !matches!(&error, VmError::Thrown(_)),
            "a root-bearing thrown outcome must be rejected unchanged"
        );
        Self {
            kind: VmResumeFailureKind::Terminal(error),
        }
    }

    pub(crate) fn rejected(error: VmError, resume: VmResumeToken, outcome: ResumeOutcome) -> Self {
        debug_assert!(
            !matches!(&error, VmError::Thrown(_)),
            "resume rejection diagnostics cannot own the returned outcome"
        );
        Self {
            kind: VmResumeFailureKind::Rejected {
                error,
                resume,
                outcome,
            },
        }
    }

    pub const fn error(&self) -> &VmError {
        match &self.kind {
            VmResumeFailureKind::Terminal(error) | VmResumeFailureKind::Rejected { error, .. } => {
                error
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn rejected_parts(&self) -> Option<(&VmResumeToken, &ResumeOutcome)> {
        match &self.kind {
            VmResumeFailureKind::Rejected {
                resume, outcome, ..
            } => Some((resume, outcome)),
            VmResumeFailureKind::Terminal(_) => None,
        }
    }

    /// Consumes the sealed failure. `None` is a rootless accepted terminal
    /// outcome; `Some` returns the exact unique token and unconsumed input.
    pub fn into_parts(self) -> (VmError, Option<(VmResumeToken, ResumeOutcome)>) {
        match self.kind {
            VmResumeFailureKind::Terminal(error) => (error, None),
            VmResumeFailureKind::Rejected {
                error,
                resume,
                outcome,
            } => (error, Some((resume, outcome))),
        }
    }

    /// Converts a rejected owned input into terminal cleanup. The receiving
    /// fiber supplies the image pin only for envelope inputs; `Values` retains
    /// and consumes its own origin image pin.
    pub fn into_terminal_escrow(
        self,
        image: Arc<DeploymentExecutionImage>,
    ) -> (VmTerminalCause, VmTerminalEscrow) {
        match self.kind {
            VmResumeFailureKind::Terminal(error) => {
                let escrow = VmTerminalEscrow::empty(Arc::clone(&image));
                (VmTerminalCause::from_error(image, error), escrow)
            }
            VmResumeFailureKind::Rejected { error, outcome, .. } => {
                let escrow = VmTerminalEscrow::from_abandoned_resume(Arc::clone(&image), outcome);
                (VmTerminalCause::from_error(image, error), escrow)
            }
        }
    }
}

impl VmRootSource for VmResumeFailure {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match &self.kind {
            VmResumeFailureKind::Terminal(error) => visit_vm_error(error, visitor),
            VmResumeFailureKind::Rejected { error, outcome, .. } => {
                visit_vm_error(error, visitor)?;
                outcome.visit_roots(visitor)
            }
        }
    }
}

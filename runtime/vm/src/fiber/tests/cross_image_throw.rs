use std::sync::Arc;

use skiff_artifact_model::{InstructionSourceSite, Opcode, SourcePosition, SourceSpanRef};
use skiff_runtime_model::{
    service_error::{ExceptionStackFrame, OpaqueServiceError, RequestException},
    vm_heap::{VmHandleInvalidReason, VmHeap, VmHeapError},
    vm_value::{ValueFlags, ValueKind, ValueSlot, VmHandle},
};

use super::*;
use crate::{
    control::VmResumeAuthority, ChildTarget, VmCompletion, VmLifecycleSite, VmOwnedException,
    VmTerminalEscrow, VmThrownDiagnostic,
};

#[derive(Default)]
struct MintHeap {
    live: BTreeSet<u64>,
    snapshot_releases: usize,
    resource_releases: usize,
}

impl MintHeap {
    fn with_live(handle: u64) -> Self {
        let mut heap = Self::default();
        heap.live.insert(handle);
        heap
    }
}

impl VmHeap for MintHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        let Some(handle) = value.as_request_heap_ref() else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        if self.live.contains(&handle.get()) {
            Ok(())
        } else {
            Err(VmHeapError::InvalidHandle {
                kind: ValueKind::RequestHeapRef,
                handle,
                reason: VmHandleInvalidReason::WrongDomain,
            })
        }
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        Ok(*source)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.validate_live(owner)?;
        self.snapshot_releases += 1;
        Ok(())
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.resource_releases += 1;
        Ok(())
    }
}

fn caller_facts() -> (
    Arc<DeploymentExecutionImage>,
    FunctionIndex,
    InstructionIndex,
    TypeIndex,
    LinkedValueTransferPlan,
) {
    let caller =
        ObservationFixture::build("test.skiff/fiber-cross-image", OWNED_THROW_RESUME_SOURCE);
    let function_index = FunctionIndex::new(caller.root_function_index());
    let function = &caller.image.functions()[function_index.get() as usize];
    let region = function.exception_regions()[0].clone();
    let LinkedCatchMatcher::Type(leaf) = region.catch_matchers()[0] else {
        panic!("cross-image fixture has an exact nominal Leaf catch")
    };
    let plan = caller
        .image
        .type_plan(leaf)
        .cloned()
        .expect("caller image resolves the nominal Leaf plan");
    (
        Arc::clone(&caller.image),
        function_index,
        region.start(),
        leaf,
        plan,
    )
}

fn provider_diagnostic() -> VmThrownDiagnostic {
    let provider =
        ObservationFixture::build("test.skiff/fiber-cross-image", OWNED_THROW_RESUME_SOURCE);
    let mut heap = ResumeHeap { next: 100 };
    let (outcome, _) = origin_owned_throw(&provider, &mut heap);
    let ResumeOutcome::Throw(exception) = outcome else {
        panic!("provider fixture seals its ordinary throw")
    };
    exception.diagnostic().clone()
}

fn caller_token(
    image: Arc<DeploymentExecutionImage>,
    function: FunctionIndex,
    instruction: InstructionIndex,
) -> crate::VmResumeToken {
    crate::VmResumeToken::new(
        image,
        1,
        function,
        instruction,
        instruction,
        None,
        ResumeSiteIndex::new(0),
        0,
        0,
        VmResumeAuthority::Child(ChildTarget::StreamNext),
        None,
    )
}

fn mint_site(function: FunctionIndex, instruction: InstructionIndex) -> VmLifecycleSite {
    VmLifecycleSite {
        function,
        instruction,
        opcode: Opcode::CallService,
    }
}

fn caller_payload(leaf: TypeIndex) -> ValueSlot {
    ValueSlot::request_heap_ref(
        VmHandle::new(1000),
        compact_tag(leaf.get()),
        ValueFlags::new(0),
    )
}

#[test]
fn cross_image_throw_mint_binds_caller_image_heap_and_exact_plan() {
    let (caller_image, function, instruction, leaf, plan) = caller_facts();
    let token = caller_token(Arc::clone(&caller_image), function, instruction);
    let payload = caller_payload(leaf);
    let diagnostic = provider_diagnostic();
    let mut heap = MintHeap::with_live(1000);

    let exception = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan.clone(),
        mint_site(function, instruction),
    )
    .expect("checked caller throw mint accepts the exact caller facts");

    assert!(Arc::ptr_eq(exception.origin_image(), &caller_image));
    assert!(exception.exception().vm_local_slot() == Some(payload));
    assert_eq!(exception.diagnostic(), &diagnostic);
    assert_eq!(exception.root_count(), 1);
    assert_eq!(exception.unresolved_count(), 0);
    assert!(exception.is_bound_to(token.binding()));

    let mut exception = exception;
    exception
        .release_all(&mut heap)
        .expect("caller-heap exception releases its exact payload");
    assert_eq!(heap.snapshot_releases, 1);
    assert_eq!(exception.root_count(), 0);
}

#[test]
fn cross_image_throw_terminal_escrow_uses_caller_plan_exactly_once() {
    let (caller_image, function, instruction, leaf, plan) = caller_facts();
    let token = caller_token(Arc::clone(&caller_image), function, instruction);
    let payload = caller_payload(leaf);
    let diagnostic = provider_diagnostic();
    let mut heap = MintHeap::with_live(1000);

    let exception = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan,
        mint_site(function, instruction),
    )
    .expect("checked caller throw mint accepts the exact caller facts");
    let mut escrow = exception.into_terminal_escrow();
    assert_eq!(escrow.root_count(), 1);
    assert_eq!(escrow.unresolved_count(), 0);
    escrow
        .release_all(&mut heap)
        .expect("terminal escrow releases through the captured caller plan");
    assert_eq!(heap.snapshot_releases, 1);
}

#[test]
fn cross_image_throw_mint_rejects_wrong_caller_image_and_token_site() {
    let (caller_image, function, instruction, leaf, plan) = caller_facts();
    let other =
        ObservationFixture::build("test.skiff/fiber-cross-image", OWNED_THROW_RESUME_SOURCE);
    let token = caller_token(Arc::clone(&caller_image), function, instruction);
    let payload = caller_payload(leaf);
    let diagnostic = provider_diagnostic();
    let heap = MintHeap::with_live(1000);

    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&other.image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan.clone(),
        mint_site(function, instruction),
    )
    .expect_err("a caller image that is not the token image must be rejected");
    assert_eq!(rejected.error(), &VmError::ResumeTokenMismatch);
    assert!(Arc::ptr_eq(rejected.image(), &caller_image));
    assert!(rejected.payload() == Some(payload));

    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan,
        VmLifecycleSite {
            function,
            instruction: InstructionIndex::new(instruction.get() + 1),
            opcode: Opcode::CallService,
        },
    )
    .expect_err("a mint site that disagrees with the token must be rejected");
    assert_eq!(rejected.error(), &VmError::ResumeTokenMismatch);
    assert!(rejected.payload() == Some(payload));
}

#[test]
fn cross_image_throw_mint_rejects_missing_payload_and_missing_identity() {
    let (caller_image, function, instruction, leaf, plan) = caller_facts();
    let token = caller_token(Arc::clone(&caller_image), function, instruction);
    let diagnostic = provider_diagnostic();
    let heap = MintHeap::with_live(1000);

    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        None,
        &diagnostic,
        plan.clone(),
        mint_site(function, instruction),
    )
    .expect_err("a missing caller payload must be rejected");
    assert!(matches!(
        rejected.error(),
        VmError::ResumeThrowEnvelopeUnavailable { .. }
    ));
    assert!(rejected.payload() == None);

    let source = InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 7,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    };
    let error = OpaqueServiceError::internal_error("boom", "trace", "error")
        .expect("internal service error is canonical");
    let imported = RequestException::imported(
        error,
        None,
        source.clone(),
        vec![ExceptionStackFrame::Local { site: source }],
    )
    .expect("imported rootless diagnostic is valid");
    let completion = VmCompletion::failed(
        Arc::clone(&caller_image),
        VmError::Thrown(Arc::new(imported)),
        VmTerminalEscrow::empty(Arc::clone(&caller_image)),
    );
    let diagnostic = completion
        .thrown_diagnostic()
        .expect("failed thrown completion exposes rootless diagnostic")
        .clone();
    assert_eq!(diagnostic.identity(), None);

    let payload = caller_payload(leaf);
    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan,
        mint_site(function, instruction),
    )
    .expect_err("a diagnostic without catch identity must be rejected");
    assert!(matches!(
        rejected.error(),
        VmError::ResumeThrowEnvelopeUnavailable { .. }
    ));
    assert!(rejected.payload() == Some(payload));
}

#[test]
fn cross_image_throw_mint_rejects_foreign_heap_and_damaged_plan() {
    let (caller_image, function, instruction, leaf, plan) = caller_facts();
    let token = caller_token(Arc::clone(&caller_image), function, instruction);
    let diagnostic = provider_diagnostic();
    let payload = caller_payload(leaf);
    let mut heap = MintHeap::default();

    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        plan.clone(),
        mint_site(function, instruction),
    )
    .expect_err("a raw payload outside the caller heap must be rejected");
    assert!(matches!(
        rejected.error(),
        VmError::Heap(VmHeapError::InvalidHandle { .. })
    ));
    assert!(rejected.payload() == Some(payload));

    heap.live.insert(1000);
    let damaged = LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::Trivial,
    };
    assert_ne!(
        plan, damaged,
        "the nominal Leaf must not already be trivial"
    );
    let rejected = VmOwnedException::try_from_caller_resume(
        Arc::clone(&caller_image),
        &token,
        &heap,
        Some(payload),
        &diagnostic,
        damaged,
        mint_site(function, instruction),
    )
    .expect_err("a plan that is not the exact image plan must be rejected");
    assert!(matches!(
        rejected.error(),
        VmError::FullValueLifecyclePlanUnavailable { .. }
    ));
    assert!(rejected.payload() == Some(payload));
    drop(rejected);
}

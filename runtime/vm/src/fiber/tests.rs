use std::{
    collections::BTreeMap,
    num::{NonZeroU32, NonZeroUsize},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex,
    },
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractOperationId, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentRevision, Opcode, PackageArtifact, ServiceContract,
    ServiceDeployment, TypeRefIr, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_compiler::{
    compile_package, CompilerPlatformSources, ManifestOwner, ManifestProvenance,
    PackageCompileInput, PackageSourceInput, PublicationManifest, PublicationSourceGraph,
    SourceTree, SourceTreeFile,
};
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_linked_bytecode::{
    FrameSlotIndex, InstructionBoundaryIndex, InstructionIndex, LinkedCatchMatcher,
    LinkedExceptionRegion, TypeIndex,
};
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionEntry, DeploymentExecutionImage,
    DeploymentExecutionLimits, LinkLimits,
};
use skiff_runtime_loader::{DeploymentBytecodeContentResolver, DeploymentBytecodeLoader};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionCorrelation, BytecodeExecutionEvent, BytecodeExecutionEventSink,
    BytecodeExecutionObservation, BytecodeExecutionObserver, VmFunctionFrameEntered,
    VmFunctionReturned, VmLocalCallDispatched, VmObservedFrameRole,
};
use skiff_runtime_model::service_error::{CatchIdentity, NominalTypeIdentity};
use skiff_runtime_model::vm_heap::{VmHeap, VmHeapError};
use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

use super::{
    catch_matches, comparable_equality, comparable_equality_with_string_resolver,
    find_exception_region, linked_type_catch_identity, nominal_tag_index, opcode_supported,
    runtime_leaf_catch_identity, DispatchOutcome, Vm, VmFiber,
};
use crate::{VmError, VmLimits};

type VmStartFn = fn(
    DeploymentExecutionEntry,
    Box<[ValueSlot]>,
    VmLimits,
    BytecodeExecutionObserver,
) -> Result<VmFiber, VmError>;

#[test]
fn production_start_signature_requires_the_concrete_pinned_entry() {
    let entry: VmStartFn = Vm::start;

    let _ = entry;
}

#[test]
fn fiber_keeps_frame_and_values_out_of_the_managed_heap() {
    fn assert_root_source<T: skiff_runtime_model::vm_root::VmRootSource>() {}
    assert_root_source::<VmFiber>();
}

#[test]
fn opcode_dispatch_has_a_heap_port_but_no_budget_port() {
    let dispatch: fn(&mut VmFiber, &mut dyn VmHeap) -> Result<DispatchOutcome, VmError> =
        VmFiber::dispatch_one;

    let _ = dispatch;
}

#[test]
fn value_control_and_scalar_opcodes_are_supported() {
    for opcode in [
        Opcode::Const,
        Opcode::CopySlot,
        Opcode::StoreSlot,
        Opcode::Drop,
        Opcode::Dup,
        Opcode::LoadSlot,
        Opcode::TakeSlot,
        Opcode::Pop,
        Opcode::MoveSlot,
        Opcode::Jump,
        Opcode::JumpIfTrue,
        Opcode::JumpIfFalse,
        Opcode::BudgetCheckpoint,
        Opcode::CallLocal,
        Opcode::TailCallLocal,
        Opcode::Return,
        Opcode::Not,
        Opcode::Negate,
        Opcode::Add,
        Opcode::Subtract,
        Opcode::Multiply,
        Opcode::Divide,
        Opcode::Equal,
        Opcode::NotEqual,
        Opcode::LessThan,
        Opcode::LessOrEqual,
        Opcode::GreaterThan,
        Opcode::GreaterOrEqual,
    ] {
        assert!(opcode_supported(opcode), "{opcode:?} should be supported");
    }
}

#[test]
fn unsupported_opcodes_remain_fail_closed() {
    let opcode = Opcode::CallLocalInOut;
    assert!(
        !opcode_supported(opcode),
        "{opcode:?} should be unsupported"
    );
}

#[test]
fn production_opcode_families_are_dispatched() {
    for opcode in [
        Opcode::SwitchTag,
        Opcode::Trap,
        Opcode::CallService,
        Opcode::CallActor,
        Opcode::CallInterface,
        Opcode::InvokeHost,
        Opcode::InvokeIntrinsic,
        Opcode::MakeCallback,
        Opcode::InvokeCallback,
        Opcode::NewRecord,
        Opcode::GetDenseField,
        Opcode::SetWritablePath,
        Opcode::RepresentationWrap,
        Opcode::NewArrayBuilder,
        Opcode::ArrayBuilderPush,
        Opcode::FreezeArray,
        Opcode::ArrayGet,
        Opcode::ArrayPushOwned,
        Opcode::ArrayLen,
        Opcode::NewMapBuilder,
        Opcode::MapBuilderPut,
        Opcode::FreezeMap,
        Opcode::MapGet,
        Opcode::MapPutOwned,
        Opcode::MapLen,
        Opcode::MapEntryAt,
        Opcode::StreamNext,
        Opcode::EmitStream,
        Opcode::Throw,
        Opcode::Rethrow,
        Opcode::EnterRegion,
        Opcode::LeaveRegion,
        Opcode::InterfaceBoxLocal,
        Opcode::InterfaceBoxRemote,
    ] {
        assert!(opcode_supported(opcode), "{opcode:?} should be supported");
    }
}

#[test]
fn exception_region_selection_uses_the_innermost_matching_handler() {
    let outer = exception_region(0, 8, 20, TypeIndex::new(1), FrameSlotIndex::new(0));
    let inner = exception_region(2, 5, 30, TypeIndex::new(2), FrameSlotIndex::new(1));
    let regions = [outer.clone(), inner];

    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(3), Some(TypeIndex::new(2)))
            .map(|region| region.handler()),
        Some(InstructionIndex::new(30))
    );
    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(6), Some(TypeIndex::new(1)))
            .map(|region| region.handler()),
        Some(InstructionIndex::new(20))
    );
    assert_eq!(
        find_exception_region(&regions, InstructionIndex::new(3), Some(TypeIndex::new(9))),
        None
    );
}

#[test]
fn catch_all_and_exact_type_matchers_are_closed() {
    assert!(catch_matches(&LinkedCatchMatcher::CatchAll, None));
    assert!(!catch_matches(
        &LinkedCatchMatcher::Type(TypeIndex::new(3)),
        Some(TypeIndex::new(4))
    ));
    assert!(catch_matches(
        &LinkedCatchMatcher::Type(TypeIndex::new(3)),
        Some(TypeIndex::new(3))
    ));
}

#[test]
fn nominal_switch_tag_comes_from_reference_metadata_only() {
    assert_eq!(nominal_tag_index(&ValueSlot::number(1.0)), 0);
    assert_eq!(
        nominal_tag_index(&ValueSlot::request_heap_ref(
            skiff_runtime_model::vm_value::VmHandle::new(1),
            skiff_runtime_model::vm_value::CompactTypeTag::new(42),
            skiff_runtime_model::vm_value::ValueFlags::new(0),
        )),
        42
    );
}

fn exception_region(
    start: u32,
    end: u32,
    handler: u32,
    catch_type: TypeIndex,
    catch_slot: FrameSlotIndex,
) -> LinkedExceptionRegion {
    LinkedExceptionRegion::new(
        InstructionIndex::new(start),
        InstructionBoundaryIndex::new(end),
        InstructionIndex::new(handler),
        0,
        Box::new([LinkedCatchMatcher::Type(catch_type)]),
        catch_slot,
        catch_type,
        0,
    )
}

#[test]
fn comparable_equality_matches_same_kind_and_numeric_equality() {
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::integer(3)),
        Some(true)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::number(3.0)),
        Some(true)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::integer(3), &ValueSlot::number(4.0)),
        Some(false)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::bool(false), &ValueSlot::bool(true)),
        Some(false)
    );
    assert_eq!(
        comparable_equality(&ValueSlot::null(), &ValueSlot::null()),
        Some(true)
    );
}

#[test]
fn comparable_equality_resolves_const_and_request_heap_strings() {
    let const_left =
        ValueSlot::const_ref(VmHandle::new(1), CompactTypeTag::new(0), ValueFlags::new(0));
    let const_right =
        ValueSlot::const_ref(VmHandle::new(2), CompactTypeTag::new(0), ValueFlags::new(0));
    let heap_same =
        ValueSlot::request_heap_ref(VmHandle::new(3), CompactTypeTag::new(0), ValueFlags::new(0));
    let heap_different =
        ValueSlot::request_heap_ref(VmHandle::new(4), CompactTypeTag::new(0), ValueFlags::new(0));
    let resolve_string = |value: &ValueSlot| match value.as_handle()?.get() {
        1 | 2 | 3 => Some("same".to_string()),
        4 => Some("different".to_string()),
        _ => None,
    };

    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &const_right, resolve_string),
        Some(true)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &heap_same, resolve_string),
        Some(true)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &heap_different, resolve_string),
        Some(false)
    );
    assert_eq!(
        comparable_equality_with_string_resolver(&heap_same, &heap_same, resolve_string),
        Some(true)
    );

    let unresolved =
        ValueSlot::const_ref(VmHandle::new(9), CompactTypeTag::new(0), ValueFlags::new(0));
    assert_eq!(
        comparable_equality_with_string_resolver(&const_left, &unresolved, resolve_string),
        None
    );
}

// ---------------------------------------------------------------------------
// O1 Phase 1 observation-window tests. The fixtures below compile the same
// scalar local-call sources accepted by the Phase 1 containment surface and
// drive the real production fiber/observer seam. No existing budget or DEC1-B
// test is modified.
// ---------------------------------------------------------------------------

const LOCAL_CALL_SOURCE: &str = "function helper(value: number) -> number { return value + 1 }\n\
     function run(value: number) -> number { final result = helper(value)\n return result }\n";
const REPEATED_CALL_SOURCE: &str =
    "function helper(value: number) -> number { return value + 1 }\n\
     function run(value: number) -> number {\n\
       final first = helper(value)\n final second = helper(first)\n return second\n }\n";
const DEEP_CALL_SOURCE: &str = "function innermost(value: number) -> number { return value + 1 }\n\
     function middle(value: number) -> number {\n\
       final result = innermost(value)\n return result\n }\n\
     function run(value: number) -> number {\n\
       final result = middle(value)\n return result\n }\n";
const ROOT_ONLY_SOURCE: &str = "function run() -> number { return 1 }\n";

struct ObservationFixture {
    image: Arc<DeploymentExecutionImage>,
    operation: ContractOperationId,
}

impl ObservationFixture {
    fn build(package_id: &str, source: &str) -> Self {
        let (package, bytecode) = compile_fixture_package(package_id, source);
        let (contract, operation) = service_contract(package_id);
        let (deployment, deployment_reference) =
            service_deployment(&package, &contract, operation.clone());
        let resolver = TestResolver {
            deployment,
            contract,
            package,
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&deployment_reference)
            .unwrap();
        let image = Arc::new(
            link_deployment_execution_image(
                hydrated,
                &DeploymentExecutionLimits::new(link_limits(), verification_limits()),
            )
            .unwrap(),
        );
        Self { image, operation }
    }

    fn root_function_index(&self) -> u32 {
        self.image
            .operation_entry(&self.operation)
            .unwrap()
            .function()
            .get()
    }

    fn slot_count(&self, function_index: u32) -> usize {
        self.image.functions()[function_index as usize]
            .frame()
            .slot_types()
            .len()
    }

    fn start(
        &self,
        limits: VmLimits,
        observer: BytecodeExecutionObserver,
        arguments: Box<[ValueSlot]>,
    ) -> VmFiber {
        let entry = self.image.operation_entry(&self.operation).unwrap();
        Vm::start(entry, arguments, limits, observer).unwrap()
    }
}

#[derive(Default)]
struct RecordingSink(StdMutex<Vec<BytecodeExecutionObservation>>);

impl BytecodeExecutionEventSink for RecordingSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.0.lock().unwrap().push(observation);
    }
}

fn observation_correlation() -> BytecodeExecutionCorrelation {
    BytecodeExecutionCorrelation {
        router_session_id: "fiber-observation-session".to_string(),
        request_id: "fiber-observation-request".to_string(),
    }
}

struct TestHeap;

impl VmHeap for TestHeap {
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

fn vm_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(128).unwrap(),
        NonZeroUsize::new(4096).unwrap(),
        NonZeroU32::new(1024).unwrap(),
    )
}

fn single_frame_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(1).unwrap(),
        NonZeroUsize::new(4096).unwrap(),
        NonZeroU32::new(1024).unwrap(),
    )
}

fn drive_to_completion(
    fiber: &mut VmFiber,
    track: &mut dyn FnMut(&VmFiber),
) -> Result<(), VmError> {
    let mut heap = TestHeap;
    for _ in 0..10_000 {
        let outcome = fiber.dispatch_one(&mut heap)?;
        track(fiber);
        match outcome {
            DispatchOutcome::Continue => {}
            DispatchOutcome::Complete(_) => return Ok(()),
            DispatchOutcome::Handoff(_) => panic!("observation fixture must not hand off"),
            DispatchOutcome::Throw(_) => panic!("observation fixture must not throw"),
        }
    }
    panic!("observation fixture did not terminate within the step cap");
}

fn drive_until_error(fiber: &mut VmFiber) -> VmError {
    let mut heap = TestHeap;
    for _ in 0..10_000 {
        match fiber.dispatch_one(&mut heap) {
            Err(error) => return error,
            Ok(DispatchOutcome::Complete(_)) => {
                panic!("observation fixture completed instead of failing")
            }
            Ok(DispatchOutcome::Continue) => {}
            Ok(DispatchOutcome::Handoff(_)) => panic!("observation fixture must not hand off"),
            Ok(DispatchOutcome::Throw(_)) => panic!("observation fixture must not throw"),
        }
    }
    panic!("observation fixture never failed");
}

fn event_kind(event: &BytecodeExecutionEvent) -> &'static str {
    match event {
        BytecodeExecutionEvent::DeploymentImageSelected(_) => "DeploymentImageSelected",
        BytecodeExecutionEvent::RouteEntryPinned(_) => "RouteEntryPinned",
        BytecodeExecutionEvent::VmFirstInstructionDispatched(_) => "VmFirstInstructionDispatched",
        BytecodeExecutionEvent::VmFunctionFrameEntered(_) => "VmFunctionFrameEntered",
        BytecodeExecutionEvent::VmLocalCallDispatched(_) => "VmLocalCallDispatched",
        BytecodeExecutionEvent::VmFunctionReturned(_) => "VmFunctionReturned",
        BytecodeExecutionEvent::VmBudgetAccounted(_) => "VmBudgetAccounted",
        BytecodeExecutionEvent::RequestTerminalClaimed(_) => "RequestTerminalClaimed",
        BytecodeExecutionEvent::RequestCleanupComplete(_) => "RequestCleanupComplete",
    }
}

fn new_events(records: &[BytecodeExecutionObservation]) -> Vec<&BytecodeExecutionEvent> {
    records
        .iter()
        .map(|record| &record.event)
        .filter(|event| {
            matches!(
                event,
                BytecodeExecutionEvent::VmFunctionFrameEntered(_)
                    | BytecodeExecutionEvent::VmLocalCallDispatched(_)
                    | BytecodeExecutionEvent::VmFunctionReturned(_)
            )
        })
        .collect()
}

#[test]
fn root_and_first_root_local_callee_are_selected_and_paired() {
    let fixture =
        ObservationFixture::build("example.com/fiber-observation-scalar", LOCAL_CALL_SOURCE);
    let root = fixture.root_function_index();
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    drive_to_completion(&mut fiber, &mut |_| {}).unwrap();

    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    assert_eq!(events.len(), 5, "exactly the bounded five VM events");

    let BytecodeExecutionEvent::VmFunctionFrameEntered(root_entry) = events[0] else {
        panic!("first new event must be the root frame entry");
    };
    assert_eq!(root_entry.role, VmObservedFrameRole::Root);
    assert_eq!(root_entry.function_index, root);
    assert_eq!(root_entry.frame_depth, 1);
    assert_eq!(root_entry.slot_count as usize, fixture.slot_count(root));

    let BytecodeExecutionEvent::VmLocalCallDispatched(dispatched) = events[1] else {
        panic!("second new event must be the selected local call");
    };
    assert_eq!(dispatched.caller_function_index, root);
    assert_eq!(dispatched.caller_frame_depth, 1);
    assert_eq!(dispatched.callee_frame_depth, 2);
    let callee = dispatched.callee_function_index;
    assert_ne!(callee, root);
    assert_eq!(fixture.image.functions().len(), 2);

    let BytecodeExecutionEvent::VmFunctionFrameEntered(callee_entry) = events[2] else {
        panic!("third new event must be the selected callee frame entry");
    };
    assert_eq!(callee_entry.role, VmObservedFrameRole::FirstRootLocalCallee);
    assert_eq!(callee_entry.function_index, callee);
    assert_eq!(callee_entry.frame_depth, 2);
    assert_eq!(callee_entry.slot_count as usize, fixture.slot_count(callee));

    let BytecodeExecutionEvent::VmFunctionReturned(callee_return) = events[3] else {
        panic!("fourth new event must be the selected callee return");
    };
    assert_eq!(
        callee_return.role,
        VmObservedFrameRole::FirstRootLocalCallee
    );
    assert_eq!(callee_return.function_index, callee);
    assert_eq!(callee_return.caller_function_index, Some(root));
    assert_eq!(callee_return.remaining_frame_depth, 1);

    let BytecodeExecutionEvent::VmFunctionReturned(root_return) = events[4] else {
        panic!("fifth new event must be the root return");
    };
    assert_eq!(root_return.role, VmObservedFrameRole::Root);
    assert_eq!(root_return.function_index, root);
    assert_eq!(root_return.caller_function_index, None);
    assert_eq!(root_return.remaining_frame_depth, 0);

    assert_eq!(
        records
            .iter()
            .map(|record| event_kind(&record.event))
            .collect::<Vec<_>>(),
        [
            "VmFunctionFrameEntered",
            "VmFirstInstructionDispatched",
            "VmLocalCallDispatched",
            "VmFunctionFrameEntered",
            "VmFunctionReturned",
            "VmFunctionReturned",
        ]
    );
}

#[test]
fn every_mint_reports_the_state_transition_that_already_completed() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-observation-transition",
        LOCAL_CALL_SOURCE,
    );
    let root = fixture.root_function_index();
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));

    {
        let records = sink.0.lock().unwrap();
        assert!(matches!(
            records[0].event,
            BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
                frame_depth: 1,
                ..
            })
        ));
        assert_eq!(fiber.frames.len(), 1);
    }

    let mut saw_call_return = false;
    let mut saw_root_return = false;
    let mut step = |fiber: &VmFiber| {
        let records = sink.0.lock().unwrap();
        if !saw_call_return
            && records.iter().any(|record| {
                matches!(
                    record.event,
                    BytecodeExecutionEvent::VmLocalCallDispatched(_)
                )
            })
        {
            assert_eq!(fiber.frames.len(), 2, "call mint ran after the child push");
            saw_call_return = true;
        }
        if !saw_root_return
            && records.iter().any(|record| {
                matches!(
                    record.event,
                    BytecodeExecutionEvent::VmFunctionReturned(VmFunctionReturned {
                        role: VmObservedFrameRole::Root,
                        ..
                    })
                )
            })
        {
            assert_eq!(
                fiber.frames.len(),
                0,
                "root return mint ran after the clear"
            );
            assert_eq!(fiber.state(), super::VmFiberState::Terminal);
            saw_root_return = true;
        }
    };
    drive_to_completion(&mut fiber, &mut step).unwrap();
    assert!(saw_call_return);
    assert!(saw_root_return);

    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    let callee = match events[2] {
        BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
            function_index,
            ..
        }) => *function_index,
        _ => panic!("expected callee frame entry"),
    };
    let callee_return = match events[3] {
        BytecodeExecutionEvent::VmFunctionReturned(returned) => returned,
        _ => panic!("expected callee return"),
    };
    assert_eq!(callee_return.function_index, callee);
    assert_eq!(callee_return.caller_function_index, Some(root));
    assert_eq!(callee_return.remaining_frame_depth, 1);
}

#[test]
fn rejected_call_and_return_emit_no_events_and_consume_no_claims() {
    let call_fixture = ObservationFixture::build(
        "example.com/fiber-observation-rejected-call",
        LOCAL_CALL_SOURCE,
    );
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut rejected = call_fixture.start(
        single_frame_limits(),
        observer.clone(),
        Box::new([ValueSlot::number(1.0)]),
    );
    assert!(matches!(
        drive_until_error(&mut rejected),
        VmError::FrameLimitExceeded { .. }
    ));
    {
        let records = sink.0.lock().unwrap();
        assert!(!records.iter().any(|record| {
            matches!(
                record.event,
                BytecodeExecutionEvent::VmLocalCallDispatched(_)
                    | BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
                        role: VmObservedFrameRole::FirstRootLocalCallee,
                        ..
                    })
                    | BytecodeExecutionEvent::VmFunctionReturned(_)
            )
        }));
    }

    let mut accepted =
        call_fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    drive_to_completion(&mut accepted, &mut |_| {}).unwrap();
    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    assert_eq!(
        events.len(),
        5,
        "rejected call must not consume selection claims"
    );
    assert!(events
        .iter()
        .any(|event| { matches!(event, BytecodeExecutionEvent::VmLocalCallDispatched(_)) }));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
                role: VmObservedFrameRole::FirstRootLocalCallee,
                ..
            })
        )
    }));

    let root_fixture = ObservationFixture::build(
        "example.com/fiber-observation-rejected-return",
        ROOT_ONLY_SOURCE,
    );
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = root_fixture.start(vm_limits(), observer, Box::<[ValueSlot]>::default());
    let (function, instruction) = {
        let frame = fiber.frames.last().unwrap();
        (frame.function(), frame.instruction())
    };
    let mut heap = TestHeap;
    let mut lifecycle = crate::lifecycle::LifecycleExecutor::new(&mut heap);
    assert!(matches!(
        fiber.execute_return(&mut lifecycle, function, instruction),
        Err(VmError::OperandStackUnderflow { .. })
    ));
    assert!(sink
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|record| !matches!(record.event, BytecodeExecutionEvent::VmFunctionReturned(_))));
    assert_eq!(fiber.state(), super::VmFiberState::Runnable);

    drive_to_completion(&mut fiber, &mut |_| {}).unwrap();
    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    assert!(events.iter().any(|event| {
        matches!(
            event,
            BytecodeExecutionEvent::VmFunctionReturned(VmFunctionReturned {
                role: VmObservedFrameRole::Root,
                ..
            })
        )
    }));
}

#[test]
fn repeated_root_local_calls_stop_at_the_fixed_event_maximum() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-observation-repeated",
        REPEATED_CALL_SOURCE,
    );
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    let mut depth = 1usize;
    let mut call_transitions = 0usize;
    drive_to_completion(&mut fiber, &mut |fiber| {
        let next = fiber.frames.len();
        if next == 2 && depth == 1 {
            call_transitions += 1;
        }
        depth = next;
    })
    .unwrap();
    assert_eq!(call_transitions, 2, "helper was really called twice");

    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    assert_eq!(events.len(), 5);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmFunctionFrameEntered(_)))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmLocalCallDispatched(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmFunctionReturned(_)))
            .count(),
        2
    );
}

#[test]
fn deep_local_calls_stop_at_the_fixed_event_maximum() {
    let fixture = ObservationFixture::build("example.com/fiber-observation-deep", DEEP_CALL_SOURCE);
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    let mut max_depth = 0usize;
    drive_to_completion(&mut fiber, &mut |fiber| {
        max_depth = max_depth.max(fiber.frames.len());
    })
    .unwrap();
    assert_eq!(max_depth, 3, "the deep call chain really executed");

    let records = sink.0.lock().unwrap();
    let events = new_events(&records);
    assert_eq!(events.len(), 5);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmFunctionFrameEntered(_)))
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmLocalCallDispatched(_)))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, BytecodeExecutionEvent::VmFunctionReturned(_)))
            .count(),
        2
    );

    let selected_callee = match events[2] {
        BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
            function_index,
            ..
        }) => *function_index,
        _ => panic!("expected selected callee entry"),
    };
    let returned_callee = match events[3] {
        BytecodeExecutionEvent::VmFunctionReturned(VmFunctionReturned {
            function_index, ..
        }) => *function_index,
        _ => panic!("expected selected callee return"),
    };
    assert_eq!(returned_callee, selected_callee);
    let root = fixture.root_function_index();
    assert!(
        events.iter().all(|event| match event {
            BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
                function_index,
                ..
            })
            | BytecodeExecutionEvent::VmFunctionReturned(VmFunctionReturned {
                function_index,
                ..
            }) => *function_index == root || *function_index == selected_callee,
            _ => true,
        }),
        "the intermediate frame must not mint events"
    );
}

#[test]
fn new_event_payloads_expose_only_scalar_coordinates() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-observation-payload-shape",
        LOCAL_CALL_SOURCE,
    );
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink.clone(), observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    drive_to_completion(&mut fiber, &mut |_| {}).unwrap();

    for record in sink.0.lock().unwrap().iter() {
        match record.event {
            BytecodeExecutionEvent::VmFunctionFrameEntered(VmFunctionFrameEntered {
                role,
                function_index,
                frame_depth,
                slot_count,
            }) => {
                assert!(matches!(
                    role,
                    VmObservedFrameRole::Root | VmObservedFrameRole::FirstRootLocalCallee
                ));
                let _ = (function_index, frame_depth, slot_count);
            }
            BytecodeExecutionEvent::VmLocalCallDispatched(VmLocalCallDispatched {
                caller_function_index,
                callee_function_index,
                caller_frame_depth,
                callee_frame_depth,
            }) => {
                let _ = (
                    caller_function_index,
                    callee_function_index,
                    caller_frame_depth,
                    callee_frame_depth,
                );
            }
            BytecodeExecutionEvent::VmFunctionReturned(VmFunctionReturned {
                role,
                function_index,
                caller_function_index,
                remaining_frame_depth,
            }) => {
                assert!(matches!(
                    role,
                    VmObservedFrameRole::Root | VmObservedFrameRole::FirstRootLocalCallee
                ));
                let _ = (function_index, caller_function_index, remaining_frame_depth);
            }
            _ => {}
        }
    }
}

fn compile_fixture_package(
    package_id: &str,
    text: &str,
) -> (Arc<PackageArtifact>, Arc<ValidatedBytecodeArtifact>) {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let package_id = skiff_compiler_core::id::PublicationId::parse(package_id).unwrap();
    let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let temp = std::env::temp_dir().join(format!(
        "skiff-vm-observation-{}-{}-{}",
        std::process::id(),
        unique,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp).unwrap();
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).unwrap();
    let source_tree = SourceTree {
        root: temp.clone(),
        sources: vec![SourceTreeFile {
            module_path: "main".to_string(),
            file_path: PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .unwrap();
    let package = PackageSourceInput::new(
        PublicationManifest::new(
            package_id.clone(),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            ManifestProvenance {
                owner: ManifestOwner::UserOrBuiltinPackage,
                path: PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let compiled = compile_package(PackageCompileInput::new(
        &platform_sources,
        &package,
        &BTreeMap::new(),
        package_id.as_str(),
        true,
    ))
    .unwrap();
    let bytecode = Arc::new(
        ValidatedBytecodeArtifact::admit(compiled.bytecode_handoff().unwrap().artifact().clone())
            .unwrap(),
    );
    let package = Arc::new(compiled.package().artifact.clone());
    std::fs::remove_dir_all(temp).unwrap();
    (package, bytecode)
}

fn service_contract(package_id: &str) -> (Arc<ServiceContract>, ContractOperationId) {
    let operation =
        skiff_artifact_identity::contract_operation_id(package_id, "1.0.0", "run").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: package_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: skiff_artifact_model::ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation.clone(),
                stable_key: "run".to_string(),
                contract: BoundaryOperationContract {
                    parameters: Vec::new(),
                    return_value: BoundaryReturn {
                        ty: ContractTypeRef::builtin("number"),
                        value_plan: BoundaryValuePlan::Linkable {
                            carrier: BoundaryValueCarrier::DetachedValueGraph,
                            encoding: BoundaryValueEncoding::CanonicalValue,
                            owner: BoundaryValueOwner::Provider,
                            lifetime: BoundaryValueLifetime::Call,
                        },
                    },
                    stream: BoundaryStreamContract::Unary,
                    callbacks: BoundaryCallbackContract::None,
                    effect_guarantee: BoundaryEffectGuarantee {
                        detached_parameters: true,
                        detached_return: true,
                        detached_error: true,
                        no_caller_reachable_mutation: true,
                        no_caller_value_escape: true,
                        no_same_heap_identity: true,
                    },
                },
            },
        )]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: skiff_artifact_model::ContractDiagnosticText {
            service: package_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (Arc::new(contract), operation)
}

fn service_deployment(
    package: &PackageArtifact,
    contract: &ServiceContract,
    operation: ContractOperationId,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let package_ref = skiff_artifact_identity::package_artifact_ref(package).unwrap();
    let contract_ref = skiff_artifact_identity::service_contract_ref(contract).unwrap();
    let callable = package
        .callable_links
        .keys()
        .find(|callable| callable.as_str().ends_with(":main.run"))
        .expect("compiled fixture exposes the main.run callable")
        .clone();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("revision:fiber-observation"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref,
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation,
            package_callable_id: callable,
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "fiber observation".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

struct TestResolver {
    deployment: Arc<ServiceDeployment>,
    contract: Arc<ServiceContract>,
    package: Arc<PackageArtifact>,
    bytecode: Arc<ValidatedBytecodeArtifact>,
}

impl DeploymentBytecodeContentResolver for TestResolver {
    fn resolve_deployment(
        &self,
        reference: &skiff_artifact_model::ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        let actual = skiff_artifact_identity::service_deployment_ref(&self.deployment);
        anyhow::ensure!(&actual == reference, "deployment reference mismatch");
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &skiff_artifact_model::ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        let actual = skiff_artifact_identity::service_contract_ref(&self.contract).unwrap();
        anyhow::ensure!(&actual == reference, "contract reference mismatch");
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &skiff_artifact_model::PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        let actual = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual == reference, "package reference mismatch");
        Ok(Arc::clone(&self.package))
    }

    fn resolve_package_bytecode(
        &self,
        package: &skiff_artifact_model::PackageArtifactRef,
        reference: &skiff_artifact_model::BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        let actual = skiff_artifact_identity::package_artifact_ref(&self.package).unwrap();
        anyhow::ensure!(&actual == package, "bytecode package mismatch");
        anyhow::ensure!(
            self.bytecode.reference() == reference,
            "bytecode reference mismatch"
        );
        Ok(Arc::clone(&self.bytecode))
    }
}

fn link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: u64::MAX,
        max_root_specializations: u64::MAX,
        max_specializations: u64::MAX,
        max_code_words_per_function: u64::MAX,
        max_total_code_words: u64::MAX,
        max_relocations_per_function: u64::MAX,
        max_total_relocations: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_total_image_table_entries: u64::MAX,
        max_total_function_table_entries: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_expanded_type_nodes: u64::MAX,
        max_expanded_type_bytes: u64::MAX,
        max_constant_graph_nodes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

fn verification_limits() -> VerificationLimits {
    VerificationLimits {
        max_functions: u64::MAX,
        max_total_instructions: u64::MAX,
        max_instructions_per_function: u64::MAX,
        max_frame_slots_per_function: u64::MAX,
        max_operand_depth: u64::MAX,
        max_control_flow_edges_per_function: u64::MAX,
        max_exception_regions_per_function: u64::MAX,
        max_switch_targets_per_function: u64::MAX,
        max_statement_events_per_pc: u64::MAX,
        max_statement_events_per_function: u64::MAX,
        max_total_statement_events: u64::MAX,
        max_source_map_entries_per_function: u64::MAX,
        max_image_table_entries: u64::MAX,
        max_arity: u64::MAX,
        max_callback_captures_per_callback: u64::MAX,
        max_type_nesting_depth: u64::MAX,
        max_value_lifecycle_nodes: u64::MAX,
        max_value_lifecycle_canonical_bytes: u64::MAX,
        max_constant_graph_edges: u64::MAX,
    }
}

/// Recording heap for the lifecycle-focused dispatch test. Every physical
/// primitive is counted so the test can prove that slot transitions actually
/// route through the lifecycle executor rather than raw bit copies.
#[derive(Default)]
struct LifecycleRecordingHeap {
    shares: usize,
    transfers: usize,
    snapshot_releases: usize,
    resource_releases: usize,
}

impl VmHeap for LifecycleRecordingHeap {
    fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
        Ok(())
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.shares += 1;
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.transfers += 1;
        Ok(*source)
    }

    fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.snapshot_releases += 1;
        Ok(())
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.resource_releases += 1;
        Ok(())
    }
}

#[test]
fn lifecycle_executor_keeps_scalar_dispatch_sidecar_free() {
    let fixture = ObservationFixture::build(
        "example.com/fiber-lifecycle-recording",
        "function run(value: number) -> number {\n\
         \x20 final a = value\n\
         \x20 final b = a\n\
         \x20 return b\n\
         }\n",
    );
    let sink = Arc::new(RecordingSink::default());
    let observer = BytecodeExecutionObserver::new(sink, observation_correlation());
    let mut fiber = fixture.start(vm_limits(), observer, Box::new([ValueSlot::number(1.0)]));
    let mut heap = LifecycleRecordingHeap::default();
    for _ in 0..10_000 {
        match fiber.dispatch_one(&mut heap) {
            Ok(DispatchOutcome::Continue) => {}
            Ok(DispatchOutcome::Complete(_)) => break,
            Ok(DispatchOutcome::Handoff(_)) => panic!("lifecycle fixture must not hand off"),
            Ok(DispatchOutcome::Throw(_)) => panic!("lifecycle fixture must not throw"),
            Err(error) => panic!("lifecycle fixture failed: {error}"),
        }
    }

    // Every scalar transition takes the lifecycle executor's trivial fast
    // path: copy, return transfer, and frame-exit release all keep the
    // sidecar-free Phase 1 invariant and never touch a heap primitive.
    assert_eq!(heap.shares, 0, "scalar copy must not snapshot-share");
    assert_eq!(heap.transfers, 0, "scalar return must not transfer-owner");
    assert_eq!(
        heap.snapshot_releases, 0,
        "scalar frame exit must not release-snapshot"
    );
    assert_eq!(heap.resource_releases, 0, "no resource owners in a scalar fixture");
}

#[test]
fn catch_matchers_compare_the_runtime_leaf_not_a_static_union_type() {
    let leaf_a = TypeIndex::new(3);
    let leaf_b = TypeIndex::new(4);
    let union = TypeIndex::new(9);
    let catch_a = LinkedCatchMatcher::Type(leaf_a);
    assert!(catch_matches(&catch_a, Some(leaf_a)));
    assert!(!catch_matches(&catch_a, Some(leaf_b)));
    assert!(
        !catch_matches(&catch_a, Some(union)),
        "a union static tag must never satisfy a branch-level catch"
    );
    assert!(catch_matches(&LinkedCatchMatcher::CatchAll, None));
    assert!(catch_matches(&LinkedCatchMatcher::CatchAll, Some(leaf_a)));
}

#[test]
fn local_nominal_record_derives_a_stable_linked_execution_identity() {
    const SOURCE: &str = "type Payload { value: number }\n\
         function run(value: Payload) -> Payload { return value }\n";
    let fixture = ObservationFixture::build("test.skiff/fiber-identity", SOURCE);
    let record_index = fixture
        .image
        .types()
        .iter()
        .position(|entry| matches!(entry.type_ref(), TypeRefIr::PackageSymbol { .. }))
        .expect("fixture interns the nominal record as a package symbol");
    let leaf = TypeIndex::new(record_index as u32);
    let identity = linked_type_catch_identity(&fixture.image, leaf)
        .expect("a nominal record has a concrete leaf identity");
    assert!(matches!(
        identity,
        CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(_))
    ));
}

#[test]
fn structural_scalar_leaves_have_no_runtime_catch_identity() {
    let fixture = ObservationFixture::build(
        "test.skiff/fiber-scalar-identity",
        "function run(value: number) -> number { return value }\n",
    );
    assert!(runtime_leaf_catch_identity(&fixture.image, &ValueSlot::number(1.0)).is_none());
    assert!(runtime_leaf_catch_identity(&fixture.image, &ValueSlot::null()).is_none());
}

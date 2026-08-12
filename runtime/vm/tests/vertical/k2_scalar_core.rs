use std::{num::NonZeroUsize, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    derive_bytecode_statement_manifest_identity, BytecodeFunctionStatementManifest,
    InstructionSourceSite, Opcode, StatementAttributionId, StatementEntry,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_model::vm_heap::{VmHeapOperation, VmMapEntry, VmRecordField};

use super::*;

const SCALAR_SOURCE: &str =
    "function helper(value: number) -> number { return value + 5 }\n\
     function run(value: number) -> number { final stored = helper(value) if stored == 7 { return stored - 4 } return 0 }\n";

#[test]
fn production_image_executes_source_scalar_slot_branch_local_call_and_return_without_lifecycle() {
    let fixture = ExecutionFixture::from_source(
        "example.com/vm-k2-source-scalar",
        SCALAR_SOURCE,
        "skiff-vm-k2-source-scalar",
    );
    let opcodes = fixture.opcodes();
    assert_no_unwind_or_pending_opcode(&opcodes);
    for expected in [
        Opcode::LoadSlot,
        Opcode::StoreSlot,
        Opcode::Add,
        Opcode::Equal,
        Opcode::JumpIfFalse,
        Opcode::Subtract,
        Opcode::CallLocal,
        Opcode::Return,
    ] {
        assert!(
            opcodes.contains(&expected),
            "source fixture emits {expected:?}"
        );
    }

    let outcome = fixture.execute(
        ValueSlot::number(2.0),
        vm_limits(32, 512),
        SpyHeap::default(),
    );

    assert_eq!(outcome.scalar_result, Some(3.0));
    assert!(outcome.max_frames >= 2, "local call pushes a VM frame");
    assert!(outcome.max_value_slots > outcome.root_value_slots);
    assert_eq!(outcome.heap, SpyHeap::default());
}

#[test]
fn verifier_owned_slot_transfer_fixture_executes_copy_and_move_without_heap_sidecars() {
    // The production compiler has no scalar CopySlot producer and emits
    // MoveSlot only for the disabled Stream lane. This fixture therefore does
    // not claim source reachability: it changes a compiler-produced scalar
    // artifact, reassigns every content/owner identity, and then must pass the
    // same structural loader, sole image linker, verifier, opaque entry and VM
    // path as production.
    let source =
        "function run(value: number) -> number { final first = value final second = first return second }\n";
    let mut fixture = ExecutionFixture::from_source(
        "example.com/vm-k2-slot-transfer",
        source,
        "skiff-vm-k2-slot-transfer",
    );
    let run = fixture.function_mut("main::run");
    let synthetic = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    };
    run.words = vec![
        opcode_word(Opcode::CopySlot),
        0,
        1,
        opcode_word(Opcode::MoveSlot),
        1,
        2,
        opcode_word(Opcode::LoadSlot),
        2,
        opcode_word(Opcode::Return),
    ];
    run.statement_entries = vec![StatementEntry {
        pc: 0,
        sequence_ordinal: 0,
        attribution_id: StatementAttributionId::Generated { ordinal: 0 },
        site: synthetic.clone(),
    }];
    run.source_map.clear();
    run.source_map.push(skiff_artifact_model::SourceMapEntry {
        start_pc: 0,
        end_pc: u32::try_from(run.words.len()).unwrap(),
        site: synthetic,
    });
    fixture.republish_verifier_owned_artifact();
    assert_no_unwind_or_pending_opcode(&fixture.opcodes());

    let outcome = fixture.execute(
        ValueSlot::number(13.0),
        vm_limits(8, 64),
        SpyHeap::default(),
    );

    assert_eq!(outcome.scalar_result, Some(13.0));
    assert_eq!(outcome.heap, SpyHeap::default());
}

#[test]
fn source_deep_local_call_stays_in_dispatch_loop_and_hits_frame_and_value_bounds() {
    let source = "function dive(value: number) -> number { final deeper = dive(value) return deeper + 0 }\n\
                  function run(value: number) -> number { final result = dive(value) return result + 0 }\n";
    let fixture = ExecutionFixture::from_source(
        "example.com/vm-k2-frame-bound",
        source,
        "skiff-vm-k2-frame-bound",
    );
    let opcodes = fixture.opcodes();
    assert_no_unwind_or_pending_opcode(&opcodes);
    assert!(opcodes.contains(&Opcode::CallLocal));
    assert!(!opcodes.contains(&Opcode::TailCallLocal));
    let root_segment = fixture.segment_len("main::run");
    let child_segment = fixture.segment_len("main::dive");
    let frame_value_limit = root_segment + child_segment * 4095;

    // vm_limits fixes every segment at one instruction. Reaching 4096 live VM
    // frames therefore requires CallLocal to return to the same outer
    // run_segment loop after each push; a recursively invoked native evaluator
    // cannot manufacture this observable frame-vector progression.
    let failure = fixture.execute_error(
        ValueSlot::number(1.0),
        vm_limits(4096, frame_value_limit),
        SpyHeap::default(),
    );
    assert_eq!(
        failure.error,
        skiff_runtime_vm::VmError::FrameLimitExceeded { limit: 4096 }
    );
    assert_eq!(failure.max_frames, 4096);
    assert_eq!(failure.heap, SpyHeap::default());

    let value_limit = root_segment + child_segment;
    let requested = value_limit + child_segment;
    let value_failure = fixture.execute_error(
        ValueSlot::number(1.0),
        vm_limits(4096, value_limit),
        SpyHeap::default(),
    );
    assert!(matches!(
        value_failure.error,
        skiff_runtime_vm::VmError::ValueStackLimitExceeded {
            limit,
            requested: actual,
        } if limit == value_limit && actual == requested
    ));
    assert_eq!(value_failure.max_frames, 2);
    assert_eq!(value_failure.max_value_slots, value_limit);
    assert_eq!(value_failure.heap, SpyHeap::default());
}

fn opcode_word(opcode: Opcode) -> u32 {
    u32::from(skiff_artifact_model::descriptor_for_opcode(opcode).opcode)
}

fn assert_no_unwind_or_pending_opcode(opcodes: &[Opcode]) {
    for disabled in [
        Opcode::Throw,
        Opcode::Rethrow,
        Opcode::EnterRegion,
        Opcode::LeaveRegion,
        Opcode::CallService,
        Opcode::CallActor,
        Opcode::CallInterface,
        Opcode::InvokeHost,
        Opcode::InvokeIntrinsic,
        Opcode::MakeCallback,
        Opcode::InvokeCallback,
        Opcode::StreamNext,
        Opcode::EmitStream,
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
        Opcode::InterfaceBoxLocal,
        Opcode::InterfaceBoxRemote,
    ] {
        assert!(
            !opcodes.contains(&disabled),
            "accepted scalar image excludes {disabled:?}"
        );
    }
}

fn vm_limits(max_frames: usize, max_value_slots: usize) -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(max_frames).unwrap(),
        NonZeroUsize::new(max_value_slots).unwrap(),
        NonZeroU32::new(1024).unwrap(),
        NonZeroU32::new(1).unwrap(),
    )
}

fn service_deployment_for_function(
    package: &PackageArtifact,
    bytecode: &ValidatedBytecodeArtifact,
    contract: &ServiceContract,
    operation: skiff_artifact_model::ContractOperationId,
    function_key: &str,
) -> (
    Arc<ServiceDeployment>,
    skiff_artifact_model::ServiceDeploymentRef,
) {
    let executable = bytecode
        .artifact()
        .image
        .functions
        .get(function_key)
        .and_then(|function| function.origin.ordinary_executable())
        .unwrap_or_else(|| panic!("fixture has ordinary executable {function_key}"));
    let callable = package
        .callable_links
        .values()
        .find(|link| {
            link.target.file_ref.file_ir_identity == executable.file_ir_identity
                && link.target.file_ref.module_path == executable.module_path
                && link.target.executable_index == executable.executable_index
        })
        .map(|link| link.callable_id.clone())
        .unwrap_or_else(|| panic!("package links exact executable {function_key}"));
    let (deployment, _) = service_deployment(package, contract, operation);
    let mut deployment = deployment.as_ref().clone();
    deployment.operation_bindings[0].package_callable_id = callable;
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    (Arc::new(deployment), reference)
}

struct ExecutionFixture {
    package: Arc<PackageArtifact>,
    bytecode: skiff_artifact_model::BytecodeArtifact,
    package_id: String,
}

impl ExecutionFixture {
    fn from_source(package_id: &str, source: &str, prefix: &str) -> Self {
        let (package, bytecode) =
            compile_package_with_dependencies(package_id, source, prefix, Vec::new(), &[]);
        Self {
            package,
            bytecode: bytecode.artifact().clone(),
            package_id: package_id.to_string(),
        }
    }

    fn function_mut(
        &mut self,
        key: &str,
    ) -> &mut skiff_artifact_model::RelocatableBytecodeFunction {
        self.bytecode
            .image
            .functions
            .get_mut(key)
            .unwrap_or_else(|| panic!("compiler emits {key}"))
    }

    fn republish_verifier_owned_artifact(&mut self) {
        skiff_artifact_identity::assign_bytecode_identity(&mut self.bytecode).unwrap();
        let admitted = ValidatedBytecodeArtifact::admit(self.bytecode.clone()).unwrap();

        let package = Arc::make_mut(&mut self.package);
        package.bytecode = Some(admitted.reference().clone());
        let mut manifests = admitted
            .view()
            .functions()
            .iter()
            .map(|function| {
                BytecodeFunctionStatementManifest::new(
                    function.origin.clone(),
                    function.statement_entries.clone(),
                )
            })
            .collect::<Vec<_>>();
        manifests.sort_by(|left, right| left.origin.cmp(&right.origin));
        package.bytecode_statement_manifest_identity =
            derive_bytecode_statement_manifest_identity(&package.package_id, &manifests).unwrap();
        skiff_artifact_identity::assign_package_artifact_identities(package).unwrap();
    }

    fn opcodes(&self) -> Vec<Opcode> {
        skiff_artifact_model::structurally_validate(&self.bytecode)
            .unwrap()
            .functions()
            .iter()
            .flat_map(|function| function.instructions.iter())
            .map(|instruction| instruction.descriptor.kind)
            .collect()
    }

    fn segment_len(&self, key: &str) -> usize {
        let (image, _) = self.build_image();
        let function = image
            .functions()
            .iter()
            .find(|function| function.key().artifact_function_key().as_str() == key)
            .unwrap_or_else(|| panic!("linked image contains {key}"));
        function.frame().slot_types().len() + usize::try_from(function.max_operand_depth()).unwrap()
    }

    fn build_image(
        &self,
    ) -> (
        Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        skiff_artifact_model::ContractOperationId,
    ) {
        let bytecode = Arc::new(ValidatedBytecodeArtifact::admit(self.bytecode.clone()).unwrap());
        let (contract, operation) = service_contract(&self.package_id);
        let (deployment, reference) = service_deployment_for_function(
            &self.package,
            &bytecode,
            &contract,
            operation.clone(),
            "main::run",
        );
        let resolver = TestResolver {
            deployment,
            contract,
            package: Arc::clone(&self.package),
            bytecode,
        };
        let hydrated = DeploymentBytecodeLoader::new(&resolver)
            .load(&reference)
            .unwrap();
        let image = Arc::new(
            link_deployment_execution_image(
                hydrated,
                &DeploymentExecutionLimits::new(
                    generous_link_limits(),
                    generous_verification_limits(),
                ),
            )
            .unwrap(),
        );
        (image, operation)
    }

    fn start(&self, argument: ValueSlot, limits: VmLimits) -> skiff_runtime_vm::VmFiber {
        let (image, operation) = self.build_image();
        let entry = image.operation_entry(&operation).unwrap();
        Vm::start(entry, Box::new([argument]), limits, noop_observer()).unwrap()
    }

    fn execute(
        &self,
        argument: ValueSlot,
        limits: VmLimits,
        mut heap: SpyHeap,
    ) -> ExecutionOutcome {
        let mut fiber = self.start(argument, limits);
        let root_value_slots = fiber.allocated_value_slot_count();
        let mut max_frames = fiber.active_frame_count();
        let mut max_value_slots = root_value_slots;
        let mut budget = TestBudget::new();
        loop {
            max_frames = max_frames.max(fiber.active_frame_count());
            max_value_slots = max_value_slots.max(fiber.allocated_value_slot_count());
            match fiber.run_segment(&mut heap, &mut budget) {
                VmControl::Continue => {}
                VmControl::Complete(Ok(values)) => {
                    return ExecutionOutcome {
                        scalar_result: values.values().first().and_then(ValueSlot::as_number),
                        root_value_slots,
                        max_frames,
                        max_value_slots,
                        heap,
                    };
                }
                VmControl::Complete(Err(error)) => panic!("scalar execution failed: {error}"),
                _ => panic!("accepted scalar path left the synchronous VM dispatch loop"),
            }
        }
    }

    fn execute_error(
        &self,
        argument: ValueSlot,
        limits: VmLimits,
        mut heap: SpyHeap,
    ) -> ExecutionFailure {
        let mut fiber = self.start(argument, limits);
        let mut budget = TestBudget::new();
        let mut max_frames = fiber.active_frame_count();
        let mut max_value_slots = fiber.allocated_value_slot_count();
        loop {
            max_frames = max_frames.max(fiber.active_frame_count());
            max_value_slots = max_value_slots.max(fiber.allocated_value_slot_count());
            match fiber.run_segment(&mut heap, &mut budget) {
                VmControl::Continue => {}
                VmControl::Complete(Err(error)) => {
                    return ExecutionFailure {
                        error,
                        max_frames,
                        max_value_slots,
                        heap,
                    }
                }
                VmControl::Complete(Ok(_)) => {
                    panic!("deep recursive fixture unexpectedly completed")
                }
                _ => panic!("deep recursive fixture left the synchronous VM loop"),
            }
        }
    }
}

struct ExecutionOutcome {
    scalar_result: Option<f64>,
    root_value_slots: usize,
    max_frames: usize,
    max_value_slots: usize,
    heap: SpyHeap,
}

struct ExecutionFailure {
    error: skiff_runtime_vm::VmError,
    max_frames: usize,
    max_value_slots: usize,
    heap: SpyHeap,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SpyHeap {
    validate_live: usize,
    snapshot_share: usize,
    transfer_owner: usize,
    release_snapshot: usize,
    release_resource: usize,
    aggregate_or_cow: usize,
}

impl VmHeap for SpyHeap {
    fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
        panic!("accepted scalar path called validate_live")
    }

    fn snapshot_share(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.snapshot_share += 1;
        panic!("accepted scalar path called snapshot_share")
    }

    fn transfer_owner(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.transfer_owner += 1;
        panic!("accepted scalar path called transfer_owner")
    }

    fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_snapshot += 1;
        panic!("accepted scalar path called release_snapshot")
    }

    fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_resource += 1;
        panic!("accepted scalar path called release_resource")
    }

    fn allocate_array(
        &mut self,
        _elements: &[ValueSlot],
        _tag: skiff_runtime_model::vm_value::CompactTypeTag,
        _flags: skiff_runtime_model::vm_value::ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.aggregate_or_cow += 1;
        Err(unexpected_heap(VmHeapOperation::AllocateArray))
    }

    fn allocate_map(
        &mut self,
        _entries: &[VmMapEntry],
        _tag: skiff_runtime_model::vm_value::CompactTypeTag,
        _flags: skiff_runtime_model::vm_value::ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.aggregate_or_cow += 1;
        Err(unexpected_heap(VmHeapOperation::AllocateMap))
    }

    fn allocate_record(
        &mut self,
        _fields: &[VmRecordField],
        _tag: skiff_runtime_model::vm_value::CompactTypeTag,
        _flags: skiff_runtime_model::vm_value::ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.aggregate_or_cow += 1;
        Err(unexpected_heap(VmHeapOperation::AllocateRecord))
    }
}

fn unexpected_heap(operation: VmHeapOperation) -> VmHeapError {
    VmHeapError::HeapOperationFailed {
        operation,
        message: "accepted scalar path reached an aggregate/COW heap operation".to_string(),
    }
}

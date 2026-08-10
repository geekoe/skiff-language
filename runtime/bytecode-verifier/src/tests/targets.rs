use skiff_artifact_model::{
    host_effect_registry_identity, intrinsic_registry_identity,
    native_value_lifecycle_registry_identity, opcode_table_fingerprint,
    value_lifecycle_policy_identity, BytecodeArtifactRef, CallableEffectSummary,
    NativeValueDropPlan, NativeValueEmbedding, NativeValueLifecycleConcrete,
    NativeValueLifecycleResolution, Opcode, PackageBuildId, PackageCallableId, ParamModeIr,
    TypeRefIr, BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactFunctionKey, ArtifactTypeIndex, BytecodePackageIndex, FrameSlotIndex, FunctionIndex,
    InstructionIndex, LinkedArtifactPoolOrigin, LinkedBytecodeAuthorityPins,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateParts, LinkedCallableEffectDeclaration,
    LinkedExactLocalTarget, LinkedFrameLayout, LinkedFunction, LinkedFunctionTables,
    LinkedInstruction, LinkedInstructionTarget, LinkedPackageBytecodeProvenance,
    LinkedParameterSlot, LinkedProgramPointState, LinkedResolvedOperand, LinkedSlotState,
    LinkedStackMapCandidate, LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    SpecializationKey, TypeIndex,
};

use crate::{
    concrete_values::ConcreteValueFacts, control_flow::prove_exact_local_call_plan_for_test,
    verify, VerificationError, VerificationLocation, VerificationObligation,
};

use super::fixtures::{
    generous_limits, loader_backed_local_call, LocalCallCandidateCorruption, TARGET_FUNCTION_INDEX,
};

#[test]
fn loader_backed_local_target_authority_advances_to_frozen_constant_safety() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("exact hydrated local authority must cross P3 target proof");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::FrozenConstantSafety,
            location: VerificationLocation::Image,
        }
    );
}

#[test]
fn loader_backed_target_summary_drift_is_stopped_by_exact_binding() {
    let (hydrated, candidate) =
        loader_backed_local_call(LocalCallCandidateCorruption::TargetDeclarativeSummary);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("candidate effect summary drift must fail closed");

    assert_exact_function_binding_rejection(error);
}

#[test]
fn loader_backed_target_effect_owner_drift_is_stopped_by_exact_binding() {
    let (hydrated, candidate) =
        loader_backed_local_call(LocalCallCandidateCorruption::TargetEffectOwner);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("candidate effect owner drift must fail closed");

    assert_exact_function_binding_rejection(error);
}

#[test]
fn loader_backed_wrong_canonical_function_authority_is_stopped_by_exact_binding() {
    let (hydrated, candidate) =
        loader_backed_local_call(LocalCallCandidateCorruption::TargetCanonicalFunction);
    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("candidate canonical-function drift must fail closed");

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Instruction {
                function,
                instruction,
            },
            detail,
        } if function == FunctionIndex::new(0)
            && instruction == InstructionIndex::new(0)
            && detail.contains("local executable relocation target")
    ));
}

fn assert_exact_function_binding_rejection(error: VerificationError) {
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Function { function },
            detail,
        } if function == TARGET_FUNCTION_INDEX
            && detail.contains("differ from the admitted artifact")
    ));
}

#[test]
fn exact_call_local_zero_arity_plan_is_proved() {
    prove(&candidate(
        Opcode::CallLocal,
        0,
        Some(0),
        ParamModeIr::Value,
    ))
    .expect("exact zero-arity fallthrough call has a complete P3 call plan");
}

#[test]
fn exact_tail_call_zero_arity_plan_is_proved() {
    prove(&candidate(
        Opcode::TailCallLocal,
        0,
        None,
        ParamModeIr::Value,
    ))
    .expect("exact zero-arity tail call has a complete P3 call plan");
}

#[test]
fn tail_call_accepts_different_raw_indices_in_one_concrete_class() {
    let types = [TypeRefIr::builtin("string"), TypeRefIr::builtin("string")];
    let facts = facts_for_results(types.clone());
    let candidate = candidate_with_results(types);

    prove_with_facts(&candidate, &facts)
        .expect("tail-call results in one semantic class must compare equal");
}

#[test]
fn tail_call_rejects_equal_plans_from_different_concrete_classes() {
    let types = [TypeRefIr::builtin("string"), TypeRefIr::builtin("bytes")];
    let facts = facts_for_results(types.clone());
    let error = prove_with_facts(&candidate_with_results(types), &facts)
        .expect_err("different semantic result classes must fail closed");

    assert_target_violation(error, "differs from the caller result type");
}

#[test]
fn wrong_local_argument_count_is_rejected_at_the_call_site() {
    let error = prove(&candidate(
        Opcode::CallLocal,
        1,
        Some(0),
        ParamModeIr::Value,
    ))
    .expect_err("ArgCount must come from and match the exact target frame");

    assert_target_violation(error, "ArgCount");
}

#[test]
fn wrong_local_result_count_is_rejected_at_the_call_site() {
    let error = prove(&candidate(
        Opcode::CallLocal,
        0,
        Some(1),
        ParamModeIr::Value,
    ))
    .expect_err("ResultCount must come from and match the exact target frame");

    assert_target_violation(error, "ResultCount");
}

#[test]
fn ordinary_call_to_inout_target_remains_unavailable() {
    let error = prove(&candidate(
        Opcode::CallLocal,
        1,
        Some(0),
        ParamModeIr::InOut,
    ))
    .expect_err("ordinary local calls cannot silently consume an InOut parameter");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
            location: call_location(),
        }
    );
}

fn prove(candidate: &LinkedBytecodeCandidate) -> Result<(), VerificationError> {
    let facts = ConcreteValueFacts::empty_for_test();
    prove_with_facts(candidate, &facts)
}

fn prove_with_facts(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    prove_exact_local_call_plan_for_test(
        candidate,
        facts,
        FunctionIndex::new(0),
        InstructionIndex::new(0),
        FunctionIndex::new(1),
        &generous_limits(),
    )
}

fn candidate_with_results(types: [TypeRefIr; 2]) -> LinkedBytecodeCandidate {
    candidate_with_result_types(Some(types))
}

fn candidate(
    opcode: Opcode,
    argument_count: u32,
    result_count: Option<u32>,
    target_mode: ParamModeIr,
) -> LinkedBytecodeCandidate {
    candidate_with_result_types_and_call(opcode, argument_count, result_count, target_mode, None)
}

fn candidate_with_result_types(types: Option<[TypeRefIr; 2]>) -> LinkedBytecodeCandidate {
    candidate_with_result_types_and_call(Opcode::TailCallLocal, 0, None, ParamModeIr::Value, types)
}

fn candidate_with_result_types_and_call(
    opcode: Opcode,
    argument_count: u32,
    result_count: Option<u32>,
    target_mode: ParamModeIr,
    result_types: Option<[TypeRefIr; 2]>,
) -> LinkedBytecodeCandidate {
    let build = PackageBuildId::new("package-build:p3-call-plan-test");
    let caller_key = key(&build, "module::caller", "callable:caller");
    let target_key = key(&build, "module::target", "callable:target");
    let mut caller_instructions = vec![call_instruction(opcode, argument_count, result_count)];
    if opcode != Opcode::TailCallLocal {
        caller_instructions.push(plain(Opcode::Return));
    }
    let (caller_frame, target_frame, types) = if let Some([caller_type, target_type]) = result_types
    {
        assert_eq!(target_mode, ParamModeIr::Value);
        (
            result_frame(TypeIndex::new(0)),
            result_frame(TypeIndex::new(1)),
            vec![
                linked_type(&build, 0, caller_type),
                linked_type(&build, 1, target_type),
            ],
        )
    } else {
        let target_frame = if target_mode == ParamModeIr::Value {
            empty_frame()
        } else {
            parameter_frame(target_mode)
        };
        let types = (target_mode != ParamModeIr::Value)
            .then(|| linked_type(&build, 0, TypeRefIr::builtin("string")))
            .into_iter()
            .collect();
        (empty_frame(), target_frame, types)
    };
    let caller = function(
        FunctionIndex::new(0),
        caller_key.clone(),
        caller_instructions,
        caller_frame,
    );
    let target = function(
        FunctionIndex::new(1),
        target_key.clone(),
        vec![plain(Opcode::Return)],
        target_frame,
    );
    LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
        packages: vec![linked_package(build)],
        functions: vec![caller, target],
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: vec![
            LinkedExactLocalTarget::new(caller_key, FunctionIndex::new(0)),
            LinkedExactLocalTarget::new(target_key, FunctionIndex::new(1)),
        ],
        service_operations: Vec::new(),
        actor_creates: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        callback_capture_layouts: Vec::new(),
        host_effect_adapters: Vec::new(),
        intrinsics: Vec::new(),
        types,
        shapes: Vec::new(),
        constants: Vec::new(),
        constant_roots: Vec::new(),
        frozen_constant_nodes: Vec::new(),
        resume_sites: Vec::new(),
        writable_paths: Vec::new(),
    })
    .unwrap()
}

fn linked_type(build: &PackageBuildId, index: u32, ty: TypeRefIr) -> LinkedTypeEntry {
    LinkedTypeEntry::new(
        TypeIndex::new(index),
        LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(index), None).unwrap(),
        ty,
        None,
    )
}

fn function(
    index: FunctionIndex,
    key: SpecializationKey,
    instructions: Vec<LinkedInstruction>,
    frame: LinkedFrameLayout,
) -> LinkedFunction {
    let states = (0..instructions.len())
        .map(|ordinal| {
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(ordinal).unwrap()),
                Box::new([]),
                (0..frame.slot_types().len())
                    .map(|_| LinkedSlotState::Uninitialized)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let stack_map = LinkedStackMapCandidate::try_new(
        states.into_boxed_slice(),
        instructions.len(),
        frame.slot_types().len(),
        1,
    )
    .unwrap();
    let callable = key.template_function_key().clone();
    LinkedFunction::new(
        index,
        key,
        instructions.into_boxed_slice(),
        frame,
        1,
        LinkedCallableEffectDeclaration::new(callable, CallableEffectSummary::analysis_pending()),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        ),
        stack_map,
    )
}

fn empty_frame() -> LinkedFrameLayout {
    LinkedFrameLayout::new(
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
    )
    .unwrap()
}

fn parameter_frame(mode: ParamModeIr) -> LinkedFrameLayout {
    let plan = snapshot_plan();
    LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            mode,
            plan.clone(),
        )]),
        Box::new([]),
        Box::new([]),
        Box::new([plan]),
        Box::new([]),
    )
    .unwrap()
}

fn result_frame(ty: TypeIndex) -> LinkedFrameLayout {
    LinkedFrameLayout::new(
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([ty]),
        Box::new([]),
        Box::new([snapshot_plan()]),
    )
    .unwrap()
}

fn facts_for_results(types: [TypeRefIr; 2]) -> ConcreteValueFacts {
    ConcreteValueFacts::from_classified_types_for_test(
        types
            .into_iter()
            .map(|ty| (ty, snapshot_resolution()))
            .collect(),
    )
    .unwrap()
}

fn snapshot_resolution() -> NativeValueLifecycleResolution {
    NativeValueLifecycleResolution {
        lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
            drop: NativeValueDropPlan::SnapshotRelease,
        },
        embedding: NativeValueEmbedding::Ordinary,
    }
}

fn snapshot_plan() -> LinkedValueTransferPlan {
    LinkedValueTransferPlan::SnapshotShare {
        drop: LinkedValueDropPlan::SnapshotRelease,
    }
}

fn call_instruction(
    opcode: Opcode,
    argument_count: u32,
    result_count: Option<u32>,
) -> LinkedInstruction {
    let operands = match result_count {
        Some(result_count) => vec![0, argument_count, result_count],
        None => vec![0, argument_count],
    };
    LinkedInstruction::new(
        opcode,
        operands.into_boxed_slice(),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(1)),
        )]),
        0,
    )
    .unwrap()
}

fn plain(opcode: Opcode) -> LinkedInstruction {
    LinkedInstruction::new(opcode, Box::new([]), Box::new([]), 0).unwrap()
}

fn key(build: &PackageBuildId, function: &str, callable: &str) -> SpecializationKey {
    SpecializationKey::new(
        build.clone(),
        ArtifactFunctionKey::parse(function).unwrap(),
        PackageCallableId::new(callable),
        Box::new([]),
        None,
    )
}

fn linked_package(build: PackageBuildId) -> LinkedPackageBytecodeProvenance {
    LinkedPackageBytecodeProvenance::new(
        BytecodePackageIndex::new(0),
        build,
        BytecodeArtifactRef::new("bytecode:p3-call-plan-test"),
        "bytecode:p3-call-plan-test",
        BYTECODE_MAGIC,
        BYTECODE_SCHEMA_VERSION,
        BYTECODE_ISA_VERSION,
        opcode_table_fingerprint(),
        LinkedBytecodeAuthorityPins::new(
            native_value_lifecycle_registry_identity().clone(),
            value_lifecycle_policy_identity().clone(),
            host_effect_registry_identity().clone(),
            intrinsic_registry_identity().clone(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn assert_target_violation(error: VerificationError, detail: &str) {
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
            location,
            detail: actual,
        } if location == call_location() && actual.contains(detail)
    ));
}

const fn call_location() -> VerificationLocation {
    VerificationLocation::Instruction {
        function: FunctionIndex::new(0),
        instruction: InstructionIndex::new(0),
    }
}

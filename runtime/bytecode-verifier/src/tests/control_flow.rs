use skiff_artifact_model::{
    host_effect_registry_identity, intrinsic_registry_identity,
    native_value_lifecycle_registry_identity, opcode_table_fingerprint,
    value_lifecycle_policy_identity, BytecodeArtifactRef, CallableEffectSummary,
    InstructionSourceSite, Opcode, PackageBuildId, PackageCallableId, ResumeErrorMode,
    SyntheticInstructionSiteReason, TypeRefIr, BYTECODE_ISA_VERSION, BYTECODE_MAGIC,
    BYTECODE_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, ArtifactFunctionKey, ArtifactTypeIndex, BytecodePackageIndex, FunctionIndex,
    InstructionBoundaryIndex, InstructionIndex, LinkedActiveRegion, LinkedActiveRegionKind,
    LinkedArtifactPoolOrigin, LinkedBytecodeAuthorityPins, LinkedBytecodeCandidate,
    LinkedBytecodeCandidateParts, LinkedCallableEffectDeclaration, LinkedFrameLayout,
    LinkedFunction, LinkedFunctionTables, LinkedInstruction, LinkedInstructionTarget,
    LinkedPackageBytecodeProvenance, LinkedProgramPointState, LinkedResolvedOperand,
    LinkedResumeSite, LinkedStackMapCandidate, LinkedSwitchCase, LinkedSwitchTable,
    LinkedTypeEntry, ResumeSiteIndex, SpecializationKey, SwitchTableIndex, TypeIndex,
};

use crate::{
    control_flow::prove_control_flow_for_test, verify, VerificationError, VerificationLimit,
    VerificationLocation, VerificationObligation,
};

use super::fixtures::{candidate_for, exact_hydration, generous_limits};

#[test]
fn empty_hydration_advances_to_stack_and_slot_state() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::StackAndSlotState,
            location: VerificationLocation::Image,
        }
    );
}

#[test]
fn straight_return_has_a_complete_ordinary_cfg() {
    prove(
        &candidate(vec![plain(Opcode::Return)], Vec::new(), Vec::new()),
        u64::MAX,
    )
    .expect("one return is a complete acyclic CFG");
}

#[test]
fn final_fallthrough_is_rejected_at_the_instruction() {
    let error = prove(
        &candidate(
            vec![plain(Opcode::BudgetCheckpoint)],
            Vec::new(),
            Vec::new(),
        ),
        u64::MAX,
    )
    .unwrap_err();

    assert_semantic(error, VerificationObligation::ControlFlow, 0);
}

#[test]
fn unreachable_instruction_is_rejected_at_its_dense_ordinal() {
    let error = prove(
        &candidate(
            vec![plain(Opcode::Return), plain(Opcode::Return)],
            Vec::new(),
            Vec::new(),
        ),
        u64::MAX,
    )
    .unwrap_err();

    assert_semantic(error, VerificationObligation::ControlFlow, 1);
}

#[test]
fn jump_self_loop_requires_a_budget_checkpoint() {
    let error = prove(
        &candidate(vec![branch(Opcode::Jump, 0)], Vec::new(), Vec::new()),
        u64::MAX,
    )
    .unwrap_err();

    assert_semantic(error, VerificationObligation::BudgetCheckpoint, 0);
}

#[test]
fn loop_through_budget_checkpoint_is_accepted() {
    prove(
        &candidate(
            vec![plain(Opcode::BudgetCheckpoint), branch(Opcode::Jump, 0)],
            Vec::new(),
            Vec::new(),
        ),
        u64::MAX,
    )
    .expect("removing the checkpoint makes the ordinary graph acyclic");
}

#[test]
fn duplicate_switch_destinations_are_one_canonical_edge() {
    let table = LinkedSwitchTable::try_new(
        Box::new([
            LinkedSwitchCase::new(TypeIndex::new(0), InstructionIndex::new(1)),
            LinkedSwitchCase::new(TypeIndex::new(1), InstructionIndex::new(1)),
        ]),
        InstructionIndex::new(1),
    )
    .unwrap();
    prove(
        &candidate(
            vec![switch(0), plain(Opcode::Return)],
            vec![table],
            Vec::new(),
        ),
        1,
    )
    .expect("case and default destinations are deduplicated before charging");
}

#[test]
fn exact_local_invoke_is_charged_but_not_added_to_cfg_cycles() {
    let candidate = candidate(
        vec![call_local(0), plain(Opcode::Return)],
        Vec::new(),
        Vec::new(),
    );
    prove(&candidate, 2).expect("fallthrough plus recursive invoke consumes two records");
    let error = prove(&candidate, 1).unwrap_err();

    assert_eq!(
        error,
        VerificationError::LimitExceeded {
            limit: VerificationLimit::ControlFlowEdgesPerFunction,
            actual: 2,
            max: 1,
            location: instruction_location(0),
        }
    );
}

#[test]
fn actual_resume_semantics_remain_fail_closed() {
    let resume = LinkedResumeSite::new(
        ResumeSiteIndex::new(0),
        FunctionIndex::new(0),
        InstructionIndex::new(0),
        InstructionIndex::new(1),
        0,
        Box::new([]),
        Box::new([]),
        ResumeErrorMode::RaiseAtSite,
    )
    .unwrap();
    let error = prove(
        &candidate(
            vec![emit_stream(0), plain(Opcode::Return)],
            Vec::new(),
            vec![resume],
        ),
        u64::MAX,
    )
    .unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ResumeSite,
            location: instruction_location(0),
        }
    );
}

#[test]
fn active_region_semantics_remain_fail_closed() {
    let region = LinkedActiveRegion::new(
        ActiveRegionIndex::new(0),
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        LinkedActiveRegionKind::Timeout {
            duration_ms: 1,
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
            },
        },
    );
    let error = prove(
        &candidate_with_regions(vec![plain(Opcode::Return)], vec![region]),
        u64::MAX,
    )
    .unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExceptionRegion,
            location: VerificationLocation::Function {
                function: FunctionIndex::new(0),
            },
        }
    );
}

fn prove(candidate: &LinkedBytecodeCandidate, max_edges: u64) -> Result<(), VerificationError> {
    let mut limits = generous_limits();
    limits.max_control_flow_edges_per_function = max_edges;
    prove_control_flow_for_test(candidate, &limits)
}

fn candidate(
    instructions: Vec<LinkedInstruction>,
    switch_tables: Vec<LinkedSwitchTable>,
    resume_sites: Vec<LinkedResumeSite>,
) -> LinkedBytecodeCandidate {
    candidate_from_parts(instructions, switch_tables, Vec::new(), resume_sites)
}

fn candidate_with_regions(
    instructions: Vec<LinkedInstruction>,
    active_regions: Vec<LinkedActiveRegion>,
) -> LinkedBytecodeCandidate {
    candidate_from_parts(instructions, Vec::new(), active_regions, Vec::new())
}

fn candidate_from_parts(
    instructions: Vec<LinkedInstruction>,
    switch_tables: Vec<LinkedSwitchTable>,
    active_regions: Vec<LinkedActiveRegion>,
    resume_sites: Vec<LinkedResumeSite>,
) -> LinkedBytecodeCandidate {
    let build = PackageBuildId::new("package-build:cfg-test");
    let function = linked_function(build.clone(), instructions, switch_tables, active_regions);
    let package = linked_package(build.clone());
    let types = (0..2)
        .map(|index| {
            LinkedTypeEntry::new(
                TypeIndex::new(index),
                LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(index), None)
                    .unwrap(),
                TypeRefIr::builtin("string"),
                None,
            )
        })
        .collect();
    LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
        packages: vec![package],
        functions: vec![function],
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
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
        resume_sites,
        writable_paths: Vec::new(),
    })
    .unwrap()
}

fn linked_function(
    build: PackageBuildId,
    instructions: Vec<LinkedInstruction>,
    switch_tables: Vec<LinkedSwitchTable>,
    active_regions: Vec<LinkedActiveRegion>,
) -> LinkedFunction {
    let states = (0..instructions.len())
        .map(|instruction| {
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(instruction).unwrap()),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let stack_map =
        LinkedStackMapCandidate::try_new(states.into_boxed_slice(), instructions.len(), 0, 0)
            .unwrap();
    LinkedFunction::new(
        FunctionIndex::new(0),
        SpecializationKey::new(
            build,
            ArtifactFunctionKey::parse("module::cfg").unwrap(),
            PackageCallableId::new("cfg"),
            Box::new([]),
            None,
        ),
        instructions.into_boxed_slice(),
        LinkedFrameLayout::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        )
        .unwrap(),
        0,
        LinkedCallableEffectDeclaration::new(
            PackageCallableId::new("cfg"),
            CallableEffectSummary::analysis_pending(),
        ),
        LinkedFunctionTables::new(
            Box::new([]),
            active_regions.into_boxed_slice(),
            switch_tables.into_boxed_slice(),
            Box::new([]),
            Box::new([]),
            Box::new([]),
        ),
        stack_map,
    )
}

fn linked_package(build: PackageBuildId) -> LinkedPackageBytecodeProvenance {
    LinkedPackageBytecodeProvenance::new(
        BytecodePackageIndex::new(0),
        build,
        BytecodeArtifactRef::new("bytecode:cfg-test"),
        "bytecode:cfg-test",
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

fn plain(opcode: Opcode) -> LinkedInstruction {
    LinkedInstruction::new(opcode, Box::new([]), Box::new([]), 0).unwrap()
}

fn branch(opcode: Opcode, target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        opcode,
        Box::new([0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Branch(InstructionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

fn switch(table: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::SwitchTag,
        Box::new([table]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::SwitchTable(SwitchTableIndex::new(table)),
        )]),
        0,
    )
    .unwrap()
}

fn call_local(target: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::CallLocal,
        Box::new([0, 0, 0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Function(FunctionIndex::new(target)),
        )]),
        0,
    )
    .unwrap()
}

fn emit_stream(resume: u32) -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::EmitStream,
        Box::new([resume]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(resume)),
        )]),
        0,
    )
    .unwrap()
}

fn assert_semantic(
    error: VerificationError,
    expected_obligation: VerificationObligation,
    instruction: u32,
) {
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation,
            location,
            ..
        } if obligation == expected_obligation && location == instruction_location(instruction)
    ));
}

const fn instruction_location(instruction: u32) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: FunctionIndex::new(0),
        instruction: InstructionIndex::new(instruction),
    }
}

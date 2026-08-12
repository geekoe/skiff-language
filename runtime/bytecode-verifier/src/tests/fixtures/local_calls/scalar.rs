use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    bytecode::opcodes::{descriptor_for_opcode, opcode_table_fingerprint},
    BytecodeArtifact, BytecodeConstantRef, BytecodeFunctionOrigin, BytecodeImage,
    BytecodePoolEntry, BytecodePools, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentRevision, FrameLayout, FrozenConstantGraph, FrozenConstantNode,
    InstructionSourceSite, LiteralIr, RelocatableBytecodeFunction, ServiceDeployment,
    SourceMapEntry, SyntheticInstructionSiteReason, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    BYTECODE_ISA_VERSION, BYTECODE_MAGIC, BYTECODE_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ArtifactTypeIndex, ConstantIndex,
    FrozenConstantNodeIndex, FunctionIndex, InstructionBoundaryIndex, InstructionIndex,
    LinkedArtifactPoolOrigin, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
    LinkedConstantEntry, LinkedConstantReference, LinkedExactLocalTarget, LinkedFrozenConstantNode,
    LinkedFrozenConstantValue, LinkedFunction, LinkedFunctionTables, LinkedInstruction,
    LinkedInstructionTarget, LinkedProgramPointState, LinkedResolvedOperand, LinkedSourceMapEntry,
    LinkedStackMapCandidate, LinkedStackValue, LinkedTypeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_loader::{DeploymentBytecodeLoader, HydratedDeploymentBytecode};

use super::{
    callable, coordinate, effects, package_with_caller_summary, specialization,
    LocalCallCandidateCorruption, CALLER_CALLABLE, CALLER_FUNCTION, TARGET_CALLABLE,
    TARGET_FUNCTION,
};
use crate::tests::fixtures::{candidate_parts, contract, ExactResolver};

pub(crate) struct ScalarFixture {
    pub(crate) hydrated: HydratedDeploymentBytecode,
    pub(crate) candidate: LinkedBytecodeCandidate,
}

#[derive(Clone)]
struct ScalarOp {
    opcode: skiff_artifact_model::Opcode,
    constant: Option<u32>,
    pc: u32,
    before: Vec<u32>,
}

pub(crate) fn loader_backed_scalar() -> ScalarFixture {
    let ops = scalar_ops();
    let bytecode = admitted_bytecode(&ops);
    let package = package_with_caller_summary(
        bytecode.as_ref(),
        LocalCallCandidateCorruption::TargetAnalyzedNoPending,
        effects::analyzed_no_effects(),
    );
    let contract = contract();
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: skiff_artifact_identity::service_contract_ref(&contract).unwrap(),
        deployment_revision: DeploymentRevision::new("revision:scalar-test"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: skiff_artifact_identity::package_artifact_ref(&package).unwrap(),
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "scalar test".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    let package_ref = skiff_artifact_identity::package_artifact_ref(&package).unwrap();
    let resolver = ExactResolver {
        deployment: Arc::new(deployment),
        contract: Arc::new(contract),
        packages: BTreeMap::from([(package_ref.clone(), Arc::new(package))]),
        bytecodes: BTreeMap::from([(package_ref, bytecode)]),
    };
    let hydrated = DeploymentBytecodeLoader::new(&resolver)
        .load(&deployment_ref)
        .unwrap();
    let candidate = candidate(&hydrated, &ops);
    ScalarFixture {
        hydrated,
        candidate,
    }
}

fn admitted_bytecode(ops: &[ScalarOp]) -> Arc<ValidatedBytecodeArtifact> {
    let mut artifact = BytecodeArtifact {
        magic: BYTECODE_MAGIC.to_string(),
        schema_version: BYTECODE_SCHEMA_VERSION.to_string(),
        isa_version: BYTECODE_ISA_VERSION.to_string(),
        opcode_table_fingerprint: opcode_table_fingerprint(),
        native_value_lifecycle_registry:
            skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
        value_lifecycle_policy: skiff_artifact_model::value_lifecycle_policy_identity().clone(),
        host_effect_registry: skiff_artifact_model::host_effect_registry_identity().clone(),
        intrinsic_registry: skiff_artifact_model::intrinsic_registry_identity().clone(),
        platform_error_projection_registry:
            skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
        bytecode_identity: "unassigned".to_string(),
        image: BytecodeImage {
            functions: BTreeMap::from([
                (CALLER_FUNCTION.to_string(), scalar_artifact_function(ops)),
                (TARGET_FUNCTION.to_string(), target_artifact_function()),
            ]),
            pools: BytecodePools {
                constants: constant_pool_entries(),
                types: vec![
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("null"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("bool"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("number"),
                    },
                    BytecodePoolEntry::TypeRef {
                        ty: TypeRefIr::builtin("string"),
                    },
                ],
                shapes: Vec::new(),
                effects: Vec::new(),
                resume: Vec::new(),
                callback_capture: Vec::new(),
                writable_paths: Vec::new(),
            },
            constant_roots: BTreeMap::new(),
            frozen_constant_graph: FrozenConstantGraph {
                nodes: constant_nodes(),
            },
            debug_table: None,
        },
    };
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap())
}

fn scalar_artifact_function(ops: &[ScalarOp]) -> RelocatableBytecodeFunction {
    let words = scalar_words(ops);
    let word_count = u32::try_from(words.len()).unwrap();
    RelocatableBytecodeFunction {
        function_key: CALLER_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(0),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words,
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: Vec::new(),
        },
        max_operand_depth: 2,
        effect_summary_ref: callable(CALLER_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: vec![SourceMapEntry {
            start_pc: 0,
            end_pc: word_count,
            site: scalar_site(),
        }],
    }
}

fn target_artifact_function() -> RelocatableBytecodeFunction {
    RelocatableBytecodeFunction {
        function_key: TARGET_FUNCTION.to_string(),
        origin: BytecodeFunctionOrigin::Executable {
            executable: coordinate(1),
        },
        type_parameters: Vec::new(),
        self_type_ref: None,
        words: vec![descriptor_for_opcode(skiff_artifact_model::Opcode::Return)
            .opcode
            .into()],
        relocations: Vec::new(),
        call_loan_layouts: Vec::new(),
        frame_layout: FrameLayout {
            slot_count: 0,
            slot_type_refs: Vec::new(),
            parameter_slots: Vec::new(),
            writable_local_slots: Vec::new(),
            result_count: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            stream_result_type_ref: None,
            slot_plans: Vec::new(),
        },
        max_operand_depth: 0,
        effect_summary_ref: callable(TARGET_CALLABLE),
        exception_regions: Vec::new(),
        active_regions: Vec::new(),
        switch_tables: Vec::new(),
        statement_entries: Vec::new(),
        source_map: Vec::new(),
    }
}

fn candidate(hydrated: &HydratedDeploymentBytecode, ops: &[ScalarOp]) -> LinkedBytecodeCandidate {
    let package = hydrated.packages().values().next().unwrap();
    let build = package.reference().package_build_id.clone();
    let caller_key = specialization(&build, CALLER_FUNCTION, CALLER_CALLABLE);
    let target_key = specialization(&build, TARGET_FUNCTION, TARGET_CALLABLE);
    let mut parts = candidate_parts(hydrated, None, None);
    parts.types = linked_types(&build);
    parts.frozen_constant_nodes = linked_nodes(&build);
    parts.constants = linked_constants(&build);
    parts.functions = vec![
        scalar_linked_function(FunctionIndex::new(0), caller_key.clone(), ops),
        linked_return_function(FunctionIndex::new(1), target_key.clone()),
    ];
    parts.exact_local_targets = vec![
        LinkedExactLocalTarget::new(caller_key, FunctionIndex::new(0)),
        LinkedExactLocalTarget::new(target_key, FunctionIndex::new(1)),
    ];
    LinkedBytecodeCandidate::try_from_parts(parts).unwrap()
}

fn linked_types(build: &skiff_artifact_model::PackageBuildId) -> Vec<LinkedTypeEntry> {
    ["null", "bool", "number", "string"]
        .into_iter()
        .enumerate()
        .map(|(ordinal, name)| {
            let index = u32::try_from(ordinal).unwrap();
            LinkedTypeEntry::new(
                TypeIndex::new(index),
                origin(build.clone(), ArtifactTypeIndex::new(index)),
                TypeRefIr::builtin(name),
                None,
            )
        })
        .collect()
}

fn linked_nodes(build: &skiff_artifact_model::PackageBuildId) -> Vec<LinkedFrozenConstantNode> {
    constant_literals()
        .into_iter()
        .enumerate()
        .map(|(ordinal, literal)| {
            let index = u32::try_from(ordinal).unwrap();
            LinkedFrozenConstantNode::new(
                FrozenConstantNodeIndex::new(index),
                origin(build.clone(), ArtifactConstantNodeIndex::new(index)),
                LinkedFrozenConstantValue::Literal(literal),
            )
        })
        .collect()
}

fn linked_constants(build: &skiff_artifact_model::PackageBuildId) -> Vec<LinkedConstantEntry> {
    constant_literals()
        .into_iter()
        .enumerate()
        .map(|(ordinal, literal)| {
            let index = u32::try_from(ordinal).unwrap();
            let ty = constant_type_index(ordinal);
            LinkedConstantEntry::new(
                ConstantIndex::new(index),
                origin(build.clone(), ArtifactConstantIndex::new(index)),
                LinkedConstantReference::LocalNode {
                    node: FrozenConstantNodeIndex::new(index),
                },
                TypeIndex::new(ty),
                linked_literal_plan(&literal),
            )
        })
        .collect()
}

fn scalar_linked_function(
    index: FunctionIndex,
    key: skiff_runtime_linked_bytecode::SpecializationKey,
    ops: &[ScalarOp],
) -> LinkedFunction {
    let instructions = ops.iter().map(linked_instruction).collect::<Vec<_>>();
    let states = ops
        .iter()
        .enumerate()
        .map(|(ordinal, op)| {
            LinkedProgramPointState::new(
                InstructionIndex::new(u32::try_from(ordinal).unwrap()),
                op.before
                    .iter()
                    .copied()
                    .map(stack_value)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
                Box::new([]),
                Box::new([]),
                Box::new([]),
            )
        })
        .collect::<Vec<_>>();
    let stack_map =
        LinkedStackMapCandidate::try_new(states.into_boxed_slice(), instructions.len(), 0, 2)
            .unwrap();
    let source_map = Box::new([LinkedSourceMapEntry::new(
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(u32::try_from(instructions.len()).unwrap()),
        scalar_site(),
    )]);
    LinkedFunction::new(
        index,
        key,
        instructions.into_boxed_slice(),
        super::empty_linked_frame(),
        2,
        LinkedCallableEffectDeclaration::new(
            callable(CALLER_CALLABLE),
            effects::analyzed_no_effects(),
        ),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            source_map,
        ),
        stack_map,
    )
}

fn linked_return_function(
    index: FunctionIndex,
    key: skiff_runtime_linked_bytecode::SpecializationKey,
) -> LinkedFunction {
    let instruction = LinkedInstruction::new(
        skiff_artifact_model::Opcode::Return,
        Box::new([]),
        Box::new([]),
        0,
    )
    .unwrap();
    let state = LinkedProgramPointState::new(
        InstructionIndex::new(0),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
    );
    let stack_map = LinkedStackMapCandidate::try_new(Box::new([state]), 1, 0, 0).unwrap();
    LinkedFunction::new(
        index,
        key,
        Box::new([instruction]),
        super::empty_linked_frame(),
        0,
        LinkedCallableEffectDeclaration::new(
            callable(TARGET_CALLABLE),
            effects::analyzed_no_effects(),
        ),
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

fn linked_instruction(op: &ScalarOp) -> LinkedInstruction {
    let (operands, resolved): (Box<[u32]>, Box<[LinkedResolvedOperand]>) = match op.constant {
        None => (Box::from(Vec::new()), Box::from(Vec::new())),
        Some(constant) => (
            Box::from(vec![constant]),
            Box::from(vec![LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::Constant(ConstantIndex::new(constant)),
            )]),
        ),
    };
    LinkedInstruction::new(op.opcode, operands, resolved, op.pc).unwrap()
}

fn scalar_ops() -> Vec<ScalarOp> {
    let mut ops = Vec::new();
    let mut stack = Vec::new();
    let mut pc = 0_u32;

    push_const(&mut ops, &mut stack, &mut pc, 2, 2);
    unary(
        skiff_artifact_model::Opcode::Negate,
        &mut ops,
        &mut stack,
        &mut pc,
        2,
    );
    pop(&mut ops, &mut stack, &mut pc);

    for opcode in [
        skiff_artifact_model::Opcode::Add,
        skiff_artifact_model::Opcode::Subtract,
        skiff_artifact_model::Opcode::Multiply,
        skiff_artifact_model::Opcode::Divide,
    ] {
        push_const(&mut ops, &mut stack, &mut pc, 2, 2);
        push_const(&mut ops, &mut stack, &mut pc, 3, 2);
        binary(opcode, &mut ops, &mut stack, &mut pc, 2);
        pop(&mut ops, &mut stack, &mut pc);
    }

    for opcode in [
        skiff_artifact_model::Opcode::Equal,
        skiff_artifact_model::Opcode::NotEqual,
    ] {
        push_const(&mut ops, &mut stack, &mut pc, 2, 2);
        push_const(&mut ops, &mut stack, &mut pc, 3, 2);
        binary(opcode, &mut ops, &mut stack, &mut pc, 1);
        pop(&mut ops, &mut stack, &mut pc);
    }

    push_const(&mut ops, &mut stack, &mut pc, 1, 1);
    unary(
        skiff_artifact_model::Opcode::Not,
        &mut ops,
        &mut stack,
        &mut pc,
        1,
    );
    pop(&mut ops, &mut stack, &mut pc);

    for opcode in [
        skiff_artifact_model::Opcode::LessThan,
        skiff_artifact_model::Opcode::LessOrEqual,
        skiff_artifact_model::Opcode::GreaterThan,
        skiff_artifact_model::Opcode::GreaterOrEqual,
    ] {
        push_const(&mut ops, &mut stack, &mut pc, 2, 2);
        push_const(&mut ops, &mut stack, &mut pc, 3, 2);
        binary(opcode, &mut ops, &mut stack, &mut pc, 1);
        pop(&mut ops, &mut stack, &mut pc);
    }

    push(
        skiff_artifact_model::Opcode::Return,
        None,
        &mut ops,
        &stack,
        &mut pc,
    );
    ops
}

fn push_const(ops: &mut Vec<ScalarOp>, stack: &mut Vec<u32>, pc: &mut u32, constant: u32, ty: u32) {
    stack.push(ty);
    ops.push(ScalarOp {
        opcode: skiff_artifact_model::Opcode::Const,
        constant: Some(constant),
        pc: *pc,
        before: stack[..stack.len() - 1].to_vec(),
    });
    *pc += 2;
}

fn unary(
    opcode: skiff_artifact_model::Opcode,
    ops: &mut Vec<ScalarOp>,
    stack: &mut Vec<u32>,
    pc: &mut u32,
    output: u32,
) {
    let before = stack.clone();
    stack.pop();
    stack.push(output);
    ops.push(ScalarOp {
        opcode,
        constant: None,
        pc: *pc,
        before,
    });
    *pc += 1;
}

fn binary(
    opcode: skiff_artifact_model::Opcode,
    ops: &mut Vec<ScalarOp>,
    stack: &mut Vec<u32>,
    pc: &mut u32,
    output: u32,
) {
    let before = stack.clone();
    stack.pop();
    stack.pop();
    stack.push(output);
    ops.push(ScalarOp {
        opcode,
        constant: None,
        pc: *pc,
        before,
    });
    *pc += 1;
}

fn pop(ops: &mut Vec<ScalarOp>, stack: &mut Vec<u32>, pc: &mut u32) {
    let before = stack.clone();
    stack.pop();
    ops.push(ScalarOp {
        opcode: skiff_artifact_model::Opcode::Pop,
        constant: None,
        pc: *pc,
        before,
    });
    *pc += 1;
}

fn push(
    opcode: skiff_artifact_model::Opcode,
    constant: Option<u32>,
    ops: &mut Vec<ScalarOp>,
    stack: &[u32],
    pc: &mut u32,
) {
    ops.push(ScalarOp {
        opcode,
        constant,
        pc: *pc,
        before: stack.to_vec(),
    });
    *pc += 1;
}

fn scalar_words(ops: &[ScalarOp]) -> Vec<u32> {
    ops.iter()
        .flat_map(|op| {
            let opcode = u32::from(descriptor_for_opcode(op.opcode).opcode);
            op.constant
                .map_or_else(|| vec![opcode], |constant| vec![opcode, constant])
        })
        .collect()
}

fn constant_pool_entries() -> Vec<BytecodePoolEntry> {
    constant_literals()
        .into_iter()
        .enumerate()
        .map(|(ordinal, literal)| BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode {
                node_index: u32::try_from(ordinal).unwrap(),
            },
            type_ref: constant_type_index(ordinal),
            plan: source_literal_plan(&literal),
        })
        .collect()
}

fn constant_nodes() -> Vec<FrozenConstantNode> {
    constant_literals()
        .into_iter()
        .map(|literal| FrozenConstantNode::Literal { literal })
        .collect()
}

fn constant_literals() -> Vec<LiteralIr> {
    vec![
        LiteralIr::Null,
        LiteralIr::Bool { value: true },
        LiteralIr::Number {
            value: serde_json::Number::from_f64(2.5).unwrap(),
        },
        LiteralIr::Number {
            value: serde_json::Number::from_f64(3.5).unwrap(),
        },
        LiteralIr::String {
            value: "pinned".to_string(),
        },
    ]
}

fn constant_type_index(ordinal: usize) -> u32 {
    match ordinal {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        4 => 3,
        _ => panic!("fixture constant ordinal is out of bounds"),
    }
}

fn source_literal_plan(literal: &LiteralIr) -> ValueTransferPlan {
    let drop = match literal {
        LiteralIr::String { .. } => ValueDropPlan::SnapshotRelease,
        LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            ValueDropPlan::Trivial
        }
    };
    ValueTransferPlan::SnapshotShare { drop }
}

fn linked_literal_plan(literal: &LiteralIr) -> LinkedValueTransferPlan {
    let drop = match literal {
        LiteralIr::String { .. } => LinkedValueDropPlan::SnapshotRelease,
        LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            LinkedValueDropPlan::Trivial
        }
    };
    LinkedValueTransferPlan::SnapshotShare { drop }
}

fn stack_value(ty: u32) -> LinkedStackValue {
    let plan = match ty {
        0..=2 => LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        },
        3 => LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        },
        _ => panic!("fixture stack type is out of bounds"),
    };
    LinkedStackValue::new(TypeIndex::new(ty), plan)
}

fn scalar_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}

fn origin<I>(build: skiff_artifact_model::PackageBuildId, index: I) -> LinkedArtifactPoolOrigin<I> {
    LinkedArtifactPoolOrigin::new(build, index, None).unwrap()
}

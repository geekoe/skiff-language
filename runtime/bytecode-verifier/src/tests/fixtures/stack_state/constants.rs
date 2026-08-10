use skiff_artifact_model::{
    CallableEffectSummary, LiteralIr, Opcode, PackageCallableId, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ConstantIndex, FrozenConstantNodeIndex,
    FunctionIndex, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate, LinkedConstantEntry,
    LinkedConstantReference, LinkedFrozenConstantNode, LinkedFrozenConstantValue,
    LinkedInstruction, LinkedInstructionTarget, LinkedResolvedOperand,
};

use super::{
    candidate_parts, exact_hydration_with_types, linked_function, linked_types, plain, plan_for,
    FunctionSpec, Hint, StackFixture,
};

pub(crate) fn constant_fixture(ty: TypeRefIr) -> StackFixture {
    let types = vec![ty];
    let hydrated = exact_hydration_with_types(types.clone());
    let build = hydrated
        .packages()
        .values()
        .next()
        .expect("constant fixture package is hydrated")
        .reference()
        .package_build_id
        .clone();
    let mut parts = candidate_parts(&hydrated, None, None);
    parts.types = linked_types(&build, &types);
    parts.functions = vec![linked_function(
        FunctionIndex::new(0),
        super::ordinary_key(&build, 0),
        PackageCallableId::new("fixture:constant:0"),
        CallableEffectSummary::analysis_pending(),
        FunctionSpec {
            slots: Vec::new(),
            parameters: Vec::new(),
            writable: Vec::new(),
            results: Vec::new(),
            instructions: vec![
                constant_instruction(),
                plain(Opcode::Pop),
                plain(Opcode::Return),
            ],
            declared_max: 1,
            hints: Some(vec![
                Hint {
                    stack: Vec::new(),
                    slots: Vec::new(),
                },
                Hint {
                    stack: vec![0],
                    slots: Vec::new(),
                },
                Hint {
                    stack: Vec::new(),
                    slots: Vec::new(),
                },
            ]),
        },
        &types,
    )];
    parts.frozen_constant_nodes = vec![LinkedFrozenConstantNode::new(
        FrozenConstantNodeIndex::new(0),
        LinkedArtifactPoolOrigin::new(build.clone(), ArtifactConstantNodeIndex::new(0), None)
            .expect("constant-node origin is valid"),
        LinkedFrozenConstantValue::Literal(LiteralIr::Null),
    )];
    parts.constants = vec![LinkedConstantEntry::new(
        ConstantIndex::new(0),
        LinkedArtifactPoolOrigin::new(build, ArtifactConstantIndex::new(0), None)
            .expect("constant origin is valid"),
        LinkedConstantReference::LocalNode {
            node: FrozenConstantNodeIndex::new(0),
        },
        skiff_runtime_linked_bytecode::TypeIndex::new(0),
        plan_for(&types[0]),
    )];
    StackFixture {
        hydrated,
        candidate: LinkedBytecodeCandidate::try_from_parts(parts)
            .expect("isolated constant fixture passes local validation"),
    }
}

fn constant_instruction() -> LinkedInstruction {
    LinkedInstruction::new(
        Opcode::Const,
        Box::new([0]),
        Box::new([LinkedResolvedOperand::new(
            0,
            LinkedInstructionTarget::Constant(ConstantIndex::new(0)),
        )]),
        0,
    )
    .expect("constant test instruction is valid")
}

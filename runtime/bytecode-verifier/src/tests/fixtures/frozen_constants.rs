use std::collections::BTreeMap;

use skiff_artifact_model::{
    BytecodeConstantRef, BytecodePoolEntry, BytecodePools, FrozenConstantGraph, FrozenConstantNode,
    LiteralIr, TypeRefIr, ValueDropPlan, ValueTransferPlan,
};
use skiff_runtime_linked_bytecode::{
    ArtifactConstantIndex, ArtifactConstantNodeIndex, ArtifactTypeIndex, ConstantIndex,
    FrozenConstantNodeIndex, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate,
    LinkedConstantEntry, LinkedConstantReference, LinkedFrozenConstantNode,
    LinkedFrozenConstantValue, LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    TypeIndex,
};
use skiff_runtime_loader::HydratedDeploymentBytecode;

#[derive(Debug, Clone, Copy)]
pub(crate) enum FrozenAuthorityPresence {
    Empty,
    NonEmpty,
}

pub(crate) struct FrozenConstantFixture {
    pub(crate) hydrated: HydratedDeploymentBytecode,
    pub(crate) candidate: LinkedBytecodeCandidate,
}

pub(crate) fn fixture(
    source: FrozenAuthorityPresence,
    candidate: FrozenAuthorityPresence,
) -> FrozenConstantFixture {
    let hydrated = match source {
        FrozenAuthorityPresence::Empty => super::exact_hydration(),
        FrozenAuthorityPresence::NonEmpty => nonempty_hydration(),
    };
    let candidate = candidate_with_presence(&hydrated, candidate);
    FrozenConstantFixture {
        hydrated,
        candidate,
    }
}

fn nonempty_hydration() -> HydratedDeploymentBytecode {
    let pools = BytecodePools {
        constants: vec![BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index: 0 },
            type_ref: 0,
            plan: ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            },
        }],
        types: vec![BytecodePoolEntry::TypeRef {
            ty: TypeRefIr::builtin("null"),
        }],
        ..BytecodePools::default()
    };
    let graph = FrozenConstantGraph {
        nodes: vec![FrozenConstantNode::Literal {
            literal: LiteralIr::Null,
        }],
    };
    super::hydrate_bytecode(super::admit_bytecode(pools, BTreeMap::new(), graph))
}

fn candidate_with_presence(
    hydrated: &HydratedDeploymentBytecode,
    presence: FrozenAuthorityPresence,
) -> LinkedBytecodeCandidate {
    let mut parts = super::candidate_parts(hydrated, None, None);
    if matches!(presence, FrozenAuthorityPresence::NonEmpty) {
        let build = hydrated
            .packages()
            .values()
            .next()
            .expect("frozen-constant fixture package is hydrated")
            .reference()
            .package_build_id
            .clone();
        parts.types = vec![LinkedTypeEntry::new(
            TypeIndex::new(0),
            LinkedArtifactPoolOrigin::new(build.clone(), ArtifactTypeIndex::new(0), None)
                .expect("frozen-constant type origin is package-global"),
            TypeRefIr::builtin("null"),
            None,
        )];
        parts.frozen_constant_nodes = vec![LinkedFrozenConstantNode::new(
            FrozenConstantNodeIndex::new(0),
            LinkedArtifactPoolOrigin::new(build.clone(), ArtifactConstantNodeIndex::new(0), None)
                .expect("frozen-constant node origin is package-global"),
            LinkedFrozenConstantValue::Literal(LiteralIr::Null),
        )];
        parts.constants = vec![LinkedConstantEntry::new(
            ConstantIndex::new(0),
            LinkedArtifactPoolOrigin::new(build, ArtifactConstantIndex::new(0), None)
                .expect("frozen-constant origin is package-global"),
            LinkedConstantReference::LocalNode {
                node: FrozenConstantNodeIndex::new(0),
            },
            TypeIndex::new(0),
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            },
        )];
    }
    LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("frozen-constant fixture passes candidate-local validation")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenLiteralKind {
    Null,
    Bool,
    Number,
    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConstantCorruption {
    None,
    AggregateNode,
    MissingNodeOrigin,
    MissingTypeOrigin,
    WrongPlan,
    TypeMismatch,
}

pub(crate) fn literal_fixture(
    kind: FrozenLiteralKind,
    corruption: ConstantCorruption,
) -> FrozenConstantFixture {
    let (literal, ty) = literal_parts(kind);
    let hydrated = literal_hydration(&literal, &ty);
    let build = hydrated
        .packages()
        .values()
        .next()
        .expect("literal fixture package is hydrated")
        .reference()
        .package_build_id
        .clone();
    let mut parts = super::candidate_parts(&hydrated, None, None);
    let type_origin = if corruption == ConstantCorruption::MissingTypeOrigin {
        1
    } else {
        0
    };
    let node_origin = if corruption == ConstantCorruption::MissingNodeOrigin {
        1
    } else {
        0
    };
    let type_ref = if corruption == ConstantCorruption::TypeMismatch {
        TypeRefIr::builtin("bool")
    } else {
        ty
    };
    parts.types = vec![LinkedTypeEntry::new(
        TypeIndex::new(0),
        origin(build.clone(), ArtifactTypeIndex::new(type_origin)),
        type_ref,
        None,
    )];
    let node_value = if corruption == ConstantCorruption::AggregateNode {
        LinkedFrozenConstantValue::Array {
            children: Box::new([]),
        }
    } else {
        LinkedFrozenConstantValue::Literal(literal.clone())
    };
    parts.frozen_constant_nodes = vec![LinkedFrozenConstantNode::new(
        FrozenConstantNodeIndex::new(0),
        origin(build.clone(), ArtifactConstantNodeIndex::new(node_origin)),
        node_value,
    )];
    let plan = if corruption == ConstantCorruption::WrongPlan {
        corrupted_linked_plan(kind)
    } else {
        linked_literal_plan(kind)
    };
    parts.constants = vec![LinkedConstantEntry::new(
        ConstantIndex::new(0),
        origin(build, ArtifactConstantIndex::new(0)),
        LinkedConstantReference::LocalNode {
            node: FrozenConstantNodeIndex::new(0),
        },
        TypeIndex::new(0),
        plan,
    )];
    FrozenConstantFixture {
        hydrated,
        candidate: LinkedBytecodeCandidate::try_from_parts(parts)
            .expect("literal fixture passes candidate-local validation"),
    }
}

fn literal_hydration(literal: &LiteralIr, ty: &TypeRefIr) -> HydratedDeploymentBytecode {
    let pools = BytecodePools {
        constants: vec![BytecodePoolEntry::ConstantRef {
            reference: BytecodeConstantRef::LocalNode { node_index: 0 },
            type_ref: 0,
            plan: source_literal_plan(literal),
        }],
        types: vec![BytecodePoolEntry::TypeRef { ty: ty.clone() }],
        ..BytecodePools::default()
    };
    let graph = FrozenConstantGraph {
        nodes: vec![FrozenConstantNode::Literal {
            literal: literal.clone(),
        }],
    };
    super::hydrate_bytecode(super::admit_bytecode(pools, BTreeMap::new(), graph))
}

fn literal_parts(kind: FrozenLiteralKind) -> (LiteralIr, TypeRefIr) {
    match kind {
        FrozenLiteralKind::Null => (LiteralIr::Null, TypeRefIr::builtin("null")),
        FrozenLiteralKind::Bool => (LiteralIr::Bool { value: true }, TypeRefIr::builtin("bool")),
        FrozenLiteralKind::Number => (
            LiteralIr::Number {
                value: serde_json::Number::from_f64(2.5).expect("fixture number is finite"),
            },
            TypeRefIr::builtin("number"),
        ),
        FrozenLiteralKind::String => (
            LiteralIr::String {
                value: "pinned".to_string(),
            },
            TypeRefIr::builtin("string"),
        ),
    }
}

fn source_literal_plan(literal: &LiteralIr) -> ValueTransferPlan {
    match literal {
        LiteralIr::String { .. } => ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::SnapshotRelease,
        },
        LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. } => {
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }
        }
    }
}

fn linked_literal_plan(kind: FrozenLiteralKind) -> LinkedValueTransferPlan {
    let drop = match kind {
        FrozenLiteralKind::String => LinkedValueDropPlan::SnapshotRelease,
        FrozenLiteralKind::Null | FrozenLiteralKind::Bool | FrozenLiteralKind::Number => {
            LinkedValueDropPlan::Trivial
        }
    };
    LinkedValueTransferPlan::SnapshotShare { drop }
}

fn corrupted_linked_plan(kind: FrozenLiteralKind) -> LinkedValueTransferPlan {
    let drop = match kind {
        FrozenLiteralKind::String => LinkedValueDropPlan::Trivial,
        FrozenLiteralKind::Null | FrozenLiteralKind::Bool | FrozenLiteralKind::Number => {
            LinkedValueDropPlan::SnapshotRelease
        }
    };
    LinkedValueTransferPlan::SnapshotShare { drop }
}

fn origin<I>(build: skiff_artifact_model::PackageBuildId, index: I) -> LinkedArtifactPoolOrigin<I> {
    LinkedArtifactPoolOrigin::new(build, index, None).expect("fixture artifact origin is valid")
}

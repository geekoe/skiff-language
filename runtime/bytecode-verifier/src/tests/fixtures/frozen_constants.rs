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

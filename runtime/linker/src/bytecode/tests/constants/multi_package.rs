use skiff_artifact_model::{LiteralIr, Opcode, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    LinkedConstantReference, LinkedFrozenConstantValue, LinkedInstructionTarget,
    LinkedValueDropPlan, LinkedValueTransferPlan,
};

use crate::bytecode::{link_deployment, BytecodeLinkError, BytecodeLinkLimit};

use super::super::{
    fixtures::{ConstantProgram, Fixture, DEPENDENCY_PACKAGE_ID, ROOT_FUNCTION},
    generous_limits,
};

#[test]
fn two_packages_rebase_local_zero_rows_and_relink_deterministically() {
    let fixture = Fixture::two_package_constants(ConstantProgram::Number, ConstantProgram::Bool);
    let hydrated = fixture.hydrate();
    let first = link_deployment(&hydrated, &generous_limits()).unwrap();
    let second = link_deployment(&hydrated, &generous_limits()).unwrap();

    assert_eq!(first.packages(), second.packages());
    assert_eq!(first.functions(), second.functions());
    assert_eq!(first.types(), second.types());
    assert_eq!(first.constants(), second.constants());
    assert_eq!(first.constant_roots(), second.constant_roots());
    assert_eq!(
        first.frozen_constant_nodes(),
        second.frozen_constant_nodes()
    );
    assert_eq!(first.packages().len(), 2);
    assert_eq!(first.packages().len(), hydrated.packages().len());
    for provenance in first.packages() {
        let package = hydrated
            .packages()
            .get(provenance.package_build_id())
            .expect("linked package provenance must retain an exact hydrated owner");
        assert_eq!(
            package.platform_error_projection_registry(),
            hydrated.platform_error_projection_registry()
        );
        assert_eq!(
            provenance
                .authorities()
                .platform_error_projection_registry(),
            package.platform_error_projection_registry()
        );
    }
    assert_eq!(first.constants().len(), 2);
    assert_eq!(first.constant_roots().len(), 2);
    assert_eq!(first.frozen_constant_nodes().len(), 2);

    let dependency = hydrated
        .packages()
        .values()
        .find(|package| package.reference().package_id == DEPENDENCY_PACKAGE_ID)
        .unwrap();
    let primary_build = &fixture.package_reference.package_build_id;
    let dependency_build = &dependency.reference().package_build_id;
    let primary = constant_for(&first, primary_build);
    let dependency_constant = constant_for(&first, dependency_build);
    assert_ne!(primary.index(), dependency_constant.index());
    assert_eq!(first.constants()[1].index().get(), 1);
    assert_eq!(first.constants()[1].origin().artifact_index().get(), 0);
    assert!(first.constants()[1].origin().specialization().is_none());

    assert_constant_authority(
        &first,
        primary,
        primary_build,
        &LiteralIr::Number {
            value: serde_json::Number::from(42),
        },
        &TypeRefIr::builtin("number"),
        &LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        },
    );
    assert_constant_authority(
        &first,
        dependency_constant,
        dependency_build,
        &LiteralIr::Bool { value: true },
        &TypeRefIr::builtin("bool"),
        &LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        },
    );

    let root_function = first
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == ROOT_FUNCTION)
        .unwrap();
    assert_eq!(root_function.instructions()[0].opcode(), Opcode::Const);
    assert_eq!(
        root_function.instructions()[0].resolved_operands()[0].target(),
        LinkedInstructionTarget::Constant(primary.index())
    );
    let stack_value = &root_function.stack_map().entries()[1].stack_before()[0];
    assert_eq!(stack_value.ty(), primary.ty());
    assert_eq!(stack_value.plan(), primary.plan());
}

#[test]
fn graph_limits_sum_nodes_and_edges_across_packages() {
    let nodes = Fixture::two_package_constants(ConstantProgram::Number, ConstantProgram::Number);
    let nodes_hydrated = nodes.hydrate();
    let mut node_limits = generous_limits();
    node_limits.max_constant_graph_nodes = 1;
    assert!(matches!(
        link_deployment(&nodes_hydrated, &node_limits),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ConstantGraphNodes,
            actual: 2,
            max: 1,
            ..
        })
    ));

    let edges = Fixture::two_package_constants(ConstantProgram::Array, ConstantProgram::Array);
    let edges_hydrated = edges.hydrate();
    let mut edge_limits = generous_limits();
    edge_limits.max_constant_graph_edges = 1;
    assert!(matches!(
        link_deployment(&edges_hydrated, &edge_limits),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ConstantGraphEdges,
            actual: 2,
            max: 1,
            ..
        })
    ));
}

#[test]
fn all_constant_candidate_tables_participate_in_image_table_limits() {
    let literals = Fixture::two_package_constants(ConstantProgram::Number, ConstantProgram::Number);
    let literals_hydrated = literals.hydrate();

    let mut constants_limit = generous_limits();
    constants_limit.max_image_table_entries = 1;
    assert_image_limit(&literals_hydrated, &constants_limit, 2, false);

    let mut roots_total = generous_limits();
    roots_total.max_total_image_table_entries = 3;
    assert_image_limit(&literals_hydrated, &roots_total, 4, true);

    let mut nodes_total = generous_limits();
    nodes_total.max_total_image_table_entries = 5;
    assert_image_limit(&literals_hydrated, &nodes_total, 6, true);

    let arrays = Fixture::two_package_constants(ConstantProgram::Array, ConstantProgram::Array);
    let arrays_hydrated = arrays.hydrate();
    let mut node_table = generous_limits();
    node_table.max_image_table_entries = 3;
    assert_image_limit(&arrays_hydrated, &node_table, 4, false);
}

fn constant_for<'a>(
    candidate: &'a skiff_runtime_linked_bytecode::LinkedBytecodeCandidate,
    build: &skiff_artifact_model::PackageBuildId,
) -> &'a skiff_runtime_linked_bytecode::LinkedConstantEntry {
    candidate
        .constants()
        .iter()
        .find(|constant| constant.origin().package_build_id() == build)
        .unwrap()
}

fn assert_constant_authority(
    candidate: &skiff_runtime_linked_bytecode::LinkedBytecodeCandidate,
    constant: &skiff_runtime_linked_bytecode::LinkedConstantEntry,
    build: &skiff_artifact_model::PackageBuildId,
    literal: &LiteralIr,
    expected_type: &TypeRefIr,
    expected_plan: &LinkedValueTransferPlan,
) {
    assert_eq!(constant.origin().package_build_id(), build);
    assert_eq!(constant.origin().artifact_index().get(), 0);
    assert!(constant.origin().specialization().is_none());
    assert_eq!(constant.plan(), expected_plan);
    let LinkedConstantReference::LocalNode { node } = constant.reference() else {
        panic!("literal-only linked constant must retain a local node")
    };
    let node_position = usize::try_from(node.get()).unwrap();
    let linked_node = &candidate.frozen_constant_nodes()[node_position];
    assert_eq!(linked_node.index(), *node);
    assert_eq!(linked_node.origin().package_build_id(), build);
    assert_eq!(linked_node.origin().artifact_index().get(), 0);
    assert!(linked_node.origin().specialization().is_none());
    assert!(matches!(
        linked_node.value(),
        LinkedFrozenConstantValue::Literal(actual) if actual == literal
    ));

    let type_position = usize::try_from(constant.ty().get()).unwrap();
    let linked_type = &candidate.types()[type_position];
    assert_eq!(linked_type.type_ref(), expected_type);
    assert_eq!(linked_type.origin().package_build_id(), build);
    assert_eq!(linked_type.origin().artifact_index().get(), 0);
    assert!(linked_type.origin().specialization().is_none());
    let root = candidate
        .constant_roots()
        .iter()
        .find(|root| root.owner_package_build_id() == build)
        .unwrap();
    assert_eq!(root.symbol_path().as_str(), "fixture.answer");
    assert_eq!(root.constant(), constant.index());
}

fn assert_image_limit(
    hydrated: &skiff_runtime_loader::HydratedDeploymentBytecode,
    limits: &crate::bytecode::LinkLimits,
    actual: u64,
    total: bool,
) {
    let expected = if total {
        BytecodeLinkLimit::TotalImageTableEntries
    } else {
        BytecodeLinkLimit::ImageTableEntries
    };
    assert!(matches!(
        link_deployment(hydrated, limits),
        Err(BytecodeLinkError::LimitExceeded {
            limit,
            actual: observed,
            ..
        }) if limit == expected && observed == actual
    ));
}

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{
    link_deployment, BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation,
    BytecodeLinkObligation, LinkLimits,
};

#[test]
fn entry_contract_borrows_exact_hydration_and_returns_only_a_candidate() {
    let entry: fn(
        &HydratedDeploymentBytecode,
        &LinkLimits,
    ) -> Result<LinkedBytecodeCandidate, BytecodeLinkError> = link_deployment;

    let _ = entry;
}

#[test]
fn link_limits_are_a_complete_explicit_policy_value() {
    let limits = LinkLimits {
        max_packages: 1,
        max_root_specializations: 2,
        max_specializations: 3,
        max_code_words_per_function: 4,
        max_total_code_words: 5,
        max_relocations_per_function: 6,
        max_total_relocations: 7,
        max_image_table_entries: 8,
        max_total_image_table_entries: 9,
        max_total_function_table_entries: 10,
        max_type_nesting_depth: 11,
        max_constant_graph_nodes: 12,
        max_constant_graph_edges: 13,
    };

    assert_eq!(limits.max_packages, 1);
    assert_eq!(limits.max_root_specializations, 2);
    assert_eq!(limits.max_specializations, 3);
    assert_eq!(limits.max_code_words_per_function, 4);
    assert_eq!(limits.max_total_code_words, 5);
    assert_eq!(limits.max_relocations_per_function, 6);
    assert_eq!(limits.max_total_relocations, 7);
    assert_eq!(limits.max_image_table_entries, 8);
    assert_eq!(limits.max_total_image_table_entries, 9);
    assert_eq!(limits.max_total_function_table_entries, 10);
    assert_eq!(limits.max_type_nesting_depth, 11);
    assert_eq!(limits.max_constant_graph_nodes, 12);
    assert_eq!(limits.max_constant_graph_edges, 13);
}

#[test]
fn unavailable_work_is_an_explicit_obligation_at_a_typed_location() {
    let location = BytecodeLinkLocation::Constant {
        package: skiff_artifact_model::PackageArtifactRef {
            package_id: "example.package".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: skiff_artifact_model::PackageBuildId::new("build:example"),
            package_local_abi_identity: skiff_artifact_model::PackageLocalAbiIdentity::new(
                "abi:example",
            ),
        },
        node_index: 7,
    };
    let error = BytecodeLinkError::ImplementationUnavailable {
        obligation: BytecodeLinkObligation::ConstantInitializationPlan,
        location: location.clone(),
    };

    assert_eq!(
        error,
        BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ConstantInitializationPlan,
            location,
        }
    );
    assert_eq!(
        error.to_string(),
        "bytecode constant initialization plan linking is unavailable at package build:example constant node 7"
    );
    assert_eq!(BytecodeLinkLimit::Specializations.name(), "specializations");
}

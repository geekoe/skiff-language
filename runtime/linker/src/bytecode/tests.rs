use std::collections::BTreeMap;

use skiff_artifact_model::{TypeRefIr, ValueTransferPlan};
use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::types::{substitute_type, TypeLinker};
use super::worklist::CanonicalWorklist;
use super::{
    limits::LinkLimitTracker, link_deployment, BytecodeLinkError, BytecodeLinkLimit,
    BytecodeLinkLocation, BytecodeLinkObligation, LinkLimits,
};

mod deployment;
mod fixtures;

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
        max_expanded_type_nodes: 12,
        max_expanded_type_bytes: 13,
        max_constant_graph_nodes: 14,
        max_constant_graph_edges: 15,
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
    assert_eq!(limits.max_expanded_type_nodes, 12);
    assert_eq!(limits.max_expanded_type_bytes, 13);
    assert_eq!(limits.max_constant_graph_nodes, 14);
    assert_eq!(limits.max_constant_graph_edges, 15);
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

fn generous_limits() -> LinkLimits {
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

fn deployment_location() -> BytecodeLinkLocation {
    BytecodeLinkLocation::Deployment {
        deployment: skiff_artifact_model::ServiceDeploymentRef {
            service_id: "example.service".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: skiff_artifact_model::DeploymentRevision::new("revision:one"),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                "deployment:one",
            ),
        },
    }
}

#[test]
fn canonical_worklist_sorts_roots_and_call_site_discoveries() {
    let limits = generous_limits();
    let location = deployment_location();
    let mut worklist = CanonicalWorklist::try_from_roots(
        ["root:z", "root:a", "root:z"],
        &limits,
        location.clone(),
    )
    .unwrap();

    assert_eq!(worklist.canonical_keys().count(), 2);
    assert_eq!(worklist.pop_next().unwrap().1, "root:a");
    worklist
        .enqueue_discovered(
            [(9, "call:z"), (3, "call:m"), (3, "call:a"), (1, "root:a")],
            &limits,
            location,
        )
        .unwrap();

    assert_eq!(worklist.pop_next().unwrap().1, "root:z");
    assert_eq!(worklist.pop_next().unwrap().1, "call:a");
    assert_eq!(worklist.pop_next().unwrap().1, "call:m");
    assert_eq!(worklist.pop_next().unwrap().1, "call:z");
    assert!(worklist.pop_next().is_none());
    assert_eq!(
        worklist.canonical_keys().copied().collect::<Vec<_>>(),
        vec!["call:a", "call:m", "call:z", "root:a", "root:z"]
    );
}

#[test]
fn expanded_type_limits_are_aggregate_and_checked() {
    let mut limits = generous_limits();
    limits.max_expanded_type_nodes = 3;
    limits.max_expanded_type_bytes = 9;
    let location = deployment_location();
    let mut tracker = LinkLimitTracker::new(&limits);

    tracker.add_expanded_type(2, 4, location.clone()).unwrap();
    assert!(matches!(
        tracker.add_expanded_type(2, 4, location),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ExpandedTypeNodes,
            actual: 4,
            max: 3,
            ..
        })
    ));

    limits.max_expanded_type_nodes = u64::MAX;
    limits.max_expanded_type_bytes = 7;
    let location = deployment_location();
    let mut tracker = LinkLimitTracker::new(&limits);
    tracker.add_expanded_type(1, 4, location.clone()).unwrap();
    assert!(matches!(
        tracker.add_expanded_type(1, 4, location),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::ExpandedTypeBytes,
            actual: 8,
            max: 7,
            ..
        })
    ));
}

#[test]
fn type_substitution_is_recursive_and_never_leaves_an_unknown_parameter() {
    let location = deployment_location();
    let substitutions = BTreeMap::from([(
        "T".to_string(),
        TypeRefIr::Builtin {
            name: "string".to_string(),
            args: Vec::new(),
        },
    )]);
    let template = TypeRefIr::Nullable {
        inner: Box::new(TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::TypeParam {
                name: "T".to_string(),
            }],
        }),
    };

    assert_eq!(
        substitute_type(&template, &substitutions, &location).unwrap(),
        TypeRefIr::Nullable {
            inner: Box::new(TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            }),
        }
    );
    assert!(matches!(
        substitute_type(&template, &BTreeMap::new(), &location),
        Err(BytecodeLinkError::UnsatisfiedObligation {
            obligation: BytecodeLinkObligation::ConcreteSpecialization,
            ..
        })
    ));
}

#[test]
fn from_type_transfer_plans_remain_fail_closed() {
    let limits = generous_limits();
    let linker = TypeLinker::new(&limits);
    let location = deployment_location();

    assert!(matches!(
        linker.link_transfer_plan(
            &ValueTransferPlan::FromType {
                ty: TypeRefIr::builtin("string"),
            },
            &BTreeMap::new(),
            location,
        ),
        Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::FrameAndValueTransferPlan,
            ..
        })
    ));
}

#[test]
fn canonical_worklist_reuses_recursive_specializations() {
    let limits = generous_limits();
    let location = deployment_location();
    let mut worklist =
        CanonicalWorklist::try_from_roots(["recursive"], &limits, location.clone()).unwrap();
    let original = worklist.index_of(&"recursive").unwrap();

    worklist
        .enqueue_discovered([(7, "recursive")], &limits, location)
        .unwrap();

    assert_eq!(worklist.canonical_keys().count(), 1);
    assert_eq!(worklist.index_of(&"recursive"), Some(original));
    assert_eq!(worklist.pop_next(), Some((original, "recursive")));
    assert!(worklist.pop_next().is_none());
}

#[test]
fn canonical_worklist_enforces_root_and_expansion_limits() {
    let mut limits = generous_limits();
    limits.max_root_specializations = 1;
    let location = deployment_location();
    assert!(matches!(
        CanonicalWorklist::try_from_roots(["a", "b"], &limits, location.clone()),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::RootSpecializations,
            actual: 2,
            max: 1,
            ..
        })
    ));

    limits.max_root_specializations = 2;
    limits.max_specializations = 2;
    let mut worklist =
        CanonicalWorklist::try_from_roots(["a", "b"], &limits, location.clone()).unwrap();
    assert!(matches!(
        worklist.enqueue_discovered([(0, "c")], &limits, location),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::Specializations,
            actual: 3,
            max: 2,
            ..
        })
    ));
}

#[test]
fn limit_tracker_checks_per_function_and_aggregate_totals() {
    let mut limits = generous_limits();
    limits.max_code_words_per_function = 5;
    limits.max_total_code_words = 8;
    limits.max_relocations_per_function = 3;
    limits.max_total_relocations = 4;
    limits.max_total_function_table_entries = 6;
    let location = deployment_location();
    let mut tracker = LinkLimitTracker::new(&limits);

    tracker.add_function(4, 2, 3, location.clone()).unwrap();
    assert!(matches!(
        tracker.add_function(5, 2, 3, location),
        Err(BytecodeLinkError::LimitExceeded {
            limit: BytecodeLinkLimit::TotalCodeWords,
            actual: 9,
            max: 8,
            ..
        })
    ));
}

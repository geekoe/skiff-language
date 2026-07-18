use std::collections::BTreeMap;

use skiff_artifact_identity::assign_package_artifact_identities;
use skiff_artifact_model::{
    PackageBuildId, PackageCallableId, PackageCallableRef, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn used_std_adds_exact_requirement_from_validated_canonical_artifact() {
    let std_artifact = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "7.4.2");
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            package_callable_id: PackageCallableId::new("callable:clock.now"),
        });

    let requirements = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        &[file],
        std::slice::from_ref(&std_artifact),
    )
    .unwrap();

    assert_eq!(requirements.len(), 1);
    assert_eq!(requirements[0].alias, "std");
    assert_eq!(requirements[0].package_id, SKIFF_STD_PUBLICATION_ID);
    assert_eq!(requirements[0].exact_version, "7.4.2");
    assert_eq!(
        requirements[0].expected_local_abi,
        std_artifact.package_local_abi.local_abi_identity
    );
}

#[test]
fn package_callable_ref_requires_std_but_unused_and_std_self_do_not() {
    let std_artifact = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "2.0.0");
    let mut callable_file = FileIrUnit::empty("main", "source");
    callable_file
        .external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            package_callable_id: PackageCallableId::new("callable:std.task.run"),
        });
    assert_eq!(
        complete_package_requirement_closure(
            "example.com/app",
            Vec::new(),
            std::slice::from_ref(&callable_file),
            std::slice::from_ref(&std_artifact),
        )
        .unwrap()
        .len(),
        1
    );

    let unused = FileIrUnit::empty("main", "source");
    assert!(
        complete_package_requirement_closure("example.com/app", Vec::new(), &[unused], &[],)
            .unwrap()
            .is_empty()
    );
    assert!(complete_package_requirement_closure(
        SKIFF_STD_PUBLICATION_ID,
        Vec::new(),
        &[callable_file],
        &[],
    )
    .unwrap()
    .is_empty());
}

#[test]
fn package_callable_id_does_not_infer_std_requirement() {
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: PackageRefIr::Dependency {
                dependency_ref: "tools".to_string(),
            },
            package_callable_id: PackageCallableId::new("callable:std.task.run"),
        });

    assert!(
        complete_package_requirement_closure("example.com/app", Vec::new(), &[file], &[],)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn used_std_fails_closed_without_one_valid_same_round_artifact() {
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            package_callable_id: PackageCallableId::new("callable:clock.now"),
        });
    let missing = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        std::slice::from_ref(&file),
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(missing.contains("same compile graph"), "{missing}");

    let mut invalid_identity = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "1.0.0");
    invalid_identity.package_build_id = PackageBuildId::new("tampered");
    let identity_error = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        std::slice::from_ref(&file),
        std::slice::from_ref(&invalid_identity),
    )
    .unwrap_err()
    .to_string();
    assert!(
        identity_error.contains("identity validation failed"),
        "{identity_error}"
    );

    let mut invalid_version = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "1.0.0");
    invalid_version.package_version = "9.9.9".to_string();
    let version_error = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        &[file],
        std::slice::from_ref(&invalid_version),
    )
    .unwrap_err()
    .to_string();
    assert!(
        version_error.contains("identity validation failed"),
        "{version_error}"
    );
}

fn canonical_artifact(package_id: &str, version: &str) -> PackageArtifact {
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: version.to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
        },
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

use std::collections::BTreeMap;

use skiff_artifact_identity::{assign_package_artifact_identities, package_schema_index_identity};
use skiff_artifact_model::{
    PackageBuildId, PackageCallableId, PackageCallableRef, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndex,
    PackageSchemaIndexRef, PackageSymbolRef, PACKAGE_ARTIFACT_SCHEMA_VERSION,
};

use super::*;

#[cfg(unix)]
mod p5_f18a;

#[test]
fn type_only_std_reference_adds_exact_requirement_from_validated_canonical_artifact() {
    let std_artifact = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "7.4.2");
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs.package_symbols.push(PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
        },
        symbol_path: "time.Instant".to_string(),
        abi_expectation: None,
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
fn callable_only_std_reference_adds_exact_requirement() {
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
    let requirements = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        std::slice::from_ref(&callable_file),
        std::slice::from_ref(&std_artifact),
    )
    .unwrap();

    assert_eq!(requirements.len(), 1);
    assert_eq!(requirements[0].alias, "std");
    assert_eq!(requirements[0].package_id, SKIFF_STD_PUBLICATION_ID);
    assert_eq!(requirements[0].exact_version, "2.0.0");
    assert_eq!(
        requirements[0].expected_local_abi,
        std_artifact.package_local_abi.local_abi_identity
    );
}

#[test]
fn native_signature_package_type_adds_exact_std_requirement() {
    let std_artifact = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "2.1.0");
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs.native_targets.push(NativeTarget {
        namespace: "std.time".to_string(),
        symbol: "sleep".to_string(),
        binding_key: Some("std.time.sleep".to_string()),
        metadata: BTreeMap::new(),
    });

    let requirements = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        std::slice::from_ref(&file),
        std::slice::from_ref(&std_artifact),
    )
    .unwrap();

    assert_eq!(requirements.len(), 1);
    assert_eq!(requirements[0].alias, "std");
    assert_eq!(requirements[0].package_id, SKIFF_STD_PUBLICATION_ID);
    assert_eq!(requirements[0].exact_version, "2.1.0");
    assert_eq!(
        requirements[0].expected_local_abi,
        std_artifact.package_local_abi.local_abi_identity
    );
}

#[test]
fn native_signature_without_package_types_does_not_add_std_requirement() {
    let mut file = FileIrUnit::empty("main", "source");
    file.external_refs.native_targets.push(NativeTarget {
        namespace: "std.crypto".to_string(),
        symbol: "uuid".to_string(),
        binding_key: Some("std.crypto.uuid".to_string()),
        metadata: BTreeMap::new(),
    });

    assert!(
        complete_package_requirement_closure("example.com/app", Vec::new(), &[file], &[])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unused_std_and_std_self_do_not_add_requirements() {
    let unused = FileIrUnit::empty("main", "source");
    assert!(
        complete_package_requirement_closure("example.com/app", Vec::new(), &[unused], &[],)
            .unwrap()
            .is_empty()
    );

    let mut std_self_reference = FileIrUnit::empty("main", "source");
    std_self_reference
        .external_refs
        .package_symbols
        .push(PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            symbol_path: "time.Instant".to_string(),
            abi_expectation: None,
        });
    assert!(complete_package_requirement_closure(
        SKIFF_STD_PUBLICATION_ID,
        Vec::new(),
        &[std_self_reference],
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
    let requirements = complete_package_requirement_closure(
        "example.com/app",
        Vec::new(),
        &[file],
        std::slice::from_ref(&invalid_version),
    )
    .expect("human version relabeling does not invalidate immutable identity");
    assert_eq!(requirements[0].exact_version, "9.9.9");
}

#[test]
fn exact_package_schema_batch_rejects_missing_version_build_and_abi_mismatch() {
    let artifact = canonical_artifact("example.types", "1.0.0");
    let requirement = PackageRequirement {
        alias: "types".to_string(),
        package_id: artifact.package_id.clone(),
        exact_version: artifact.package_version.clone(),
        expected_local_abi: artifact.package_local_abi.local_abi_identity.clone(),
        collection_name_mapping: BTreeMap::new(),
        expected_package_build: None,
    };
    let schema = ResolvedPackageSchema::new(
        requirement.alias.clone(),
        artifact.package_id.clone(),
        artifact.package_version.clone(),
        artifact.package_build_id.clone(),
        artifact.package_local_abi.local_abi_identity.clone(),
        PackageSchemaIndex {
            package_id: artifact.package_id.clone(),
            package_schema_index_identity: artifact
                .package_schema_index
                .package_schema_index_identity
                .clone(),
            types: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();

    assert!(exact_resolved_package_schemas(
        std::slice::from_ref(&requirement),
        std::slice::from_ref(&artifact),
        &[],
        None,
    )
    .unwrap_err()
    .to_string()
    .contains("no resolved schema"));

    assert!(exact_resolved_package_schemas(
        std::slice::from_ref(&requirement),
        std::slice::from_ref(&artifact),
        std::slice::from_ref(&schema),
        None,
    )
    .is_ok());

    let wrong_version = ResolvedPackageSchema::new(
        "types".to_string(),
        artifact.package_id.clone(),
        "2.0.0".to_string(),
        artifact.package_build_id.clone(),
        artifact.package_local_abi.local_abi_identity.clone(),
        PackageSchemaIndex {
            package_id: artifact.package_id.clone(),
            package_schema_index_identity: artifact
                .package_schema_index
                .package_schema_index_identity
                .clone(),
            types: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    assert!(exact_resolved_package_schemas(
        std::slice::from_ref(&requirement),
        std::slice::from_ref(&artifact),
        &[wrong_version],
        None,
    )
    .is_err());

    let wrong_build = ResolvedPackageSchema::new(
        "types".to_string(),
        artifact.package_id.clone(),
        artifact.package_version.clone(),
        PackageBuildId::new("wrong-build"),
        artifact.package_local_abi.local_abi_identity.clone(),
        PackageSchemaIndex {
            package_id: artifact.package_id.clone(),
            package_schema_index_identity: artifact
                .package_schema_index
                .package_schema_index_identity
                .clone(),
            types: BTreeMap::new(),
        },
        BTreeMap::new(),
    )
    .unwrap();
    assert!(exact_resolved_package_schemas(
        std::slice::from_ref(&requirement),
        std::slice::from_ref(&artifact),
        &[wrong_build],
        None,
    )
    .is_err());

    let wrong_abi_requirement = PackageRequirement {
        expected_local_abi: PackageLocalAbiIdentity::new("wrong-abi"),
        ..requirement
    };
    assert!(
        exact_resolved_package_schemas(&[wrong_abi_requirement], &[artifact], &[], None,).is_err()
    );
}

#[test]
fn authored_dependency_collection_mapping_reaches_compile_requirement_exactly() {
    let dependency_artifact = canonical_artifact("example.store", "1.0.0");
    let mut dependency = PackageDependency::id("example.store");
    dependency.alias = Some("store".to_string());
    dependency.collection_name_mapping = BTreeMap::from([
        (
            "package_secret".to_string(),
            "mapped_package_secret".to_string(),
        ),
        (
            "package_audit".to_string(),
            "mapped_package_audit".to_string(),
        ),
    ]);

    let requirement = package_requirement(
        "example.service",
        &dependency,
        std::slice::from_ref(&dependency_artifact),
    )
    .unwrap();

    assert_eq!(
        requirement.collection_name_mapping,
        dependency.collection_name_mapping
    );
}

#[test]
fn pre_source_schema_binding_is_exact_direct_or_compiler_owned_std_only() {
    let direct = canonical_artifact("example.types", "2.4.0");
    let transitive = canonical_artifact("example.transitive", "9.0.0");
    let std = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "7.4.2");
    let mut dependency = PackageDependency::id("example.types");
    dependency.version = "2.4.0".to_string();
    dependency.alias = Some("contractTypes".to_string());
    let available = [direct.clone(), transitive.clone(), std.clone()];

    let binding = pre_source_schema_binding(
        "example.types",
        std::slice::from_ref(&dependency),
        &[],
        std::slice::from_ref(&direct),
        &available,
    )
    .unwrap()
    .expect("exact direct owner");
    assert_eq!(binding.0, "contractTypes");
    assert_eq!(binding.1.package_build_id, direct.package_build_id);

    assert!(pre_source_schema_binding(
        "example.transitive",
        std::slice::from_ref(&dependency),
        &[],
        std::slice::from_ref(&direct),
        &[transitive],
    )
    .unwrap()
    .is_none());

    let binding = pre_source_schema_binding(
        SKIFF_STD_PUBLICATION_ID,
        std::slice::from_ref(&dependency),
        &[],
        std::slice::from_ref(&direct),
        std::slice::from_ref(&std),
    )
    .unwrap()
    .expect("compiler-owned std");
    assert_eq!(binding.0, "std");
    assert_eq!(binding.1.package_version, "7.4.2");
}

#[test]
fn pre_source_schema_binding_accepts_service_provider_package_owner() {
    let provider = canonical_artifact("example.service", "2.0.0");
    let contract: ServiceContract = serde_json::from_value(serde_json::json!({
        "schemaVersion": "skiff-service-contract-v5",
        "serviceId": "example.service",
        "contractVersion": "2.0.0",
        "serviceProtocolIdentity": "skiff-service-protocol-v5:sha256:test",
        "operations": {},
        "packageTypeRequirements": [],
        "diagnosticText": { "service": "", "operations": {}, "types": {} }
    }))
    .unwrap();
    let dependency = PackageContractCompileDependency {
        requirement: ContractRequirement {
            alias: "service".to_string(),
            service_id: "example.service".to_string(),
            contract_version: "2.0.0".to_string(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        },
        contract,
    };

    let binding = pre_source_schema_binding(
        "example.service",
        &[],
        std::slice::from_ref(&dependency),
        &[],
        std::slice::from_ref(&provider),
    )
    .unwrap()
    .expect("service provider package owner");

    assert_eq!(binding.0, "service");
    assert_eq!(binding.1.package_build_id, provider.package_build_id);
}

#[test]
fn pre_source_schema_binding_rejects_duplicate_owner_version_and_artifact() {
    let artifact = canonical_artifact("example.types", "1.0.0");
    let mut first = PackageDependency::id("example.types");
    first.version = "1.0.0".to_string();
    first.alias = Some("types".to_string());
    let mut second = first.clone();
    second.alias = Some("otherTypes".to_string());

    let duplicate_owner = pre_source_schema_binding(
        "example.types",
        &[first.clone(), second],
        &[],
        std::slice::from_ref(&artifact),
        std::slice::from_ref(&artifact),
    )
    .unwrap_err()
    .to_string();
    assert!(duplicate_owner.contains("duplicate direct dependency declarations"));

    let duplicate_artifact = pre_source_schema_binding(
        "example.types",
        std::slice::from_ref(&first),
        &[],
        &[artifact.clone(), artifact],
        &[],
    )
    .unwrap_err()
    .to_string();
    assert!(duplicate_artifact.contains("duplicate exact canonical artifacts"));

    let std = canonical_artifact(SKIFF_STD_PUBLICATION_ID, "1.0.0");
    let duplicate_std =
        pre_source_schema_binding(SKIFF_STD_PUBLICATION_ID, &[], &[], &[], &[std.clone(), std])
            .unwrap_err()
            .to_string();
    assert!(duplicate_std.contains("duplicate exact canonical artifacts"));
}

#[test]
fn top_level_alias_keeps_one_primary_requirement_and_pins_the_exact_build() {
    let artifact = canonical_artifact("example.widget", "1.0.0");
    let mut dependency = PackageDependency::id("example.widget");
    dependency.version = "1.0.0".to_string();
    dependency.alias = Some("widget".to_string());
    dependency.top_level_alias = Some("widgetImpl".to_string());

    let requirements = package_requirements_for_dependencies(
        "example.tests",
        std::slice::from_ref(&dependency),
        std::slice::from_ref(&artifact),
    )
    .unwrap();

    assert_eq!(requirements.len(), 1);
    assert_eq!(requirements[0].alias, "widget");
    assert_eq!(
        requirements[0].expected_package_build,
        Some(artifact.package_build_id)
    );
}

fn canonical_artifact(package_id: &str, version: &str) -> PackageArtifact {
    let empty_schema_types = BTreeMap::new();
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
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: package_schema_index_identity(
                package_id,
                &empty_schema_types,
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
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

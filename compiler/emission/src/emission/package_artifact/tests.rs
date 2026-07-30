use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_package_artifact_identities, validate_package_artifact_identities,
};
use skiff_artifact_model::{
    config_shape_from_package_requirements, FileIrRef, FileIrUnit, PackageArtifact, PackageBuildId,
    PackageConfigRequirement, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
    PackageRuntimeRequirements, PackageSchemaIndex, PackageSchemaIndexRef, PublicationResourceRef,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_core::json_utils::sha256_hex;

use super::*;

mod requirements;

#[test]
fn single_materializer_attaches_storage_paths_and_preserves_canonical_identity() {
    let (projected, file, resource) = fixture();
    let projected_identity = projected.package_build_id.clone();

    let materialized = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();

    assert_eq!(materialized.artifact.package_build_id, projected_identity);
    assert_eq!(
        materialized.artifact.files[0].artifact_path.as_deref(),
        Some("units/files/api.json")
    );
    assert_eq!(
        materialized.artifact.static_resources[0]
            .artifact_path
            .as_deref(),
        Some(resource.artifact_path.as_str())
    );
    assert_eq!(
        materialized.published.identity,
        projected.package_build_id.to_string()
    );
    assert!(materialized
        .published
        .path
        .starts_with("units/package-artifacts/example~com~~pkg/"));
    validate_package_artifact_identities(&materialized.artifact).unwrap();
}

#[test]
fn repeated_materialization_is_bit_identical() {
    let (projected, file, resource) = fixture();
    let first = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
    let second = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
    assert_eq!(first, second);
}

#[test]
fn materializer_preserves_canonical_config_requirements_without_a_second_shape_owner() {
    let (mut projected, file, resource) = fixture();
    projected.runtime_requirements.config = vec![
        PackageConfigRequirement {
            path: "app.timeout".to_string(),
            access: skiff_artifact_model::PackageConfigAccess::Optional {
                value_type: "number".to_string(),
            },
        },
        PackageConfigRequirement {
            path: "app.token".to_string(),
            access: skiff_artifact_model::PackageConfigAccess::Required {
                value_type: "string".to_string(),
            },
        },
    ];
    assign_package_artifact_identities(&mut projected).unwrap();

    let materialized = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
    assert_eq!(
        materialized.published.value["runtimeRequirements"]["config"],
        serde_json::json!([
            { "path": "app.timeout", "valueType": "number", "required": false },
            { "path": "app.token", "valueType": "string", "required": true }
        ])
    );
    assert!(materialized.published.value.get("configShape").is_none());

    let shape =
        config_shape_from_package_requirements(&materialized.artifact.runtime_requirements.config)
            .unwrap();
    assert_eq!(
        shape
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["app.timeout", "app.token"]
    );
}

#[test]
fn materializer_validates_instead_of_repairing_projection_identity() {
    let (mut projected, file, resource) = fixture();
    projected.package_build_id = PackageBuildId::new("tampered");
    let error = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("tampered"), "unexpected error: {error}");
}

#[test]
fn materializer_rejects_missing_or_unreferenced_assets() {
    let (projected, file, resource) = fixture();
    let missing_file =
        materialize_package_artifact(&projected, &[], std::slice::from_ref(&resource))
            .unwrap_err()
            .to_string();
    assert!(
        missing_file.contains("did not emit an artifact path"),
        "unexpected error: {missing_file}"
    );

    let missing_resource =
        materialize_package_artifact(&projected, std::slice::from_ref(&file), &[])
            .unwrap_err()
            .to_string();
    assert!(
        missing_resource.contains("has no emitted blob"),
        "unexpected error: {missing_resource}"
    );

    let mut extra_file = file.clone();
    extra_file.identity = "file:extra".to_string();
    extra_file.unit.file_ir_identity = extra_file.identity.clone();
    extra_file.module_path = "extra".to_string();
    extra_file.unit.module_path = "extra".to_string();
    let unreferenced = materialize_package_artifact(
        &projected,
        &[file, extra_file],
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        unreferenced.contains("not referenced by the package artifact"),
        "unexpected error: {unreferenced}"
    );
}

#[test]
fn projected_file_ir_handoff_requires_exact_typed_units() {
    let (artifact, file, _) = fixture();
    let package_schema_index = empty_schema_index(&artifact.package_id);
    let projected = ProjectedPackageArtifact {
        artifact,
        package_schema_index,
        package_schema_type_records: BTreeMap::new(),
        resolved_package_schema_type_records: BTreeMap::new(),
        file_ir_units: vec![file.unit.clone()],
        resources: Vec::new(),
    };
    validate_projected_file_ir_units(&projected, std::slice::from_ref(&file)).unwrap();

    let mut mismatched = file;
    mismatched.unit.source_ast_hash = "different-source".to_string();
    let error = validate_projected_file_ir_units(&projected, &[mismatched])
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("does not match"),
        "unexpected error: {error}"
    );
}

fn fixture() -> (
    PackageArtifact,
    PublishedFileIrArtifact,
    PublishedResourceArtifact,
) {
    let bytes = b"package resource".to_vec();
    let resource_hash = sha256_hex(&bytes);
    let mut file_unit = FileIrUnit::empty("api", "source-hash");
    file_unit.file_ir_identity =
        "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_string();
    let file_ref = FileIrRef {
        file_ir_identity: file_unit.file_ir_identity.clone(),
        module_path: file_unit.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file_unit.source_ast_hash.clone()),
    };
    let published_file = PublishedFileIrArtifact {
        unit: file_unit.clone(),
        identity: file_unit.file_ir_identity.clone(),
        hash: "file-json-hash".to_string(),
        path: "units/files/api.json".to_string(),
        source_path: "src/api.skiff".to_string(),
        module_path: file_unit.module_path.clone(),
    };
    let resource_ref = PublicationResourceRef {
        path: "templates/message.txt".to_string(),
        sha256: resource_hash.clone(),
        byte_len: bytes.len() as u64,
        content_type: Some("text/plain".to_string()),
        artifact_path: None,
    };
    let published_resource = PublishedResourceArtifact {
        logical_path: resource_ref.path.clone(),
        artifact_path: format!("resources/sha256/{resource_hash}"),
        sha256: resource_hash,
        byte_len: bytes.len() as u64,
        bytes,
    };
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.com/pkg".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref],
        static_resources: vec![resource_ref],
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.com/pkg".to_string(),
            package_schema_index_identity: empty_schema_index("example.com/pkg")
                .package_schema_index_identity,
        },
        package_schema_type_records: BTreeMap::new(),
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
    (artifact, published_file, published_resource)
}

fn empty_schema_index(package_id: &str) -> PackageSchemaIndex {
    PackageSchemaIndex {
        package_id: package_id.to_string(),
        package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
            package_id,
            &BTreeMap::new(),
        )
        .expect("empty Package schema index is canonical"),
        types: BTreeMap::new(),
    }
}

use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_package_artifact_identities, validate_package_artifact_identities,
};
use skiff_artifact_model::{
    FileIrRef, FileIrUnit, PackageArtifact, PackageBuildId, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PublicationResourceRef,
    PACKAGE_ARTIFACT_SCHEMA_VERSION,
};
use skiff_compiler_core::json_utils::sha256_hex;

use super::*;

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
fn production_and_package_test_consumers_share_bit_identical_materializer_output() {
    let (projected, file, resource) = fixture();
    let production = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
    let package_test = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
    assert_eq!(production, package_test);
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
        role: "package".to_string(),
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
    (artifact, published_file, published_resource)
}

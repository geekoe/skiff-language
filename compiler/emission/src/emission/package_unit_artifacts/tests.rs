use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use skiff_artifact_identity::{
    assign_package_unit_identities, package_build_identity, package_local_abi_identity,
};
use skiff_artifact_model::PublicationResourceRef;
use skiff_compiler_core::json_utils::sha256_hex;

use super::*;
use crate::{
    emission::resources::{
        attach_resource_artifact_paths, publish_resource_artifacts, resource_artifact_path,
    },
    projection::package_unit_artifacts::ProjectedPublicationResource,
};

#[test]
fn resource_blob_and_unit_json_refs_are_emitted_as_raw_artifacts() {
    let temp = TempDir::new("resource-blob-unit-json");
    let resource_path = temp.write("prompts/system.md", b"hello resource");
    let sha256 = sha256_hex(b"hello resource");
    let resource = ProjectedPublicationResource {
        path: "prompts/system.md".to_string(),
        absolute_path: resource_path,
        byte_len: 14,
        sha256: sha256.clone(),
        content_type: None,
    };
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.resources = vec![resource_ref("prompts/system.md", &sha256, 14)];

    let resource_blobs =
        publish_resource_artifacts(&[resource]).expect("resource blob should publish");
    attach_resource_artifact_paths(&mut unit.resources, &resource_blobs)
        .expect("resource refs should attach");
    assign_package_unit_identities(&mut unit).expect("package identities");
    let package_unit = package_unit_artifact(&unit).expect("package unit artifact");

    assert_eq!(resource_blobs.len(), 1);
    assert_eq!(
        resource_blobs[0].artifact_path,
        format!("resources/sha256/{sha256}")
    );
    assert_eq!(resource_blobs[0].sha256, sha256);
    assert_eq!(resource_blobs[0].byte_len, 14);
    assert_eq!(resource_blobs[0].bytes, b"hello resource");
    assert_eq!(
        package_unit.value["resources"][0]["artifactPath"],
        resource_blobs[0].artifact_path
    );
    assert!(package_unit.value["resources"][0].get("bytes").is_none());
}

#[test]
fn resource_content_changes_package_build_identity_not_abi_identity() {
    let first = package_unit_with_resource(b"first resource");
    let second = package_unit_with_resource(b"second resource");

    assert_ne!(
        package_build_identity(&first).expect("first build identity"),
        package_build_identity(&second).expect("second build identity")
    );
    assert_eq!(
        package_local_abi_identity(&first).expect("first ABI identity"),
        package_local_abi_identity(&second).expect("second ABI identity")
    );
}

#[test]
fn shared_materializer_preserves_logical_resource_refs_for_one_blob() {
    let bytes = b"shared resource".to_vec();
    let sha256 = sha256_hex(&bytes);
    let artifact_path = resource_artifact_path(&sha256);
    let artifact = PublishedResourceArtifact {
        logical_path: "prompts/first.md".to_string(),
        artifact_path: artifact_path.clone(),
        sha256: sha256.clone(),
        byte_len: bytes.len() as u64,
        bytes,
    };
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.resources = vec![
        PublicationResourceRef {
            path: "prompts/first.md".to_string(),
            sha256: sha256.clone(),
            byte_len: artifact.byte_len,
            content_type: Some("text/markdown".to_string()),
            artifact_path: None,
        },
        PublicationResourceRef {
            path: "schemas/second.json".to_string(),
            sha256,
            byte_len: artifact.byte_len,
            content_type: Some("application/json".to_string()),
            artifact_path: None,
        },
    ];
    assign_package_unit_identities(&mut unit).expect("projected package identities");

    let materialized = materialize_package_unit_artifact(&unit, &[], &[artifact])
        .expect("one content blob may back multiple logical resource refs");

    assert_eq!(materialized.unit.resources.len(), 2);
    assert_eq!(materialized.unit.resources[0].path, "prompts/first.md");
    assert_eq!(
        materialized.unit.resources[0].content_type.as_deref(),
        Some("text/markdown")
    );
    assert_eq!(materialized.unit.resources[1].path, "schemas/second.json");
    assert_eq!(
        materialized.unit.resources[1].content_type.as_deref(),
        Some("application/json")
    );
    assert!(materialized
        .unit
        .resources
        .iter()
        .all(|resource| resource.artifact_path.as_deref() == Some(artifact_path.as_str())));
}

#[test]
fn shared_materializer_rejects_unbound_resource_without_blob() {
    let sha256 = sha256_hex(b"missing resource");
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.resources = vec![resource_ref("prompts/missing.md", &sha256, 16)];
    assign_package_unit_identities(&mut unit).expect("projected package identities");

    let error = materialize_package_unit_artifact(&unit, &[], &[])
        .expect_err("projected resource refs require their production blobs")
        .to_string();

    assert!(
        error.contains("has no emitted blob"),
        "unexpected error: {error}"
    );
}

#[test]
fn shared_materializer_rejects_prebound_resource_without_blob() {
    let sha256 = sha256_hex(b"missing resource");
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    let mut resource = resource_ref("prompts/missing.md", &sha256, 16);
    resource.artifact_path = Some(resource_artifact_path(&sha256));
    unit.resources = vec![resource];
    assign_package_unit_identities(&mut unit).expect("projected package identities");

    let error = materialize_package_unit_artifact(&unit, &[], &[])
        .expect_err("a pre-bound storage path does not prove resource content exists")
        .to_string();

    assert!(
        error.contains("has no emitted blob"),
        "unexpected error: {error}"
    );
}

#[test]
fn shared_materializer_rejects_noncanonical_resource_blob_metadata() {
    let bytes = b"resource".to_vec();
    let sha256 = sha256_hex(&bytes);
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.resources = vec![resource_ref(
        "prompts/resource.md",
        &sha256,
        bytes.len() as u64,
    )];
    assign_package_unit_identities(&mut unit).expect("projected package identities");
    let artifact = PublishedResourceArtifact {
        logical_path: "prompts/resource.md".to_string(),
        artifact_path: "resources/not-content-addressed".to_string(),
        sha256,
        byte_len: bytes.len() as u64,
        bytes,
    };

    let error = materialize_package_unit_artifact(&unit, &[], &[artifact])
        .expect_err("resource blobs must use canonical content-addressed paths")
        .to_string();

    assert!(
        error.contains("canonical path"),
        "unexpected error: {error}"
    );
}

#[test]
fn shared_materializer_rejects_tampered_projected_identity() {
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    assign_package_unit_identities(&mut unit).expect("projected package identities");
    unit.build_identity = "tampered-build-identity".to_string();

    let error = materialize_package_unit_artifact(&unit, &[], &[])
        .expect_err("emission must validate rather than repair projected identities")
        .to_string();

    assert!(
        error.contains("declared buildIdentity tampered-build-identity"),
        "unexpected error: {error}"
    );
}

fn package_unit_with_resource(bytes: &[u8]) -> PackageUnit {
    let sha256 = sha256_hex(bytes);
    let mut unit = PackageUnit::empty("example.com/pkg", "1.0.0", "", "");
    unit.resources = vec![PublicationResourceRef {
        path: "prompts/system.md".to_string(),
        sha256: sha256.clone(),
        byte_len: bytes.len() as u64,
        content_type: None,
        artifact_path: Some(format!("resources/sha256/{sha256}")),
    }];
    assign_package_unit_identities(&mut unit).expect("package identities");
    unit
}

fn resource_ref(path: &str, sha256: &str, byte_len: u64) -> PublicationResourceRef {
    PublicationResourceRef {
        path: path.to_string(),
        sha256: sha256.to_string(),
        byte_len,
        content_type: None,
        artifact_path: None,
    }
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-emission-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp dir");
        Self { path }
    }

    fn write(&self, relative_path: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(Path::new(relative_path));
        fs::create_dir_all(path.parent().expect("resource parent")).expect("resource parent");
        fs::write(&path, bytes).expect("resource write");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

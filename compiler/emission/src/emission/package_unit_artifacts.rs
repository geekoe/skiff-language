use serde::Serialize;
use skiff_artifact_model::{FileIrRef, PackageUnit, PACKAGE_UNIT_SCHEMA_VERSION};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::json_utils::value_sha256;

use crate::emission::artifact::{
    PublishedFileIrArtifact, PublishedJsonArtifact, PublishedResourceArtifact,
};
use crate::emission::artifact_assembly::{PackageVersionIndexModel, PublishedPackageArtifacts};
use crate::emission::identity::assign_package_unit_identities;
use crate::emission::resources::{attach_resource_artifact_paths, publish_resource_artifacts};
use crate::error::EmissionError;
use crate::error::Result;
use crate::projection::package_unit_artifacts::ProjectedPackageIrArtifacts;

pub struct PublishedPackageIrArtifacts {
    pub package_unit: PublishedJsonArtifact,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

pub fn publish_package_ir_artifacts(
    package: &PublishedPackageArtifacts,
    projected: &ProjectedPackageIrArtifacts,
) -> Result<PublishedPackageIrArtifacts> {
    let mut unit = projected.unit.clone();
    let resource_blobs = publish_resource_artifacts(&projected.resources)?;
    attach_published_file_paths_to_package_unit(&mut unit.files, &package.file_ir_units)?;
    attach_resource_artifact_paths(&mut unit.resources, &resource_blobs)?;
    assign_package_unit_identities(&mut unit)?;
    let package_unit = package_unit_artifact(&unit);

    Ok(PublishedPackageIrArtifacts {
        package_unit,
        unit,
        file_ir_units: package.file_ir_units.clone(),
        resource_blobs,
    })
}

fn attach_published_file_paths_to_package_unit(
    refs: &mut [FileIrRef],
    artifacts: &[PublishedFileIrArtifact],
) -> Result<()> {
    let by_identity = artifacts
        .iter()
        .map(|artifact| (artifact.identity.as_str(), artifact))
        .collect::<std::collections::BTreeMap<_, _>>();
    for file_ref in refs {
        let Some(artifact) = by_identity.get(file_ref.file_ir_identity.as_str()) else {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "package unit File IR ref {} did not emit an artifact path",
                    file_ref.file_ir_identity
                ),
            });
        };
        file_ref.artifact_path = Some(artifact.path.clone());
        file_ref.source_ast_hash = Some(artifact.unit.source_ast_hash.clone());
    }
    Ok(())
}

fn package_unit_artifact(unit: &PackageUnit) -> PublishedJsonArtifact {
    let value = serde_json::to_value(unit).expect("PackageUnit must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(&unit.package_id)
        .expect("package id was validated before artifact projection")
        .artifact_path();
    let path = format!("units/packages/{package_path}/{hash}.json");
    PublishedJsonArtifact {
        value,
        identity: unit.build_identity.clone(),
        hash,
        path,
    }
}

pub fn package_index_with_package_unit(
    package: &PublishedPackageArtifacts,
    unit: &PackageUnit,
    package_unit: &PublishedJsonArtifact,
) -> PublishedJsonArtifact {
    let value = serde_json::to_value(PackageVersionIndexWithPackageUnit {
        index: &package.version_index_model,
        package_unit: PackageUnitPointer::new(unit, package_unit),
    })
    .expect("package version index with package unit must serialize");
    let hash = value_sha256(&value);
    PublishedJsonArtifact {
        value,
        identity: package.version_index.identity.clone(),
        hash,
        path: package.version_index.path.clone(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageVersionIndexWithPackageUnit<'a> {
    #[serde(flatten)]
    index: &'a PackageVersionIndexModel,
    package_unit: PackageUnitPointer<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageUnitPointer<'a> {
    schema_version: &'static str,
    package_id: &'a str,
    version: &'a str,
    build_identity: &'a str,
    abi_identity: &'a str,
    unit_hash: &'a str,
    unit_path: &'a str,
}

impl<'a> PackageUnitPointer<'a> {
    fn new(unit: &'a PackageUnit, artifact: &'a PublishedJsonArtifact) -> Self {
        Self {
            schema_version: PACKAGE_UNIT_SCHEMA_VERSION,
            package_id: unit.package_id.as_str(),
            version: unit.version.as_str(),
            build_identity: artifact.identity.as_str(),
            abi_identity: unit.abi_identity.as_str(),
            unit_hash: artifact.hash.as_str(),
            unit_path: artifact.path.as_str(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use skiff_artifact_identity::{package_abi_identity, package_build_identity};
    use skiff_artifact_model::PublicationResourceRef;
    use skiff_compiler_core::json_utils::sha256_hex;

    use super::*;
    use crate::{
        emission::resources::{attach_resource_artifact_paths, publish_resource_artifacts},
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
        let package_unit = package_unit_artifact(&unit);

        assert_eq!(resource_blobs.len(), 1);
        assert_eq!(resource_blobs[0].artifact_path, format!("resources/sha256/{sha256}"));
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
            package_abi_identity(&first).expect("first ABI identity"),
            package_abi_identity(&second).expect("second ABI identity")
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
}

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use skiff_artifact_identity::validate_package_unit_identities;
use skiff_artifact_model::{FileIrRef, PackageUnit, PACKAGE_UNIT_SCHEMA_VERSION};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::json_utils::value_sha256;

use crate::emission::artifact::{
    PublishedFileIrArtifact, PublishedJsonArtifact, PublishedResourceArtifact,
};
use crate::emission::artifact_assembly::{PackageVersionIndexModel, PublishedPackageArtifacts};
use crate::emission::resources::{
    normalized_resource_artifacts, publish_resource_artifacts,
    validated_resource_artifacts_by_content,
};
use crate::error::EmissionError;
use crate::error::Result;
use crate::projection::package_unit_artifacts::ProjectedPackageIrArtifacts;

pub struct PublishedPackageIrArtifacts {
    pub package_unit: PublishedJsonArtifact,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedPackageUnitArtifact {
    pub unit: PackageUnit,
    pub artifact: PublishedJsonArtifact,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

pub fn publish_package_ir_artifacts(
    package: &PublishedPackageArtifacts,
    projected: &ProjectedPackageIrArtifacts,
) -> Result<PublishedPackageIrArtifacts> {
    let resource_blobs = publish_resource_artifacts(&projected.resources)?;
    let materialized = materialize_package_unit_artifact(
        &projected.unit,
        &package.file_ir_units,
        &resource_blobs,
    )?;

    Ok(PublishedPackageIrArtifacts {
        package_unit: materialized.artifact,
        unit: materialized.unit,
        file_ir_units: package.file_ir_units.clone(),
        resource_blobs: materialized.resource_blobs,
    })
}

pub fn materialize_package_unit_artifact(
    projected: &PackageUnit,
    files: &[PublishedFileIrArtifact],
    resource_blobs: &[PublishedResourceArtifact],
) -> Result<MaterializedPackageUnitArtifact> {
    let mut unit = projected.clone();
    let resource_blobs = normalized_resource_artifacts(resource_blobs)?;
    attach_published_file_paths_to_package_unit(&mut unit.files, files)?;
    attach_resource_paths_to_package_unit(&mut unit.resources, &resource_blobs)?;
    validate_package_unit_identities(&unit)?;
    let artifact = package_unit_artifact(&unit)?;
    Ok(MaterializedPackageUnitArtifact {
        unit,
        artifact,
        resource_blobs,
    })
}

fn attach_published_file_paths_to_package_unit(
    refs: &mut [FileIrRef],
    artifacts: &[PublishedFileIrArtifact],
) -> Result<()> {
    let mut expected = BTreeMap::new();
    for file_ref in refs.iter() {
        if expected
            .insert(
                file_ref.file_ir_identity.as_str(),
                file_ref.module_path.as_str(),
            )
            .is_some()
        {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "package unit contains duplicate File IR ref {}",
                    file_ref.file_ir_identity
                ),
            });
        }
    }
    let mut by_identity = BTreeMap::new();
    for artifact in artifacts {
        if artifact.identity != artifact.unit.file_ir_identity {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published File IR {} identity does not match typed unit identity {}",
                    artifact.identity, artifact.unit.file_ir_identity
                ),
            });
        }
        if artifact.module_path != artifact.unit.module_path {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published File IR {} module {} does not match typed unit module {}",
                    artifact.identity, artifact.module_path, artifact.unit.module_path
                ),
            });
        }
        if artifact.path.trim().is_empty() {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published File IR {} has an empty artifact path",
                    artifact.identity
                ),
            });
        }
        let Some(expected_module) = expected.get(artifact.identity.as_str()) else {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published File IR {} module {} is not referenced by the package unit",
                    artifact.identity, artifact.module_path
                ),
            });
        };
        if *expected_module != artifact.module_path {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "package unit File IR {} module {} does not match published module {}",
                    artifact.identity, expected_module, artifact.module_path
                ),
            });
        }
        if by_identity
            .insert(artifact.identity.as_str(), artifact)
            .is_some()
        {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published File IR identity {} is duplicated",
                    artifact.identity
                ),
            });
        }
    }
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

fn attach_resource_paths_to_package_unit(
    refs: &mut [skiff_artifact_model::PublicationResourceRef],
    artifacts: &[PublishedResourceArtifact],
) -> Result<()> {
    let by_hash_and_len = validated_resource_artifacts_by_content(artifacts)?;
    let mut used = BTreeSet::new();
    for resource_ref in refs {
        let key = (resource_ref.sha256.as_str(), resource_ref.byte_len);
        let Some(artifact) = by_hash_and_len.get(&key) else {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "package resource {} has no emitted blob sha256 {} size {}",
                    resource_ref.path, resource_ref.sha256, resource_ref.byte_len
                ),
            });
        };
        resource_ref.artifact_path = Some(artifact.artifact_path.clone());
        used.insert(key);
    }
    for artifact in artifacts {
        let key = (artifact.sha256.as_str(), artifact.byte_len);
        if !used.contains(&key) {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "published resource blob {} is not referenced by package unit",
                    artifact.artifact_path
                ),
            });
        }
    }
    Ok(())
}

fn package_unit_artifact(unit: &PackageUnit) -> Result<PublishedJsonArtifact> {
    let value = serde_json::to_value(unit).expect("PackageUnit must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(&unit.package_id)
        .map_err(|error| EmissionError::ContractValidation {
            message: format!("package unit id {} is invalid: {error}", unit.package_id),
        })?
        .artifact_path();
    let path = format!("units/packages/{package_path}/{hash}.json");
    Ok(PublishedJsonArtifact {
        value,
        identity: unit.build_identity.clone(),
        hash,
        path,
    })
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
mod tests;

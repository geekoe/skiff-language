use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::PackageArtifact;
use skiff_compiler_core::{id::PublicationId, json_utils::value_sha256};

use crate::{
    emission::{
        artifact::{PublishedFileIrArtifact, PublishedJsonArtifact, PublishedResourceArtifact},
        resources::{normalized_resource_artifacts, publish_resource_artifacts},
    },
    error::{EmissionError, Result},
    projection::package_artifact::ProjectedPackageArtifact,
};

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedPackageArtifact {
    pub artifact: PackageArtifact,
    pub published: PublishedJsonArtifact,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedPackageArtifact {
    pub artifact: PackageArtifact,
    pub published: PublishedJsonArtifact,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

/// Terminal PackageArtifact storage materializer. It validates canonical
/// identities and never repairs or recomputes projection semantics.
pub fn materialize_package_artifact(
    projected: &PackageArtifact,
    files: &[PublishedFileIrArtifact],
    resource_blobs: &[PublishedResourceArtifact],
) -> Result<MaterializedPackageArtifact> {
    let mut artifact = projected.clone();
    let resource_blobs = normalized_resource_artifacts(resource_blobs)?;
    super::package_assets::attach_published_file_paths(
        "package artifact",
        &mut artifact.files,
        files,
    )?;
    super::package_assets::attach_resource_paths(
        "package artifact",
        &mut artifact.static_resources,
        &resource_blobs,
    )?;
    super::package_requirement_coverage::validate_file_ir_package_requirement_coverage(
        &artifact, files,
    )?;
    validate_package_artifact_identities(&artifact)?;
    let published = package_artifact_json(&artifact)?;
    Ok(MaterializedPackageArtifact {
        artifact,
        published,
        resource_blobs,
    })
}

pub fn publish_projected_package_artifact(
    projected: &ProjectedPackageArtifact,
    files: &[PublishedFileIrArtifact],
) -> Result<PublishedPackageArtifact> {
    validate_projected_file_ir_units(projected, files)?;
    let resource_blobs = publish_resource_artifacts(&projected.resources)?;
    let materialized = materialize_package_artifact(&projected.artifact, files, &resource_blobs)?;
    Ok(PublishedPackageArtifact {
        artifact: materialized.artifact,
        published: materialized.published,
        file_ir_units: files.to_vec(),
        resource_blobs: materialized.resource_blobs,
    })
}

fn validate_projected_file_ir_units(
    projected: &ProjectedPackageArtifact,
    files: &[PublishedFileIrArtifact],
) -> Result<()> {
    if projected.file_ir_units.len() != files.len() {
        return Err(EmissionError::ContractValidation {
            message: format!(
                "projected package has {} File IR units but emission received {}",
                projected.file_ir_units.len(),
                files.len()
            ),
        });
    }
    for unit in &projected.file_ir_units {
        let Some(published) = files
            .iter()
            .find(|file| file.identity == unit.file_ir_identity)
        else {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "projected File IR {} has no exact emitted unit",
                    unit.file_ir_identity
                ),
            });
        };
        if published.unit != *unit {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "emitted File IR {} does not match its projected typed unit",
                    unit.file_ir_identity
                ),
            });
        }
    }
    Ok(())
}

fn package_artifact_json(artifact: &PackageArtifact) -> Result<PublishedJsonArtifact> {
    let value = serde_json::to_value(artifact).expect("PackageArtifact must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(&artifact.package_id)
        .map_err(|error| EmissionError::ContractValidation {
            message: format!(
                "package artifact id {} is invalid: {error}",
                artifact.package_id
            ),
        })?
        .artifact_path();
    Ok(PublishedJsonArtifact {
        value,
        identity: artifact.package_build_id.to_string(),
        hash: hash.clone(),
        path: format!("units/package-artifacts/{package_path}/{hash}.json"),
    })
}

#[cfg(test)]
mod tests;

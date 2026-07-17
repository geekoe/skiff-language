use std::{collections::BTreeMap, fs};

use skiff_artifact_model::PublicationResourceRef;
use skiff_compiler_core::json_utils::sha256_hex;

use crate::{
    emission::artifact::PublishedResourceArtifact,
    error::{EmissionError, Result},
    projection::package_unit_artifacts::ProjectedPublicationResource,
};

pub fn publish_resource_artifacts(
    resources: &[ProjectedPublicationResource],
) -> Result<Vec<PublishedResourceArtifact>> {
    let mut by_artifact_path = BTreeMap::<String, PublishedResourceArtifact>::new();
    for resource in resources {
        let bytes = fs::read(&resource.absolute_path).map_err(|source| {
            EmissionError::ContractValidation {
                message: format!(
                    "failed to read resource {} from {}: {source}",
                    resource.path,
                    resource.absolute_path.display()
                ),
            }
        })?;
        let byte_len = bytes.len() as u64;
        if byte_len != resource.byte_len {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource {} size changed during emission: expected {}, got {}",
                    resource.path, resource.byte_len, byte_len
                ),
            });
        }
        let sha256 = sha256_hex(&bytes);
        if sha256 != resource.sha256 {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource {} hash changed during emission: expected {}, got {}",
                    resource.path, resource.sha256, sha256
                ),
            });
        }
        let artifact_path = resource_artifact_path(&resource.sha256);
        let artifact = PublishedResourceArtifact {
            logical_path: resource.path.clone(),
            artifact_path: artifact_path.clone(),
            sha256: resource.sha256.clone(),
            byte_len: resource.byte_len,
            bytes,
        };
        match by_artifact_path.get(&artifact_path) {
            Some(existing) if existing.bytes == artifact.bytes => {}
            Some(_) => {
                return Err(EmissionError::ContractValidation {
                    message: format!("resource artifact conflict at {artifact_path}"),
                });
            }
            None => {
                by_artifact_path.insert(artifact_path, artifact);
            }
        }
    }
    Ok(by_artifact_path.into_values().collect())
}

pub(crate) fn attach_resource_artifact_paths(
    refs: &mut [PublicationResourceRef],
    artifacts: &[PublishedResourceArtifact],
) -> Result<()> {
    let by_hash_and_len = validated_resource_artifacts_by_content(artifacts)?;
    for resource_ref in refs {
        let Some(artifact) =
            by_hash_and_len.get(&(resource_ref.sha256.as_str(), resource_ref.byte_len))
        else {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource {} did not emit blob sha256 {} size {}",
                    resource_ref.path, resource_ref.sha256, resource_ref.byte_len
                ),
            });
        };
        resource_ref.artifact_path = Some(artifact.artifact_path.clone());
    }
    Ok(())
}

pub(crate) fn validated_resource_artifacts_by_content(
    artifacts: &[PublishedResourceArtifact],
) -> Result<BTreeMap<(&str, u64), &PublishedResourceArtifact>> {
    let mut by_hash_and_len: BTreeMap<(&str, u64), &PublishedResourceArtifact> = BTreeMap::new();
    for artifact in artifacts {
        let actual_byte_len = artifact.bytes.len() as u64;
        if actual_byte_len != artifact.byte_len {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource artifact {} declares size {}, got {} bytes",
                    artifact.artifact_path, artifact.byte_len, actual_byte_len
                ),
            });
        }
        let actual_sha256 = sha256_hex(&artifact.bytes);
        if actual_sha256 != artifact.sha256 {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource artifact {} declares sha256 {}, got {}",
                    artifact.artifact_path, artifact.sha256, actual_sha256
                ),
            });
        }
        let expected_path = resource_artifact_path(&artifact.sha256);
        if artifact.artifact_path != expected_path {
            return Err(EmissionError::ContractValidation {
                message: format!(
                    "resource artifact {} must use canonical path {}",
                    artifact.artifact_path, expected_path
                ),
            });
        }
        let key = (artifact.sha256.as_str(), artifact.byte_len);
        match by_hash_and_len.get(&key) {
            Some(existing)
                if existing.artifact_path == artifact.artifact_path
                    && existing.bytes == artifact.bytes => {}
            Some(_) => {
                return Err(EmissionError::ContractValidation {
                    message: format!(
                        "resource artifacts for sha256 {} size {} conflict",
                        artifact.sha256, artifact.byte_len
                    ),
                });
            }
            None => {
                by_hash_and_len.insert(key, artifact);
            }
        }
    }
    Ok(by_hash_and_len)
}

pub(crate) fn normalized_resource_artifacts(
    artifacts: &[PublishedResourceArtifact],
) -> Result<Vec<PublishedResourceArtifact>> {
    Ok(validated_resource_artifacts_by_content(artifacts)?
        .into_values()
        .cloned()
        .collect())
}

pub(crate) fn resource_artifact_path(sha256: &str) -> String {
    format!("resources/sha256/{sha256}")
}

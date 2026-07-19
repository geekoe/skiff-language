use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{FileIrRef, PublicationResourceRef};

use crate::{
    emission::{artifact::PublishedFileIrArtifact, artifact::PublishedResourceArtifact},
    error::{EmissionError, Result},
};

use super::resources::validated_resource_artifacts_by_content;

pub(super) fn attach_published_file_paths(
    owner_label: &str,
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
            return contract_error(format!(
                "{owner_label} contains duplicate File IR ref {}",
                file_ref.file_ir_identity
            ));
        }
    }
    let mut by_identity = BTreeMap::new();
    for artifact in artifacts {
        validate_published_file(artifact)?;
        let Some(expected_module) = expected.get(artifact.identity.as_str()) else {
            return contract_error(format!(
                "published File IR {} module {} is not referenced by the {owner_label}",
                artifact.identity, artifact.module_path
            ));
        };
        if *expected_module != artifact.module_path {
            return contract_error(format!(
                "{owner_label} File IR {} module {} does not match published module {}",
                artifact.identity, expected_module, artifact.module_path
            ));
        }
        if by_identity
            .insert(artifact.identity.as_str(), artifact)
            .is_some()
        {
            return contract_error(format!(
                "published File IR identity {} is duplicated",
                artifact.identity
            ));
        }
    }
    for file_ref in refs {
        let Some(artifact) = by_identity.get(file_ref.file_ir_identity.as_str()) else {
            return contract_error(format!(
                "{owner_label} File IR ref {} did not emit an artifact path",
                file_ref.file_ir_identity
            ));
        };
        file_ref.artifact_path = Some(artifact.path.clone());
        file_ref.source_ast_hash = Some(artifact.unit.source_ast_hash.clone());
    }
    Ok(())
}

fn validate_published_file(artifact: &PublishedFileIrArtifact) -> Result<()> {
    if artifact.identity != artifact.unit.file_ir_identity {
        return contract_error(format!(
            "published File IR {} identity does not match typed unit identity {}",
            artifact.identity, artifact.unit.file_ir_identity
        ));
    }
    if artifact.module_path != artifact.unit.module_path {
        return contract_error(format!(
            "published File IR {} module {} does not match typed unit module {}",
            artifact.identity, artifact.module_path, artifact.unit.module_path
        ));
    }
    if artifact.path.trim().is_empty() {
        return contract_error(format!(
            "published File IR {} has an empty artifact path",
            artifact.identity
        ));
    }
    Ok(())
}

pub(super) fn attach_resource_paths(
    owner_label: &str,
    refs: &mut [PublicationResourceRef],
    artifacts: &[PublishedResourceArtifact],
) -> Result<()> {
    let by_hash_and_len = validated_resource_artifacts_by_content(artifacts)?;
    let mut used = BTreeSet::new();
    for resource_ref in refs {
        let key = (resource_ref.sha256.as_str(), resource_ref.byte_len);
        let Some(artifact) = by_hash_and_len.get(&key) else {
            return contract_error(format!(
                "{owner_label} resource {} has no emitted blob sha256 {} size {}",
                resource_ref.path, resource_ref.sha256, resource_ref.byte_len
            ));
        };
        resource_ref.artifact_path = Some(artifact.artifact_path.clone());
        used.insert(key);
    }
    for artifact in artifacts {
        let key = (artifact.sha256.as_str(), artifact.byte_len);
        if !used.contains(&key) {
            return contract_error(format!(
                "published resource blob {} is not referenced by {owner_label}",
                artifact.artifact_path
            ));
        }
    }
    Ok(())
}

fn contract_error<T>(message: String) -> Result<T> {
    Err(EmissionError::ContractValidation { message })
}

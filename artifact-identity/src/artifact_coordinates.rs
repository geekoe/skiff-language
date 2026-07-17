use serde_json::Value;

use crate::{
    artifact_path::ArtifactRelativePath,
    framing::{is_lowercase_sha256, sha256_hex},
    service_assembly_identity::service_assembly_identity_hash,
    ArtifactIdentityError, Result,
};

pub fn package_unit_content_hash(unit: &Value) -> Result<String> {
    let bytes = skiff_canonical_json::canonical_json_bytes(unit).map_err(|error| {
        ArtifactIdentityError::InvalidPackageIdentityInput {
            message: error.to_string(),
        }
    })?;
    Ok(sha256_hex(&bytes))
}

pub fn validate_service_assembly_artifact_path(
    path: &str,
    service_id: &str,
    assembly_identity: &str,
) -> Result<()> {
    let path = ArtifactRelativePath::parse(path, "serviceAssembly.assemblyPath")?;
    validate_canonical_path(
        &path,
        &format!(
            "assemblies/services/{}/{}.json",
            publication_storage_segment(service_id, "service id")?,
            service_assembly_identity_hash(assembly_identity)?
        ),
        "service assembly",
    )
}

pub fn validate_package_unit_artifact_path(
    path: &str,
    package_id: &str,
    unit_hash: &str,
) -> Result<()> {
    if !is_lowercase_sha256(unit_hash) {
        return Err(ArtifactIdentityError::InvalidPackageIdentityInput {
            message: format!("unit hash {unit_hash} must be 64 lowercase hex characters"),
        });
    }
    let path = ArtifactRelativePath::parse(path, "packageUnit.unitPath")?;
    validate_canonical_path(
        &path,
        &format!(
            "units/packages/{}/{}.json",
            publication_storage_segment(package_id, "package id")?,
            unit_hash
        ),
        "package unit",
    )
}

pub fn publication_storage_segment(value: &str, label: &str) -> Result<String> {
    validate_publication_id(value, label)?;
    Ok(value.replace('.', "~").replace('/', "~~"))
}

pub(crate) fn validate_canonical_path(
    path: &ArtifactRelativePath,
    expected: &str,
    label: &str,
) -> Result<()> {
    if path.as_str() != expected {
        return Err(ArtifactIdentityError::NonCanonicalArtifactPath {
            label: label.to_string(),
            path: path.to_string(),
            expected: expected.to_string(),
        });
    }
    Ok(())
}

fn validate_publication_id(value: &str, label: &str) -> Result<()> {
    if value.is_empty() || value.len() > 63 || value == "std" {
        return Err(invalid_publication_id(value, label));
    }
    if value != value.trim()
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        return Err(invalid_publication_id(value, label));
    }

    let Some((authority, local)) = value.split_once('/') else {
        return Err(invalid_publication_id(value, label));
    };
    let authority_labels = authority.split('.').collect::<Vec<_>>();
    if authority_labels.len() < 2
        || authority_labels
            .iter()
            .any(|item| !is_valid_authority_label(item))
        || local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(invalid_publication_id(value, label));
    }
    Ok(())
}

fn invalid_publication_id(value: &str, label: &str) -> ArtifactIdentityError {
    ArtifactIdentityError::InvalidPublicationId {
        label: label.to_string(),
        value: value.to_string(),
    }
}

fn is_valid_authority_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0] != b'-'
        && bytes.last() != Some(&b'-')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_valid_local_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_lowercase()
        && bytes.last() != Some(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

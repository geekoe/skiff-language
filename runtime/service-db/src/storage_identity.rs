use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use crate::{Result, ServiceDbError};

const SKIFF_PACKAGE_COLLECTION_PREFIX: &str = "_skiff_c1_";

pub(crate) fn service_storage_database_name(environment: &str, service_id: &str) -> Result<String> {
    skiff_artifact_model::validate_activation_environment(environment)
        .map_err(ServiceDbError::Decode)?;
    validate_publication_id(service_id)?;
    Ok(service_id.replace('.', "~").replace('/', "~~"))
}

/// Encodes one Package-owned logical collection identity into the physical
/// collection name used inside the current service database.
///
/// Package build/version and activation facts are deliberately absent: storage
/// remains stable across compatible Package and service deployments, while the
/// Package ID prevents equal logical names in different Packages from sharing
/// a collection.
pub fn service_storage_collection_name(
    package_id: &str,
    declared_collection_identity: &str,
) -> Result<String> {
    if package_id.is_empty() || package_id != package_id.trim() {
        return Err(ServiceDbError::InvalidDbMetadata(
            "runtime DB target must carry a canonical non-empty package ID".to_string(),
        ));
    }
    if declared_collection_identity.is_empty()
        || declared_collection_identity != declared_collection_identity.trim()
    {
        return Err(ServiceDbError::InvalidDbMetadata(
            "runtime DB target must declare a canonical non-empty collection identity".to_string(),
        ));
    }

    let mut hasher = Sha256::new();
    hash_framed_storage_identity_part(&mut hasher, b"skiff-package-collection-storage-identity-v1");
    hash_framed_storage_identity_part(&mut hasher, package_id.as_bytes());
    hash_framed_storage_identity_part(&mut hasher, declared_collection_identity.as_bytes());
    Ok(format!(
        "{SKIFF_PACKAGE_COLLECTION_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    ))
}

fn hash_framed_storage_identity_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn validate_publication_id(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 63
        || value == "std"
        || value != value.trim()
        || value.contains("://")
        || value.starts_with('/')
        || value.ends_with('/')
        || value.contains("//")
        || value.contains('~')
        || value.bytes().any(|byte| byte.is_ascii_control())
        || value
            .bytes()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'))
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    }
    let Some((authority, local)) = value.split_once('/') else {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    };
    validate_publication_authority(value, authority)?;
    if local.is_empty()
        || local
            .split('/')
            .any(|segment| !is_valid_local_segment(segment))
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{value}` must be a publication id"
        )));
    }
    Ok(())
}

fn validate_publication_authority(publication_id: &str, authority: &str) -> Result<()> {
    let labels = authority.split('.').collect::<Vec<_>>();
    if labels.len() < 2 || labels.iter().any(|label| !is_valid_authority_label(label)) {
        return Err(ServiceDbError::Decode(format!(
            "service id `{publication_id}` must be a publication id"
        )));
    }
    Ok(())
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

pub(crate) fn validate_service_database_name(database_name: &str) -> Result<()> {
    if database_name.is_empty() || database_name.len() >= 64 {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` must project to a Mongo database name of 1-63 bytes"
        )));
    }
    if matches!(database_name, "admin" | "local" | "config") {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` projects to a reserved Mongo database name"
        )));
    }
    if database_name
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || database_name.contains(['.', '/', '\\', '"', '$'])
    {
        return Err(ServiceDbError::Decode(format!(
            "service id `{database_name}` projects to a character forbidden in Mongo database names"
        )));
    }
    Ok(())
}

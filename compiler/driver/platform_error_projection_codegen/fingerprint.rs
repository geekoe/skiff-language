use serde::Serialize;
use sha2::{Digest, Sha256};
use skiff_artifact_model::TypeDeclIr;
use skiff_compiler_source::{
    ResolvedPlatformErrorProjectionCatalog, ResolvedPlatformErrorProjectionEntry,
};

use super::PlatformErrorProjectionCodegenError;

pub(super) const CODEC_VERSION: u32 = 1;
pub(super) const REGISTRY_ID: &str = "skiff-platform-error-projections";
pub(super) const REGISTRY_VERSION: u32 = 1;

#[derive(Debug)]
pub(super) struct FingerprintedCatalog<'a> {
    pub(super) entries: Vec<FingerprintedEntry<'a>>,
    pub(super) registry_fingerprint: String,
}

#[derive(Debug)]
pub(super) struct FingerprintedEntry<'a> {
    pub(super) resolved: &'a ResolvedPlatformErrorProjectionEntry,
    pub(super) fingerprint: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryPreimage<'a> {
    schema: &'static str,
    projection_key: &'a str,
    nominal_identity: &'a str,
    canonical_public_type_ir: &'a TypeDeclIr,
    codec_version: u32,
    producer_family: &'a str,
    semantic_adapter_owner: &'a str,
    public_message_policy: &'a str,
    envelope_kind: &'a str,
    fallback_policy: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryPreimage<'a> {
    schema: &'static str,
    registry_id: &'static str,
    registry_version: u32,
    entries: &'a [RegistryEntryPreimage<'a>],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RegistryEntryPreimage<'a> {
    projection_key: &'a str,
    entry_fingerprint: &'a str,
}

pub(super) fn fingerprint_catalog(
    catalog: &ResolvedPlatformErrorProjectionCatalog,
) -> Result<FingerprintedCatalog<'_>, PlatformErrorProjectionCodegenError> {
    assert_ascii_sorted_unique(catalog)?;
    let entries = catalog
        .entries()
        .iter()
        .map(|resolved| {
            let preimage = entry_preimage(resolved)?;
            let fingerprint = sha256_fingerprint(&preimage);
            Ok(FingerprintedEntry {
                resolved,
                fingerprint,
            })
        })
        .collect::<Result<Vec<_>, PlatformErrorProjectionCodegenError>>()?;
    let registry_entries = entries
        .iter()
        .map(|entry| RegistryEntryPreimage {
            projection_key: entry.resolved.projection_key(),
            entry_fingerprint: &entry.fingerprint,
        })
        .collect::<Vec<_>>();
    let registry_preimage = registry_preimage(&registry_entries)?;
    let registry_fingerprint = sha256_fingerprint(&registry_preimage);
    Ok(FingerprintedCatalog {
        entries,
        registry_fingerprint,
    })
}

fn registry_preimage(
    entries: &[RegistryEntryPreimage<'_>],
) -> Result<Vec<u8>, PlatformErrorProjectionCodegenError> {
    skiff_canonical_json::canonical_json_bytes(&RegistryPreimage {
        schema: "skiff-platform-error-projection-registry-v1",
        registry_id: REGISTRY_ID,
        registry_version: REGISTRY_VERSION,
        entries,
    })
    .map_err(|error| PlatformErrorProjectionCodegenError::InvalidCatalog(error.to_string()))
}

pub(super) fn entry_preimage(
    entry: &ResolvedPlatformErrorProjectionEntry,
) -> Result<Vec<u8>, PlatformErrorProjectionCodegenError> {
    skiff_canonical_json::canonical_json_bytes(&EntryPreimage {
        schema: "skiff-platform-error-projection-entry-v1",
        projection_key: entry.projection_key(),
        nominal_identity: entry.nominal_identity(),
        canonical_public_type_ir: entry.canonical_public_type_ir(),
        codec_version: CODEC_VERSION,
        producer_family: entry.producer_family(),
        semantic_adapter_owner: entry.semantic_adapter_owner(),
        public_message_policy: entry.public_message_policy(),
        envelope_kind: entry.envelope_kind(),
        fallback_policy: entry.fallback_policy(),
    })
    .map_err(|error| PlatformErrorProjectionCodegenError::InvalidCatalog(error.to_string()))
}

#[cfg(test)]
pub(super) fn registry_preimage_for_catalog(
    catalog: &FingerprintedCatalog<'_>,
) -> Result<Vec<u8>, PlatformErrorProjectionCodegenError> {
    let entries = catalog
        .entries
        .iter()
        .map(|entry| RegistryEntryPreimage {
            projection_key: entry.resolved.projection_key(),
            entry_fingerprint: &entry.fingerprint,
        })
        .collect::<Vec<_>>();
    registry_preimage(&entries)
}

fn assert_ascii_sorted_unique(
    catalog: &ResolvedPlatformErrorProjectionCatalog,
) -> Result<(), PlatformErrorProjectionCodegenError> {
    let mut previous = None;
    for entry in catalog.entries() {
        let key = entry.projection_key();
        if !key.is_ascii() {
            return Err(PlatformErrorProjectionCodegenError::InvalidCatalog(
                format!("projection key {key:?} is not ASCII"),
            ));
        }
        if key.split('.').any(is_numeric_version_segment) {
            return Err(PlatformErrorProjectionCodegenError::InvalidCatalog(
                format!("projection key {key:?} contains a forbidden numeric version suffix"),
            ));
        }
        if let Some(previous) = previous {
            if previous >= key {
                return Err(PlatformErrorProjectionCodegenError::InvalidCatalog(format!(
                    "projection keys are not strictly ASCII ascending and unique: {previous:?}, {key:?}"
                )));
            }
        }
        previous = Some(key);
    }
    Ok(())
}

fn is_numeric_version_segment(segment: &str) -> bool {
    segment.strip_prefix('v').is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn sha256_fingerprint(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

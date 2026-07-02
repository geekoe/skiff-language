use std::{fs, path::Path};

use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn read_json_file(path: &Path, label: &str) -> anyhow::Result<Value> {
    let bytes = fs::read(path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "failed to parse {} as {label} JSON: {error}",
            path.display()
        )
    })
}

pub(crate) fn value_sha256(value: &Value) -> anyhow::Result<String> {
    let canonical = skiff_artifact_identity::canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| anyhow::anyhow!("failed to serialize artifact JSON: {error}"))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

pub(super) fn is_sha256_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

use std::{fs, path::Path};

use serde_json::{Map, Value};

pub(crate) fn read_json_file(path: &Path, label: &str) -> anyhow::Result<Value> {
    let bytes = fs::read(path)
        .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        anyhow::anyhow!(
            "failed to parse {} as {label} JSON: {error}",
            path.display()
        )
    })
}

pub(crate) fn object_string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_string)
}

pub(crate) fn map_string(object: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    object
        .and_then(|object| object.get(key))
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(crate) fn is_sha256_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

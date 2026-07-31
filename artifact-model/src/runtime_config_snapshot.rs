use std::fmt;

use serde::{de, Deserialize, Deserializer, Serialize};

pub const RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX: &str = "skiff-runtime-config-snapshot-v1";
const RUNTIME_CONFIG_SNAPSHOT_RANDOM_HEX_LEN: usize = 32;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct RuntimeConfigSnapshotId(String);

impl RuntimeConfigSnapshotId {
    pub fn parse(value: impl Into<String>) -> Result<Self, RuntimeConfigSnapshotIdParseError> {
        let value = value.into();
        validate_runtime_config_snapshot_id(&value)
            .map_err(|_| RuntimeConfigSnapshotIdParseError(value.clone()))?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn random_suffix(&self) -> &str {
        self.0
            .strip_prefix(&format!("{RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:"))
            .expect("validated runtime config snapshot id")
    }
}

impl fmt::Debug for RuntimeConfigSnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RuntimeConfigSnapshotId")
            .field(&self.0)
            .finish()
    }
}

impl fmt::Display for RuntimeConfigSnapshotId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RuntimeConfigSnapshotId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigSnapshotIdParseError(String);

impl fmt::Display for RuntimeConfigSnapshotIdParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "runtime config snapshot id {:?} must use {RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:<32 lowercase hex>",
            self.0
        )
    }
}

impl std::error::Error for RuntimeConfigSnapshotIdParseError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfigSnapshotRef {
    pub snapshot_id: RuntimeConfigSnapshotId,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeConfigSnapshotRef {
    snapshot_id: RuntimeConfigSnapshotId,
}

impl<'de> Deserialize<'de> for RuntimeConfigSnapshotRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeConfigSnapshotRef::deserialize(deserializer)?;
        let reference = Self {
            snapshot_id: raw.snapshot_id,
        };
        validate_runtime_config_snapshot_ref(&reference).map_err(de::Error::custom)?;
        Ok(reference)
    }
}

pub fn validate_runtime_config_snapshot_id(value: &str) -> Result<(), String> {
    let expected_prefix = format!("{RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:");
    let Some(random_hex) = value.strip_prefix(&expected_prefix) else {
        return Err(invalid_id_message());
    };
    if random_hex.len() != RUNTIME_CONFIG_SNAPSHOT_RANDOM_HEX_LEN
        || !random_hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(invalid_id_message());
    }
    Ok(())
}

pub fn validate_runtime_config_snapshot_ref(
    reference: &RuntimeConfigSnapshotRef,
) -> Result<(), String> {
    validate_runtime_config_snapshot_id(reference.snapshot_id.as_str())
}

fn invalid_id_message() -> String {
    format!("snapshotId must use {RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:<32 lowercase hex>")
}

#[cfg(test)]
mod tests;

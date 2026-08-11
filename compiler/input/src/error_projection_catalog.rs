use serde::{Deserialize, Serialize};

pub const PLATFORM_ERROR_PROJECTION_CATALOG_SCHEMA_VERSION: &str =
    "skiff-platform-error-projection-catalog-v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformErrorProjectionCatalog {
    schema_version: String,
    entries: Vec<PlatformErrorProjectionCatalogEntry>,
}

impl PlatformErrorProjectionCatalog {
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    pub fn entries(&self) -> &[PlatformErrorProjectionCatalogEntry] {
        &self.entries
    }

    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let catalog = serde_yaml::from_str::<Self>(text).map_err(|error| error.to_string())?;
        catalog.validate()?;
        Ok(catalog)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != PLATFORM_ERROR_PROJECTION_CATALOG_SCHEMA_VERSION {
            return Err(format!(
                "schemaVersion must be {PLATFORM_ERROR_PROJECTION_CATALOG_SCHEMA_VERSION}"
            ));
        }

        for entry in &self.entries {
            entry.validate()?;
        }
        for adjacent in self.entries.windows(2) {
            let previous = adjacent[0].projection_key();
            let current = adjacent[1].projection_key();
            match previous.cmp(current) {
                std::cmp::Ordering::Less => {}
                std::cmp::Ordering::Equal => {
                    return Err(format!("duplicate projectionKey {current}"));
                }
                std::cmp::Ordering::Greater => {
                    return Err(format!(
                        "entries must be in strictly ascending ASCII projectionKey order: {previous} precedes {current}"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PlatformErrorProjectionCatalogEntry {
    projection_key: String,
    producer_family: String,
    semantic_adapter_owner: String,
    public_message_policy: String,
    envelope_kind: String,
    fallback_policy: String,
}

impl PlatformErrorProjectionCatalogEntry {
    pub fn projection_key(&self) -> &str {
        &self.projection_key
    }

    pub fn producer_family(&self) -> &str {
        &self.producer_family
    }

    pub fn semantic_adapter_owner(&self) -> &str {
        &self.semantic_adapter_owner
    }

    pub fn public_message_policy(&self) -> &str {
        &self.public_message_policy
    }

    pub fn envelope_kind(&self) -> &str {
        &self.envelope_kind
    }

    pub fn fallback_policy(&self) -> &str {
        &self.fallback_policy
    }

    fn validate(&self) -> Result<(), String> {
        validate_token("projectionKey", &self.projection_key)?;
        if !self.projection_key.contains('.') {
            return Err(format!(
                "projectionKey {} must contain at least one dot",
                self.projection_key
            ));
        }
        if has_numeric_version_suffix(&self.projection_key) {
            return Err(format!(
                "projectionKey {} must not have a .v<N> version suffix",
                self.projection_key
            ));
        }

        validate_token("producerFamily", &self.producer_family)?;
        validate_token("semanticAdapterOwner", &self.semantic_adapter_owner)?;
        validate_token("publicMessagePolicy", &self.public_message_policy)?;
        validate_token("envelopeKind", &self.envelope_kind)?;
        validate_token("fallbackPolicy", &self.fallback_policy)
    }
}

fn validate_token(label: &str, token: &str) -> Result<(), String> {
    if token.is_empty() || token.len() > 128 {
        return Err(format!("{label} must be between 1 and 128 bytes"));
    }
    if token.trim() != token {
        return Err(format!("{label} must not have surrounding whitespace"));
    }
    if !token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "{label} must contain only ASCII characters [A-Za-z0-9._-]"
        ));
    }
    Ok(())
}

fn has_numeric_version_suffix(key: &str) -> bool {
    let Some((_, suffix)) = key.rsplit_once(".v") else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
#[path = "error_projection_catalog/tests.rs"]
mod tests;

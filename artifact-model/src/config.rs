use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use serde::{Deserialize, Serialize};

use crate::{
    compile_requirements::{PackageConfigAccess, PackageConfigRequirement},
    metadata::MetadataValue,
};

pub const CONFIG_SHAPE_SCHEMA_VERSION: &str = "skiff-config-shape-v1";

/// Typed owner for package configuration projection facts. The contained
/// metadata remains open because each named config projection has its own
/// schema, while callable effects are intentionally excluded from this owner.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ConfigMetadataFacts(BTreeMap<String, MetadataValue>);

impl ConfigMetadataFacts {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<BTreeMap<String, MetadataValue>> for ConfigMetadataFacts {
    fn from(facts: BTreeMap<String, MetadataValue>) -> Self {
        Self(facts)
    }
}

impl std::ops::Deref for ConfigMetadataFacts {
    type Target = BTreeMap<String, MetadataValue>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ConfigMetadataFacts {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigShape {
    pub schema_version: String,
    pub entries: Vec<ConfigShapeEntry>,
}

impl ConfigShape {
    pub fn empty() -> Self {
        Self {
            schema_version: CONFIG_SHAPE_SCHEMA_VERSION.to_string(),
            entries: Vec::new(),
        }
    }

    pub fn validate_schema_version(&self) -> Result<(), ConfigShapeSchemaVersionError> {
        if self.schema_version == CONFIG_SHAPE_SCHEMA_VERSION {
            Ok(())
        } else {
            Err(ConfigShapeSchemaVersionError {
                actual: self.schema_version.clone(),
            })
        }
    }
}

impl Default for ConfigShape {
    fn default() -> Self {
        Self::empty()
    }
}

/// Constructs the canonical runtime config shape from package requirements.
///
/// `PackageRuntimeRequirements.config` owns these facts. Package and future
/// deployment consumers must use this conversion instead of reconstructing a
/// shape from source seeds, service config, or presentation DTOs.
pub fn config_shape_from_package_requirements(
    requirements: &[PackageConfigRequirement],
) -> Result<ConfigShape, PackageConfigShapeError> {
    let mut seen_paths = BTreeSet::new();
    let mut entries = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        if !seen_paths.insert(requirement.path.as_str()) {
            return Err(PackageConfigShapeError::DuplicatePath {
                path: requirement.path.clone(),
            });
        }
        let (value_type, required) = match &requirement.access {
            PackageConfigAccess::Presence => continue,
            PackageConfigAccess::Optional { value_type } => (value_type, false),
            PackageConfigAccess::Required { value_type } => (value_type, true),
        };
        let ty = ConfigShapeValueType::try_from(value_type.as_str()).map_err(|source| {
            PackageConfigShapeError::InvalidValueType {
                path: requirement.path.clone(),
                source,
            }
        })?;
        entries.push(ConfigShapeEntry {
            path: requirement.path.clone(),
            ty,
            required,
        });
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(ConfigShape {
        schema_version: CONFIG_SHAPE_SCHEMA_VERSION.to_string(),
        entries,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageConfigShapeError {
    InvalidValueType {
        path: String,
        source: ConfigShapeValueTypeParseError,
    },
    DuplicatePath {
        path: String,
    },
}

impl fmt::Display for PackageConfigShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidValueType { path, source } => {
                write!(formatter, "config requirement {path} is invalid: {source}")
            }
            Self::DuplicatePath { path } => {
                write!(
                    formatter,
                    "config requirement path {path} is declared more than once"
                )
            }
        }
    }
}

impl std::error::Error for PackageConfigShapeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidValueType { source, .. } => Some(source),
            Self::DuplicatePath { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigShapeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub ty: ConfigShapeValueType,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigShapeValueType {
    #[serde(rename = "string")]
    String,
    #[serde(rename = "number")]
    Number,
    #[serde(rename = "bool")]
    Bool,
    #[serde(rename = "Json")]
    Json,
    #[serde(rename = "JsonObject")]
    JsonObject,
}

impl ConfigShapeValueType {
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "bool",
            Self::Json => "Json",
            Self::JsonObject => "JsonObject",
        }
    }
}

impl fmt::Display for ConfigShapeValueType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_wire_str())
    }
}

impl FromStr for ConfigShapeValueType {
    type Err = ConfigShapeValueTypeParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "string" => Ok(Self::String),
            "number" => Ok(Self::Number),
            "bool" => Ok(Self::Bool),
            "Json" => Ok(Self::Json),
            "JsonObject" => Ok(Self::JsonObject),
            other => Err(ConfigShapeValueTypeParseError {
                value: other.to_string(),
            }),
        }
    }
}

impl TryFrom<&str> for ConfigShapeValueType {
    type Error = ConfigShapeValueTypeParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        value.parse()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigShapeValueTypeParseError {
    value: String,
}

impl fmt::Display for ConfigShapeValueTypeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "config shape value type {} is unsupported; expected string, number, bool, Json, or JsonObject",
            self.value
        )
    }
}

impl std::error::Error for ConfigShapeValueTypeParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigShapeSchemaVersionError {
    actual: String,
}

impl fmt::Display for ConfigShapeSchemaVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "configShape schemaVersion must be {CONFIG_SHAPE_SCHEMA_VERSION}, got {}",
            self.actual
        )
    }
}

impl std::error::Error for ConfigShapeSchemaVersionError {}

#[cfg(test)]
mod tests;

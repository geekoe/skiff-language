use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

pub const CONFIG_SHAPE_SCHEMA_VERSION: &str = "skiff-config-shape-v1";

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
mod tests {
    use serde_json::json;

    use super::{ConfigShape, ConfigShapeEntry, ConfigShapeValueType, CONFIG_SHAPE_SCHEMA_VERSION};

    #[test]
    fn config_shape_value_type_uses_canonical_wire_strings() {
        let shape = ConfigShape {
            schema_version: CONFIG_SHAPE_SCHEMA_VERSION.to_string(),
            entries: vec![
                ConfigShapeEntry {
                    path: "text".to_string(),
                    ty: ConfigShapeValueType::String,
                    required: true,
                },
                ConfigShapeEntry {
                    path: "raw".to_string(),
                    ty: ConfigShapeValueType::Json,
                    required: false,
                },
                ConfigShapeEntry {
                    path: "object".to_string(),
                    ty: ConfigShapeValueType::JsonObject,
                    required: true,
                },
            ],
        };

        assert_eq!(
            serde_json::to_value(&shape).expect("config shape should serialize"),
            json!({
                "schemaVersion": "skiff-config-shape-v1",
                "entries": [
                    { "path": "text", "type": "string", "required": true },
                    { "path": "raw", "type": "Json", "required": false },
                    { "path": "object", "type": "JsonObject", "required": true }
                ]
            })
        );
    }
}

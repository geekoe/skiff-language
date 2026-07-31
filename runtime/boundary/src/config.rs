use serde_json::Value;
use skiff_artifact_model::ConfigShapeValueType;

use crate::{
    error::{Result as RuntimeResult, RuntimeError},
    type_descriptor::{RuntimeTypeNode, RuntimeTypePlan},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigTargetType {
    value_type: ConfigShapeValueType,
}

pub fn target_type_from_type_plan(
    target: &str,
    type_arg: Option<&RuntimeTypePlan>,
) -> RuntimeResult<ConfigTargetType> {
    ConfigTargetType::from_type_plan(target, type_arg)
}

pub fn target_type_from_shape_type(ty: ConfigShapeValueType) -> ConfigTargetType {
    ConfigTargetType::from_shape_type(ty)
}

impl ConfigTargetType {
    fn from_type_plan(target: &str, type_arg: Option<&RuntimeTypePlan>) -> RuntimeResult<Self> {
        let type_arg = type_arg.ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("{target} call is missing typeArgs[0]"))
        })?;
        Ok(Self {
            value_type: config_value_type_from_runtime_type_plan(target, type_arg)?,
        })
    }

    fn from_shape_type(ty: ConfigShapeValueType) -> Self {
        Self { value_type: ty }
    }

    pub fn decode_value(self, target: &str, path: &str, value: &Value) -> RuntimeResult<Value> {
        match self.value_type {
            ConfigShapeValueType::String => value
                .as_str()
                .map(|value| Value::String(value.to_string()))
                .ok_or_else(|| config_type_error(target, path, "string")),
            ConfigShapeValueType::Number => {
                if value.is_number() {
                    Ok(value.clone())
                } else {
                    Err(config_type_error(target, path, "number"))
                }
            }
            ConfigShapeValueType::Bool => value
                .as_bool()
                .map(Value::Bool)
                .ok_or_else(|| config_type_error(target, path, "bool")),
            ConfigShapeValueType::Json => Ok(value.clone()),
            ConfigShapeValueType::JsonObject => {
                if value.is_object() {
                    Ok(value.clone())
                } else {
                    Err(config_type_error(target, path, "JsonObject"))
                }
            }
        }
    }

    pub fn matches_value(self, value: &Value) -> bool {
        match self.value_type {
            ConfigShapeValueType::String => value.is_string(),
            ConfigShapeValueType::Number => value.is_number(),
            ConfigShapeValueType::Bool => value.is_boolean(),
            ConfigShapeValueType::Json => true,
            ConfigShapeValueType::JsonObject => value.is_object(),
        }
    }
}

fn config_value_type_from_runtime_type_plan(
    target: &str,
    plan: &RuntimeTypePlan,
) -> RuntimeResult<ConfigShapeValueType> {
    match plan.node() {
        RuntimeTypeNode::Alias(inner) => config_value_type_from_runtime_type_plan(target, inner),
        RuntimeTypeNode::Nullable(_) => Err(RuntimeError::InvalidArtifact(format!(
            "{target} typeArgs[0] must be non-nullable; use config.optional<T> for optional config reads"
        ))),
        RuntimeTypeNode::String => Ok(ConfigShapeValueType::String),
        RuntimeTypeNode::Number => Ok(ConfigShapeValueType::Number),
        RuntimeTypeNode::Bool => Ok(ConfigShapeValueType::Bool),
        RuntimeTypeNode::Json => Ok(ConfigShapeValueType::Json),
        RuntimeTypeNode::JsonObject => Ok(ConfigShapeValueType::JsonObject),
        _ => {
            let ty = plan.named_type_name().unwrap_or_else(|| plan.label());
            Err(RuntimeError::InvalidArtifact(format!(
                "{target} typeArgs[0] type {ty} is unsupported"
            )))
        }
    }
}

fn config_type_error(target: &str, path: &str, expected: &str) -> RuntimeError {
    RuntimeError::decode_target(target, format!("path {path} must be a {expected}"))
}

#[cfg(test)]
mod tests;

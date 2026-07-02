//! Runtime config view helpers used by direct runtime integrations.

use anyhow::{anyhow, Result};
use serde_json::{Map, Value};
use skiff_artifact_model::{ConfigShape, ConfigShapeEntry};
use skiff_runtime_boundary::{
    config as config_boundary,
    contract::RuntimeBoundaryContract,
    json::{has_heap_handle, ProtocolJsonObject},
    plan::BoundaryUse,
    type_descriptor::RuntimeTypePlan,
    value::is_internal_metadata_key,
};
use skiff_runtime_model::request_heap::RequestHeap;

use crate::error::{Result as RuntimeResult, RuntimeError};

#[derive(Debug, Clone)]
pub struct RuntimeConfigView {
    resolved_config: Value,
    _redacted_resolved_config: Option<Value>,
    config_shape: ConfigShape,
}

impl RuntimeConfigView {
    #[cfg(any(test, feature = "test-support"))]
    pub fn empty() -> Self {
        Self {
            resolved_config: Value::Object(Map::new()),
            _redacted_resolved_config: None,
            config_shape: ConfigShape::empty(),
        }
    }

    pub fn empty_with_shape(config_shape: ConfigShape) -> Result<Self> {
        Self::from_resolved_config(Value::Object(Map::new()), config_shape)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn from_value(value: Value) -> Self {
        let resolved_config = match value {
            Value::Object(_) => value,
            _ => Value::Object(Map::new()),
        };
        Self {
            resolved_config,
            _redacted_resolved_config: None,
            config_shape: ConfigShape::empty(),
        }
    }

    pub fn from_resolved_config(resolved_config: Value, config_shape: ConfigShape) -> Result<Self> {
        Self::from_resolved_config_parts(resolved_config, config_shape, None)
    }

    pub fn from_resolved_config_with_redaction(
        resolved_config: Value,
        config_shape: ConfigShape,
        redacted_resolved_config: Value,
    ) -> Result<Self> {
        Self::from_resolved_config_parts(
            resolved_config,
            config_shape,
            Some(redacted_resolved_config),
        )
    }

    fn from_resolved_config_parts(
        resolved_config: Value,
        config_shape: ConfigShape,
        redacted_resolved_config: Option<Value>,
    ) -> Result<Self> {
        let resolved_config = match resolved_config {
            value @ Value::Object(_) => value,
            _ => return Err(anyhow!("resolvedConfig must be a JSON object")),
        };
        let redacted_resolved_config = match redacted_resolved_config {
            Some(Value::Object(_)) => redacted_resolved_config,
            Some(_) => return Err(anyhow!("redactedResolvedConfig must be a JSON object")),
            None => None,
        };
        validate_resolved_config_shape(&resolved_config, &config_shape)?;
        Ok(Self {
            resolved_config,
            _redacted_resolved_config: redacted_resolved_config,
            config_shape,
        })
    }

    pub fn resolved_config_value(&self) -> &Value {
        &self.resolved_config
    }

    pub fn config_shape(&self) -> &ConfigShape {
        &self.config_shape
    }

    pub fn dispatch_typed_config_target(
        &self,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> RuntimeResult<Value> {
        if !matches!(target, "config.require" | "config.optional" | "config.has") {
            return Err(RuntimeError::Unsupported(format!(
                "unsupported config native target {target}"
            )));
        }
        let path = config_path_arg(target, args)?;
        match target {
            "config.require" => {
                let target_type = config_boundary::target_type_from_type_plan(target, type_arg)?;
                let Some(value) = self.get_path(path) else {
                    return Err(missing_required_error(target, path));
                };
                if value.is_null() {
                    return Err(missing_required_error(target, path));
                }
                Ok(target_type.decode_value(target, path, value)?)
            }
            "config.optional" => {
                let target_type = config_boundary::target_type_from_type_plan(target, type_arg)?;
                let Some(value) = self.get_path(path) else {
                    return Ok(Value::Null);
                };
                if value.is_null() {
                    return Ok(Value::Null);
                }
                Ok(target_type.decode_value(target, path, value)?)
            }
            "config.has" => Ok(Value::Bool(
                self.get_path(path).is_some_and(|value| !value.is_null()),
            )),
            _ => unreachable!("unsupported config target rejected before argument decode"),
        }
    }

    fn get_path(&self, path: &str) -> Option<&Value> {
        config_path_value(&self.resolved_config, path)
    }
}

pub fn from_wire_json_plan(
    value: Value,
    expected_type: Option<&RuntimeTypePlan>,
) -> RuntimeResult<Value> {
    let value = sanitize_external_json(value)?;
    let expected_type =
        expected_type.ok_or_else(|| missing_boundary_type("external JSON decode"))?;
    let mut heap = RequestHeap::default();
    let codec = RuntimeBoundaryContract::default().codec_for_expected(
        expected_type,
        BoundaryUse::TypedJson,
        "external JSON decode",
    );
    let runtime_value = codec.from_wire_json(&value, &mut heap)?;
    let wire = codec.to_wire_json(&runtime_value, &mut heap)?;
    sanitize_protocol_json(wire)
}

pub fn materialize_json(value: Value) -> RuntimeResult<Value> {
    sanitize_protocol_json(value)
}

pub fn materialize_internal_json(value: Value) -> RuntimeResult<Value> {
    sanitize_protocol_json_with_options(value, true)
}

pub fn sanitize_wire_json(value: Value) -> RuntimeResult<Value> {
    sanitize_external_json(value)
}

fn missing_boundary_type(boundary: &str) -> RuntimeError {
    RuntimeError::invalid_artifact(format!(
        "{boundary} boundary is missing expected type descriptor"
    ))
}

fn sanitize_external_json(value: Value) -> RuntimeResult<Value> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(sanitize_external_json)
            .collect::<RuntimeResult<Vec<_>>>()
            .map(Value::Array),
        Value::Object(object) => {
            if ProtocolJsonObject::classify(&object).contains_internal_runtime_handle() {
                return Err(RuntimeError::Decode(
                    "wire JSON contains an internal runtime handle".to_string(),
                ));
            }
            let mut sanitized = Map::new();
            for (key, value) in object {
                sanitized.insert(key, sanitize_external_json(value)?);
            }
            Ok(Value::Object(sanitized))
        }
        other => Ok(other),
    }
}

fn sanitize_protocol_json(value: Value) -> RuntimeResult<Value> {
    sanitize_protocol_json_with_options(value, false)
}

fn sanitize_protocol_json_with_options(
    value: Value,
    allow_stream_handles: bool,
) -> RuntimeResult<Value> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|item| sanitize_protocol_json_with_options(item, allow_stream_handles))
            .collect::<RuntimeResult<Vec<_>>>()
            .map(Value::Array),
        Value::Object(object) => {
            match ProtocolJsonObject::classify(&object) {
                ProtocolJsonObject::InternalRuntimeHandle => {
                    if has_heap_handle(&object) {
                        return Err(RuntimeError::Decode(
                            "protocol JSON contains an internal runtime handle".to_string(),
                        ));
                    }
                }
                ProtocolJsonObject::StreamHandle if allow_stream_handles => {
                    return Ok(Value::Object(object));
                }
                object if object.contains_internal_runtime_handle() => {
                    return Err(RuntimeError::Decode(
                        "protocol JSON contains an internal runtime handle".to_string(),
                    ));
                }
                ProtocolJsonObject::StreamHandle => {
                    return Err(RuntimeError::Decode(
                        "protocol JSON contains an internal runtime handle".to_string(),
                    ));
                }
                ProtocolJsonObject::Bytes { .. } => return Ok(Value::Object(object)),
                ProtocolJsonObject::PlainObject => {}
            }
            let mut sanitized = Map::new();
            for (key, value) in object {
                if is_internal_metadata_key(&key) {
                    continue;
                }
                sanitized.insert(
                    key,
                    sanitize_protocol_json_with_options(value, allow_stream_handles)?,
                );
            }
            Ok(Value::Object(sanitized))
        }
        other => Ok(other),
    }
}

fn validate_resolved_config_shape(
    resolved_config: &Value,
    config_shape: &ConfigShape,
) -> Result<()> {
    config_shape.validate_schema_version()?;
    for entry in &config_shape.entries {
        validate_config_shape_entry(resolved_config, entry)?;
    }
    Ok(())
}

fn validate_config_shape_entry(resolved_config: &Value, entry: &ConfigShapeEntry) -> Result<()> {
    if entry.path.trim().is_empty() {
        return Err(anyhow!("configShape entries require non-empty path"));
    }
    let target_type = config_boundary::target_type_from_shape_type(entry.ty);
    let Some(value) = config_path_value(resolved_config, &entry.path) else {
        if entry.required {
            return Err(config_activation_missing_error(&entry.path));
        }
        return Ok(());
    };
    if value.is_null() {
        if entry.required {
            return Err(config_activation_missing_error(&entry.path));
        }
        return Ok(());
    }
    target_type
        .matches_value(value)
        .then_some(())
        .ok_or_else(|| {
            anyhow!(
                "configShape entry path {} must be a {}",
                entry.path,
                entry.ty
            )
        })
}

fn config_activation_missing_error(path: &str) -> anyhow::Error {
    anyhow!("configShape entry path {path} required value is missing or null")
}

fn config_path_arg<'a>(target: &str, args: &'a [Value]) -> RuntimeResult<&'a str> {
    if args.len() != 1 {
        return Err(RuntimeError::decode_target(
            target,
            "requires exactly one path argument",
        ));
    }
    args.first()
        .and_then(Value::as_str)
        .ok_or_else(|| RuntimeError::decode_target(target, "path argument must be a string"))
}

fn missing_required_error(target: &str, path: &str) -> RuntimeError {
    RuntimeError::decode_target(
        target,
        format!("path {path} required value is missing or null"),
    )
}

fn path_segments(path: &str) -> Vec<&str> {
    path.split('.')
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn config_path_value<'a>(resolved_config: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = resolved_config;
    for segment in path_segments(path) {
        current = current.get(segment)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests;

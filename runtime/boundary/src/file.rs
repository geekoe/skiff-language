use serde_json::{json, Value};

use crate::{
    error::{Result, RuntimeError},
    value::bytes_value,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileCreateOptions {
    pub content_type: Option<String>,
    pub purpose: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableFileRef {
    pub id: String,
    pub size: i64,
    pub sha256: String,
    pub content_type: Option<String>,
}

pub fn immutable_file_wire(file: ImmutableFileRef) -> Value {
    json!({
        "id": file.id,
        "size": file.size,
        "sha256": file.sha256,
        "contentType": file.content_type,
    })
}

pub fn immutable_file_from_wire(value: &Value, target: &str) -> Result<ImmutableFileRef> {
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::Decode(format!("{target} file must be an object")))?;
    let id = required_string(object, "id", target)?;
    let sha256 = required_string(object, "sha256", target)?;
    let size = object
        .get("size")
        .and_then(Value::as_i64)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} file.size must be an integer")))?;
    if size < 0 {
        return Err(RuntimeError::Decode(format!(
            "{target} file.size must be non-negative"
        )));
    }
    let content_type = optional_string(object, "contentType", target)?;
    Ok(ImmutableFileRef {
        id,
        size,
        sha256,
        content_type,
    })
}

pub fn create_options_from_wire(
    value: Option<&Value>,
    default_content_type: Option<&str>,
    target: &str,
) -> Result<FileCreateOptions> {
    let Some(value) = value else {
        return Ok(FileCreateOptions {
            content_type: default_content_type.map(str::to_string),
            purpose: None,
        });
    };
    if value.is_null() {
        return Ok(FileCreateOptions {
            content_type: default_content_type.map(str::to_string),
            purpose: None,
        });
    }
    let object = value
        .as_object()
        .ok_or_else(|| RuntimeError::Decode(format!("{target} options must be an object")))?;
    Ok(FileCreateOptions {
        content_type: optional_string(object, "contentType", target)?
            .or_else(|| default_content_type.map(str::to_string)),
        purpose: optional_string(object, "purpose", target)?,
    })
}

pub fn file_decode_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::file_error(format!("std.file decode error: {}", message.into()))
}

pub fn bytes_wire(bytes: &[u8]) -> Value {
    bytes_value(bytes)
}

fn required_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    target: &str,
) -> Result<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| RuntimeError::Decode(format!("{target} file.{field} must be a string")))
}

fn optional_string(
    object: &serde_json::Map<String, Value>,
    field: &str,
    target: &str,
) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(RuntimeError::Decode(format!(
            "{target} options.{field} must be a string or null"
        ))),
    }
}

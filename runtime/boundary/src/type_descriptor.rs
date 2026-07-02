//! Runtime type-plan helpers.
//!
//! Production code builds [`RuntimeTypePlan`] from linked/artifact type refs.
//! The legacy JSON descriptor parser is retained only for tests and
//! `test-support` fixtures.

#[cfg(any(test, feature = "test-support"))]
use serde_json::{json, Map, Value};

pub use skiff_runtime_model::type_plan::{
    RuntimeRecordFieldPlan, RuntimeTypeIdentityPlan, RuntimeTypeNode, RuntimeTypePlan,
};

#[cfg(any(test, feature = "test-support"))]
use crate::error::Result;
use crate::error::RuntimeError;

#[derive(Clone, Copy, Debug)]
pub enum RuntimeTypeNameError {
    MissingType,
    NullableMissingInner,
}

#[cfg(any(test, feature = "test-support"))]
pub trait RuntimeTypePlanDescriptorExt: Sized {
    /// Build a runtime plan from the legacy JSON descriptor shape.
    ///
    /// This is a bridge for old descriptor-oracle compatibility. Prefer
    /// `RuntimeTypePlanLinkedExt::from_linked` or artifact-native builders on
    /// production paths that already have structured type refs.
    fn from_descriptor(descriptor: &Value) -> Result<Self>;
}

#[cfg(any(test, feature = "test-support"))]
impl RuntimeTypePlanDescriptorExt for RuntimeTypePlan {
    fn from_descriptor(descriptor: &Value) -> Result<Self> {
        legacy_descriptor_bridge_plan(descriptor)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn legacy_descriptor_bridge_plan(descriptor: &Value) -> Result<RuntimeTypePlan> {
    let node = if let Some(target) = alias_target(descriptor)? {
        RuntimeTypeNode::Alias(Box::new(RuntimeTypePlan::from_descriptor(&target)?))
    } else if let Some(inner) = nullable_inner(descriptor) {
        RuntimeTypeNode::Nullable(Box::new(RuntimeTypePlan::from_descriptor(&inner)?))
    } else if let Some(types) = union_types(descriptor) {
        RuntimeTypeNode::Union(
            types
                .iter()
                .map(RuntimeTypePlan::from_descriptor)
                .collect::<Result<Vec<_>>>()?,
        )
    } else if let Some(value) = literal_string(descriptor) {
        RuntimeTypeNode::LiteralString(value.to_string())
    } else if let Some((type_name, payload)) = representation_descriptor(descriptor) {
        RuntimeTypeNode::Representation {
            type_name,
            payload: Box::new(RuntimeTypePlan::from_descriptor(&payload)?),
        }
    } else if is_json_descriptor(descriptor) {
        RuntimeTypeNode::Json
    } else if is_json_object_descriptor(descriptor) {
        RuntimeTypeNode::JsonObject
    } else if is_bytes_descriptor(descriptor) {
        RuntimeTypeNode::Bytes
    } else if is_date_descriptor(descriptor) {
        RuntimeTypeNode::Date
    } else if is_string_descriptor(descriptor) {
        RuntimeTypeNode::String
    } else if is_bool_descriptor(descriptor) {
        RuntimeTypeNode::Bool
    } else if is_integer_descriptor(descriptor) {
        RuntimeTypeNode::Integer
    } else if is_number_descriptor(descriptor) {
        RuntimeTypeNode::Number
    } else if is_null_descriptor(descriptor) {
        RuntimeTypeNode::Null
    } else if let Some(item_type) = stream_item_type(descriptor) {
        RuntimeTypeNode::Stream(Box::new(RuntimeTypePlan::from_descriptor(item_type)?))
    } else if let Some(item_type) = array_item_type(descriptor) {
        RuntimeTypeNode::Array(Box::new(RuntimeTypePlan::from_descriptor(&item_type)?))
    } else if let Some((key_type, value_type)) = map_types(descriptor) {
        RuntimeTypeNode::Map {
            key: Box::new(RuntimeTypePlan::from_descriptor(&key_type)?),
            value: Box::new(RuntimeTypePlan::from_descriptor(&value_type)?),
        }
    } else if let Some(fields) = db_result_fields(descriptor)? {
        runtime_type_plan_record_node(fields, descriptor)?
    } else if let Some(fields) = record_fields(descriptor)? {
        runtime_type_plan_record_node(fields, descriptor)?
    } else if let Some(node) = std_runtime_builtin_node_from_descriptor(descriptor) {
        node?
    } else {
        RuntimeTypeNode::Unknown
    };
    Ok(RuntimeTypePlan {
        label: descriptor_label(descriptor),
        named_type_name: named_type_name(descriptor),
        identity: runtime_type_identity_plan(descriptor),
        node,
    })
}

#[cfg(any(test, feature = "test-support"))]
fn runtime_type_plan_record_node(
    fields: Vec<RuntimeRecordFieldPlan>,
    descriptor: &Value,
) -> Result<RuntimeTypeNode> {
    Ok(RuntimeTypeNode::Record {
        fields,
        boundary_record_kind: boundary_record_kind(descriptor),
    })
}

#[cfg(any(test, feature = "test-support"))]
pub fn nullable_inner(expected_type: &Value) -> Option<Value> {
    let object = expected_type.as_object()?;
    if object.get("nullable").and_then(Value::as_bool) == Some(true) {
        let mut inner = object.clone();
        inner.remove("nullable");
        return Some(Value::Object(inner));
    }
    match object.get("kind").and_then(Value::as_str) {
        Some("nullable") => object.get("inner").cloned(),
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn union_types(expected_type: &Value) -> Option<Vec<Value>> {
    let object = expected_type.as_object()?;
    match object.get("kind").and_then(Value::as_str) {
        Some("union") => object
            .get("items")
            .or_else(|| object.get("variants"))?
            .as_array()
            .cloned(),
        _ => object.get("oneOf")?.as_array().cloned(),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn literal_string(expected_type: &Value) -> Option<&str> {
    let object = expected_type.as_object()?;
    match object.get("kind").and_then(Value::as_str) {
        Some("literal") => literal_ir_string_value(object.get("value")?),
        _ => {
            let values = object.get("enum")?.as_array()?;
            (values.len() == 1).then(|| values[0].as_str()).flatten()
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
fn literal_ir_string_value(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    (object.get("kind").and_then(Value::as_str) == Some("string"))
        .then(|| object.get("value")?.as_str())
        .flatten()
}

#[cfg(any(test, feature = "test-support"))]
fn literal_ir_json_value(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    match object.get("kind").and_then(Value::as_str)? {
        "null" => Some(Value::Null),
        "bool" | "number" | "string" => object.get("value").cloned(),
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn alias_target(expected_type: &Value) -> Result<Option<Value>> {
    let Some(object) = expected_type.as_object() else {
        return Ok(None);
    };
    let kind = object.get("kind").and_then(Value::as_str);
    match kind {
        Some("alias") => object.get("target").cloned().map(Some).ok_or_else(|| {
            RuntimeError::InvalidArtifact("alias type descriptor is missing target".to_string())
        }),
        _ => Ok(None),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn representation_descriptor(expected_type: &Value) -> Option<(String, Value)> {
    let object = expected_type.as_object()?;
    let kind = object.get("kind").and_then(Value::as_str);
    if kind == Some("representation") {
        let type_name = object.get("name").and_then(Value::as_str)?.to_string();
        let payload = object.get("representation").cloned()?;
        return Some((type_name, payload));
    }
    None
}

#[cfg(any(test, feature = "test-support"))]
pub fn array_item_type(expected_type: &Value) -> Option<Value> {
    if let Some((root, args)) = generic_type_parts(expected_type) {
        if bare_type_name(&root) == "Array" && args.len() == 1 {
            return Some(args[0].clone());
        }
    }
    let object = expected_type.as_object()?;
    if object.get("type").and_then(Value::as_str) == Some("array") {
        return object.get("items").cloned();
    }
    None
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_stream_descriptor(expected_type: &Value) -> bool {
    generic_type_parts(expected_type)
        .is_some_and(|(root, args)| bare_type_name(&root) == "Stream" && args.len() == 1)
}

#[cfg(any(test, feature = "test-support"))]
pub fn stream_item_type(expected_type: &Value) -> Option<&Value> {
    let object = expected_type.as_object()?;
    let name = object.get("name").and_then(Value::as_str)?;
    if object.get("kind").and_then(Value::as_str) != Some("builtin")
        || bare_type_name(name) != "Stream"
    {
        return None;
    }
    object
        .get("args")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .filter(|value| !value.is_null())
}

#[cfg(any(test, feature = "test-support"))]
pub fn db_result_fields(expected_type: &Value) -> Result<Option<Vec<RuntimeRecordFieldPlan>>> {
    let Some((root, args)) = generic_type_parts(expected_type) else {
        return Ok(None);
    };
    match bare_type_name(&root) {
        "DbInsertManyResult" if args.is_empty() => Ok(Some(vec![descriptor_record_field(
            "insertedCount",
            &number_descriptor(),
            true,
            None,
        )?])),
        "DbUpdateManyResult" if args.is_empty() => Ok(Some(vec![
            descriptor_record_field("matchedCount", &number_descriptor(), true, None)?,
            descriptor_record_field("modifiedCount", &number_descriptor(), true, None)?,
        ])),
        "DbDeleteManyResult" if args.is_empty() => Ok(Some(vec![descriptor_record_field(
            "deletedCount",
            &number_descriptor(),
            true,
            None,
        )?])),
        "DbUpsertResult" if args.len() == 1 => Ok(Some(vec![
            descriptor_record_field("value", &args[0], true, None)?,
            descriptor_record_field("inserted", &bool_descriptor(), true, None)?,
        ])),
        _ => Ok(None),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn map_types(expected_type: &Value) -> Option<(Value, Value)> {
    if let Some((root, args)) = generic_type_parts(expected_type) {
        if bare_type_name(&root) == "Map" && args.len() == 2 {
            return Some((args[0].clone(), args[1].clone()));
        }
    }
    let object = expected_type.as_object()?;
    if object.get("type").and_then(Value::as_str) == Some("object")
        && !object.contains_key("properties")
    {
        let value_type = match object.get("additionalProperties") {
            Some(Value::Object(_)) => object.get("additionalProperties")?.clone(),
            _ => return None,
        };
        return Some((string_descriptor(), value_type));
    }
    None
}

#[cfg(any(test, feature = "test-support"))]
pub fn record_fields(expected_type: &Value) -> Result<Option<Vec<RuntimeRecordFieldPlan>>> {
    let Some(object) = expected_type.as_object() else {
        return Ok(None);
    };
    let kind = object.get("kind").and_then(Value::as_str);
    if matches!(kind, Some("record" | "builtin")) {
        if let Some(fields) = object.get("fields").and_then(Value::as_object) {
            return Ok(Some(
                fields
                    .iter()
                    .map(|(name, ty)| {
                        descriptor_record_field(
                            name,
                            ty,
                            !is_nullable_descriptor(ty),
                            field_identity(ty),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?,
            ));
        }
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        item.as_str().map(str::to_string).ok_or_else(|| {
                            RuntimeError::InvalidArtifact(
                                "record required field name must be a string".to_string(),
                            )
                        })
                    })
                    .collect::<Result<std::collections::BTreeSet<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        return Ok(Some(
            properties
                .iter()
                .map(|(name, ty)| {
                    descriptor_record_field(name, ty, required.contains(name), field_identity(ty))
                })
                .collect::<Result<Vec<_>>>()?,
        ));
    }
    Ok(kind.is_some_and(|kind| kind == "record").then(Vec::new))
}

#[cfg(any(test, feature = "test-support"))]
fn descriptor_record_field(
    name: &str,
    descriptor: &Value,
    required: bool,
    identity: Option<String>,
) -> Result<RuntimeRecordFieldPlan> {
    Ok(RuntimeRecordFieldPlan {
        name: name.to_string(),
        ty: RuntimeTypePlan::from_descriptor(descriptor)?,
        required,
        identity,
    })
}

#[cfg(any(test, feature = "test-support"))]
fn runtime_type_identity_plan(descriptor: &Value) -> RuntimeTypeIdentityPlan {
    let Some(object) = descriptor.as_object() else {
        return RuntimeTypeIdentityPlan::default();
    };
    let identity = object.get("identity").and_then(Value::as_object);
    RuntimeTypeIdentityPlan {
        nominal: identity_string(identity, "nominal")
            .or_else(|| string_property(object, "nominalIdentity")),
        interface: identity_string(identity, "interface")
            .or_else(|| string_property(object, "interfaceIdentity")),
        union: identity_string(identity, "union")
            .or_else(|| string_property(object, "unionIdentity")),
        union_branch: identity_string(identity, "unionBranch")
            .or_else(|| string_property(object, "unionBranchIdentity")),
        method_projection: identity_string(identity, "methodProjection")
            .or_else(|| string_property(object, "methodProjectionIdentity")),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn field_identity(descriptor: &Value) -> Option<String> {
    descriptor
        .as_object()
        .and_then(|object| string_property(object, "fieldIdentity"))
}

#[cfg(any(test, feature = "test-support"))]
fn identity_string(identity: Option<&Map<String, Value>>, key: &str) -> Option<String> {
    identity.and_then(|object| string_property(object, key))
}

#[cfg(any(test, feature = "test-support"))]
fn string_property(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_nullable_descriptor(expected_type: &Value) -> bool {
    nullable_inner(expected_type).is_some()
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_json_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "Json")
        || schema_symbol_bare_name(expected_type).is_some_and(|name| name == "Json")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("json")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_json_object_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "JsonObject")
        || schema_symbol_bare_name(expected_type).is_some_and(|name| name == "JsonObject")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_bytes_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "bytes")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("bytes")
        || expected_type.as_object().is_some_and(|object| {
            object.get("type").and_then(Value::as_str) == Some("string")
                && object.get("contentEncoding").and_then(Value::as_str) == Some("base64")
        })
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_date_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "Date")
        || schema_symbol_bare_name(expected_type).is_some_and(|name| name == "Date")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_string_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "string")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("string")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_bool_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| {
        let bare = bare_type_name(&name);
        bare == "bool" || bare == "boolean"
    }) || expected_type
        .as_object()
        .and_then(|object| object.get("type"))
        .and_then(Value::as_str)
        .is_some_and(|ty| ty == "bool" || ty == "boolean")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_number_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "number")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("number")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_integer_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "integer")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("integer")
}

#[cfg(any(test, feature = "test-support"))]
pub fn is_null_descriptor(expected_type: &Value) -> bool {
    named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "null")
        || expected_type
            .as_object()
            .and_then(|object| object.get("type"))
            .and_then(Value::as_str)
            == Some("null")
        || named_type_name(expected_type).is_some_and(|name| bare_type_name(&name) == "void")
}

#[cfg(any(test, feature = "test-support"))]
pub fn named_type_name(expected_type: &Value) -> Option<String> {
    match expected_type {
        Value::String(value) => generic_root(value).map(str::to_string),
        Value::Object(object) => {
            if matches!(object.get("kind").and_then(Value::as_str), Some("builtin")) {
                return object
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            None
        }
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn generic_type_parts(expected_type: &Value) -> Option<(String, Vec<Value>)> {
    match expected_type {
        Value::String(value) => {
            let (root, args) = generic_text_parts(value)?;
            Some((
                root.to_string(),
                args.into_iter()
                    .map(type_text_descriptor)
                    .collect::<Vec<_>>(),
            ))
        }
        Value::Object(object) => {
            if !matches!(object.get("kind").and_then(Value::as_str), Some("builtin")) {
                return None;
            }
            let root = object.get("name")?.as_str()?.to_string();
            let args = object
                .get("args")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Some((root, args))
        }
        _ => None,
    }
}

pub fn generic_root(value: &str) -> Option<&str> {
    generic_text_parts(value)
        .map(|(root, _)| root)
        .or_else(|| Some(value.trim()).filter(|value| !value.is_empty()))
}

pub fn generic_text_parts(value: &str) -> Option<(&str, Vec<&str>)> {
    let value = value.trim();
    let start = value.find('<')?;
    if !value.ends_with('>') {
        return None;
    }
    let root = value[..start].trim();
    let inner = &value[start + 1..value.len() - 1];
    Some((root, split_top_level(inner, ',')))
}

pub fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ch if ch == delimiter && angle_depth == 0 && brace_depth == 0 && paren_depth == 0 => {
                let part = input[start..index].trim();
                if !part.is_empty() {
                    parts.push(part);
                }
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }

    let part = input[start..].trim();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_kind(value: &Value) -> Option<&str> {
    value
        .get("kind")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_kind_has_type_args(object: &Map<String, Value>) -> bool {
    matches!(
        object.get("kind").and_then(Value::as_str),
        Some("builtin") | Some("representation")
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_union_types(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("items")
        .or_else(|| value.get("variants"))
        .or_else(|| value.get("oneOf"))
        .and_then(Value::as_array)
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_name(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) => text_descriptor_name(text),
        Value::Object(object) => object.get("name").and_then(Value::as_str),
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_ref_name(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .or_else(|| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_ref_is_union(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|text| split_top_level(text, '|').len() > 1)
        || matches!(descriptor_kind(value), Some("union"))
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_ref_name_with_nullable(
    value: &Value,
    nullable: bool,
) -> std::result::Result<Option<(String, bool)>, RuntimeTypeNameError> {
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if let Some(inner) = trimmed.strip_suffix('?') {
            return Ok(Some((inner.trim().to_string(), true)));
        }
        return Ok(Some((trimmed.to_string(), nullable)));
    }

    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if object.get("nullable").and_then(Value::as_bool) == Some(true) {
        if let Some(inner) = object.get("inner") {
            return type_ref_name_with_nullable(inner, true);
        }
        return base_type_ref_name(object)
            .map(|name| strip_nullable_suffix(name, true))
            .map(Some)
            .ok_or(RuntimeTypeNameError::MissingType);
    }
    if matches!(object.get("kind").and_then(Value::as_str), Some("nullable")) {
        let Some(inner) = object.get("inner") else {
            return Err(RuntimeTypeNameError::NullableMissingInner);
        };
        return type_ref_name_with_nullable(inner, true);
    }
    base_type_ref_name(object)
        .map(|name| strip_nullable_suffix(name, nullable))
        .map(Some)
        .ok_or(RuntimeTypeNameError::MissingType)
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_text_descriptor(text: &str) -> Value {
    let text = text.trim();
    let union_parts = split_top_level(text, '|');
    if union_parts.len() > 1 {
        return json!({
            "kind": "union",
            "items": union_parts.into_iter().map(type_text_descriptor).collect::<Vec<_>>(),
        });
    }
    if let Some(inner) = strip_top_level_nullable_suffix(text) {
        return json!({
            "kind": "nullable",
            "inner": type_text_descriptor(inner),
        });
    }
    if let Some((root, args)) = generic_text_parts(text) {
        return json!({
            "kind": "builtin",
            "name": root,
            "args": args.into_iter().map(type_text_descriptor).collect::<Vec<_>>(),
        });
    }
    json!({
        "kind": "builtin",
        "name": text,
        "args": [],
    })
}

pub fn strip_top_level_nullable_suffix(value: &str) -> Option<&str> {
    let value = value.trim();
    let inner = value.strip_suffix('?')?;
    let mut angle_depth = 0usize;
    let mut brace_depth = 0usize;
    let mut paren_depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in inner.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            _ => {}
        }
    }

    (angle_depth == 0 && brace_depth == 0 && paren_depth == 0).then_some(inner.trim())
}

pub fn type_name_root(name: &str) -> &str {
    let name = name.trim();
    generic_text_parts(name)
        .map(|(root, _)| root)
        .unwrap_or_else(|| {
            name.find('<')
                .map(|index| name[..index].trim())
                .unwrap_or(name)
        })
}

pub fn short_type_name(name: &str) -> &str {
    bare_type_name(name)
}

pub fn bare_type_name(name: &str) -> &str {
    let root = type_name_root(name);
    let name = root
        .rsplit_once("::")
        .map(|(_, short)| short)
        .unwrap_or(root);
    name.rsplit(['.', ':']).next().unwrap_or(name).trim()
}

pub fn type_name_matches(pattern: &str, actual: &str) -> bool {
    pattern.trim() == actual.trim()
}

#[cfg(any(test, feature = "test-support"))]
pub fn type_parameter_name(value: &Value) -> Option<&str> {
    match value {
        Value::String(text) if is_type_parameter(text.trim()) => Some(text.trim()),
        Value::Object(object) if object.get("kind").and_then(Value::as_str) == Some("builtin") => {
            let name = object.get("name")?.as_str()?;
            let has_args = object
                .get("args")
                .and_then(Value::as_array)
                .is_some_and(|args| !args.is_empty());
            let has_runtime_descriptor =
                object.contains_key("fields") || object.contains_key("representation");
            (!has_args && !has_runtime_descriptor && is_type_parameter(name)).then_some(name)
        }
        Value::Object(object)
            if object.get("kind").and_then(Value::as_str) == Some("typeParam") =>
        {
            object.get("name")?.as_str()
        }
        _ => None,
    }
}

pub fn is_type_parameter(name: &str) -> bool {
    if is_builtin_concrete_type_name(name) {
        return false;
    }
    let mut chars = name.chars();
    matches!(chars.next(), Some(ch) if ch.is_ascii_uppercase())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub fn unresolved_type_descriptor(expected_type: &RuntimeTypePlan) -> RuntimeError {
    if let Some(name) = expected_type.named_type_name() {
        if !is_builtin_named_type(name) {
            return RuntimeError::InvalidArtifact(format!(
                "unresolved builtin {name}; runtime descriptor must include record fields, representation payload, or alias target"
            ));
        }
    }
    if expected_type.label() == "any" {
        return RuntimeError::InvalidArtifact(
            "runtime descriptor type any is not allowed; use explicit Json for arbitrary JSON"
                .to_string(),
        );
    }
    RuntimeError::InvalidArtifact(format!(
        "unsupported runtime type descriptor {}",
        expected_type.label()
    ))
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_label(expected_type: &Value) -> String {
    match expected_type {
        Value::String(value) => value.clone(),
        Value::Object(object) => object
            .get("kind")
            .and_then(Value::as_str)
            .or_else(|| object.get("type").and_then(Value::as_str))
            .unwrap_or("object")
            .to_string(),
        _ => expected_type.to_string(),
    }
}

pub fn is_builtin_named_type(name: &str) -> bool {
    matches!(
        bare_type_name(name),
        "Array"
            | "Map"
            | "Json"
            | "JsonObject"
            | "Date"
            | "string"
            | "bytes"
            | "number"
            | "integer"
            | "bool"
            | "boolean"
            | "null"
            | "void"
            | "DbInsertManyResult"
            | "DbUpdateManyResult"
            | "DbDeleteManyResult"
            | "DbUpsertResult"
    )
}

#[cfg(any(test, feature = "test-support"))]
pub fn json_descriptor() -> Value {
    json!({ "kind": "builtin", "name": "Json", "args": [] })
}

#[cfg(any(test, feature = "test-support"))]
pub fn string_descriptor() -> Value {
    json!({ "kind": "builtin", "name": "string", "args": [] })
}

#[cfg(any(test, feature = "test-support"))]
fn number_descriptor() -> Value {
    json!({ "kind": "builtin", "name": "number", "args": [] })
}

#[cfg(any(test, feature = "test-support"))]
fn bool_descriptor() -> Value {
    json!({ "kind": "builtin", "name": "bool", "args": [] })
}

#[cfg(any(test, feature = "test-support"))]
fn builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "builtin".to_string(),
        named_type_name: Some(name.to_string()),
        identity: RuntimeTypeIdentityPlan::default(),
        node,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn leaf_builtin_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    builtin_plan(name, node)
}

#[cfg(any(test, feature = "test-support"))]
fn std_field(name: &str, ty: RuntimeTypePlan) -> RuntimeRecordFieldPlan {
    let required = !matches!(ty.node, RuntimeTypeNode::Nullable(_));
    RuntimeRecordFieldPlan {
        name: name.to_string(),
        ty,
        required,
        identity: None,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn leaf_string_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("string", RuntimeTypeNode::String)
}

#[cfg(any(test, feature = "test-support"))]
fn leaf_integer_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("integer", RuntimeTypeNode::Integer)
}

#[cfg(any(test, feature = "test-support"))]
fn leaf_bytes_plan() -> RuntimeTypePlan {
    leaf_builtin_plan("bytes", RuntimeTypeNode::Bytes)
}

#[cfg(any(test, feature = "test-support"))]
fn std_record_plan(name: &str, fields: Vec<RuntimeRecordFieldPlan>) -> RuntimeTypePlan {
    builtin_plan(
        name,
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind: Some(name.to_string()),
        },
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_union_plan(name: &str, items: Vec<RuntimeTypePlan>) -> RuntimeTypePlan {
    builtin_plan(name, RuntimeTypeNode::Union(items))
}

#[cfg(any(test, feature = "test-support"))]
fn std_nullable_plan(inner: RuntimeTypePlan) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "nullable".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::Nullable(Box::new(inner)),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn std_array_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Array", RuntimeTypeNode::Array(Box::new(item)))
}

#[cfg(any(test, feature = "test-support"))]
fn std_stream_plan(item: RuntimeTypePlan) -> RuntimeTypePlan {
    builtin_plan("Stream", RuntimeTypeNode::Stream(Box::new(item)))
}

#[cfg(any(test, feature = "test-support"))]
fn std_literal_string_plan(value: &str) -> RuntimeTypePlan {
    RuntimeTypePlan {
        label: "literal".to_string(),
        named_type_name: None,
        identity: RuntimeTypeIdentityPlan::default(),
        node: RuntimeTypeNode::LiteralString(value.to_string()),
    }
}

#[cfg(any(test, feature = "test-support"))]
fn std_http_header_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpHeader",
        vec![
            std_field("name", leaf_string_plan()),
            std_field("value", leaf_string_plan()),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_http_client_request_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientRequest",
        vec![
            std_field("method", leaf_string_plan()),
            std_field("url", leaf_string_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_nullable_plan(leaf_bytes_plan())),
            std_field("timeoutMs", std_nullable_plan(leaf_integer_plan())),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_http_client_response_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientResponse",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", leaf_bytes_plan()),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_http_client_stream_handle_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.http.HttpClientStreamHandle",
        vec![
            std_field("status", leaf_integer_plan()),
            std_field("headers", std_array_plan(std_http_header_plan())),
            std_field("body", std_stream_plan(leaf_bytes_plan())),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_connection_plan(context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketConnection",
        vec![
            std_field("id", leaf_string_plan()),
            std_field("businessIdentity", std_nullable_plan(leaf_string_plan())),
            std_field("context", context),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_text_message_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.TextConnectionMessage",
        vec![
            std_field("tag", std_literal_string_plan("text")),
            std_field("text", leaf_string_plan()),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_binary_message_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.BinaryConnectionMessage",
        vec![
            std_field("tag", std_literal_string_plan("binary")),
            std_field("base64", leaf_string_plan()),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_connection_message_plan() -> RuntimeTypePlan {
    builtin_plan(
        "std.websocket.ConnectionMessage",
        RuntimeTypeNode::Representation {
            type_name: "std.websocket.ConnectionMessage".to_string(),
            payload: Box::new(std_union_plan(
                "std.websocket.ConnectionMessage",
                vec![
                    std_websocket_text_message_plan(),
                    std_websocket_binary_message_plan(),
                ],
            )),
        },
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_connect_result_plan(name: &str, context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_union_plan(
        name,
        vec![
            std_record_plan(
                "std.websocket.WebSocketConnectAccept",
                vec![
                    std_field("tag", std_literal_string_plan("accept")),
                    std_field("context", context),
                    std_field("businessIdentity", std_nullable_plan(leaf_string_plan())),
                    std_field(
                        "connectionPolicy",
                        std_nullable_plan(std_websocket_connection_policy_plan()),
                    ),
                ],
            ),
            std_record_plan(
                "std.websocket.WebSocketConnectReject",
                vec![
                    std_field("tag", std_literal_string_plan("reject")),
                    std_field("code", leaf_integer_plan()),
                    std_field("reason", leaf_string_plan()),
                ],
            ),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_connection_policy_plan() -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketConnectionPolicy",
        vec![
            std_field("maxConnections", leaf_integer_plan()),
            std_field(
                "overflow",
                std_union_plan(
                    "std.websocket.WebSocketConnectionPolicy.overflow",
                    vec![
                        std_literal_string_plan("close-oldest"),
                        std_literal_string_plan("reject-new"),
                    ],
                ),
            ),
            std_field("closeCode", std_nullable_plan(leaf_integer_plan())),
            std_field("closeReason", std_nullable_plan(leaf_string_plan())),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_websocket_receive_event_plan(context: RuntimeTypePlan) -> RuntimeTypePlan {
    std_record_plan(
        "std.websocket.WebSocketReceiveEvent",
        vec![
            std_field("connection", std_websocket_connection_plan(context)),
            std_field("message", std_websocket_connection_message_plan()),
        ],
    )
}

#[cfg(any(test, feature = "test-support"))]
fn std_runtime_builtin_node_from_descriptor(descriptor: &Value) -> Option<Result<RuntimeTypeNode>> {
    let (root, args) = generic_type_parts(descriptor)?;
    let root_name = type_name_root(&root);
    let bare = bare_type_name(root_name);
    let node = match bare {
        "HttpClientRequest" if args.is_empty() && root_name == "std.http.HttpClientRequest" => {
            std_http_client_request_plan().node
        }
        "HttpClientResponse" if args.is_empty() && root_name == "std.http.HttpClientResponse" => {
            std_http_client_response_plan().node
        }
        "HttpClientStreamHandle"
            if args.is_empty() && root_name == "std.http.HttpClientStreamHandle" =>
        {
            std_http_client_stream_handle_plan().node
        }
        "ConnectionMessage"
            if args.is_empty() && root_name == "std.websocket.ConnectionMessage" =>
        {
            std_websocket_connection_message_plan().node
        }
        "WebSocketConnection"
            if args.len() == 1
                && matches!(
                    root_name,
                    "WebSocketConnection" | "std.websocket.WebSocketConnection"
                ) =>
        {
            let context = match RuntimeTypePlan::from_descriptor(&args[0]) {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_connection_plan(context).node
        }
        "WebSocketConnectResult"
            if args.len() == 1
                && matches!(
                    root_name,
                    "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
                ) =>
        {
            let context = match RuntimeTypePlan::from_descriptor(&args[0]) {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_connect_result_plan(root_name, context).node
        }
        "WebSocketReceiveEvent"
            if args.len() == 1
                && matches!(
                    root_name,
                    "WebSocketReceiveEvent" | "std.websocket.WebSocketReceiveEvent"
                ) =>
        {
            let context = match RuntimeTypePlan::from_descriptor(&args[0]) {
                Ok(plan) => plan,
                Err(error) => return Some(Err(error)),
            };
            std_websocket_receive_event_plan(context).node
        }
        _ => return None,
    };
    Some(Ok(node))
}

#[cfg(any(test, feature = "test-support"))]
pub fn descriptor_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.trim().to_string());
    }
    let object = value.as_object()?;
    match descriptor_kind(value) {
        Some("builtin") | Some("representation") | None => {
            let name = descriptor_name(value)?;
            let args = match object.get("args").and_then(Value::as_array) {
                Some(items) => items
                    .iter()
                    .map(descriptor_text)
                    .collect::<Option<Vec<_>>>()?,
                None => Vec::new(),
            };
            if args.is_empty() {
                Some(name.to_string())
            } else {
                Some(format!("{name}<{}>", args.join(", ")))
            }
        }
        Some("nullable") => Some(format!("{}?", descriptor_text(object.get("inner")?)?)),
        Some("union") => descriptor_union_types(value)?
            .iter()
            .map(descriptor_text)
            .collect::<Option<Vec<_>>>()
            .map(|types| types.join(" | ")),
        Some("literal") => {
            serde_json::to_string(&literal_ir_json_value(object.get("value")?)?).ok()
        }
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
fn text_descriptor_name(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    generic_text_parts(text)
        .map(|(root, _)| root)
        .or(Some(text))
}

#[cfg(any(test, feature = "test-support"))]
fn schema_symbol_bare_name(expected_type: &Value) -> Option<&str> {
    schema_symbol_name(expected_type)?.rsplit('.').next()
}

#[cfg(any(test, feature = "test-support"))]
pub fn schema_symbol_name(expected_type: &Value) -> Option<&str> {
    expected_type.as_object()?.get("xSkiffSymbol")?.as_str()
}

#[cfg(any(test, feature = "test-support"))]
fn boundary_record_kind(expected_type: &Value) -> Option<String> {
    named_type_name(expected_type).or_else(|| schema_symbol_name(expected_type).map(str::to_string))
}

#[cfg(any(test, feature = "test-support"))]
fn base_type_ref_name(object: &Map<String, Value>) -> Option<String> {
    object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(any(test, feature = "test-support"))]
fn strip_nullable_suffix(name: String, nullable: bool) -> (String, bool) {
    if let Some(inner) = name.trim().strip_suffix('?') {
        return (inner.trim().to_string(), true);
    }
    (name, nullable)
}

fn is_builtin_concrete_type_name(name: &str) -> bool {
    matches!(
        name.trim(),
        "Json"
            | "JsonObject"
            | "Date"
            | "Stream"
            | "Config"
            | "DbInsertManyResult"
            | "DbUpdateManyResult"
            | "DbDeleteManyResult"
            | "DbUpsertResult"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_descriptor_identity_fields_parse_when_present() {
        let descriptor = json!({
            "kind": "builtin",
            "name": "pkg.UserView",
            "args": [],
            "identity": {
                "nominal": "type:pkg.UserView@1",
                "interface": "iface:pkg.Viewable@1",
                "union": "union:pkg.UserView@1",
                "methodProjection": "methodProjection:pkg.UserView.render@1"
            }
        });

        let plan =
            RuntimeTypePlan::from_descriptor(&descriptor).expect("descriptor plan should build");

        assert_eq!(plan.nominal_identity(), Some("type:pkg.UserView@1"));
        assert_eq!(plan.interface_identity(), Some("iface:pkg.Viewable@1"));
        assert_eq!(plan.union_identity(), Some("union:pkg.UserView@1"));
        assert_eq!(
            plan.method_projection_identity(),
            Some("methodProjection:pkg.UserView.render@1")
        );
    }

    #[test]
    fn type_descriptor_record_field_identity_can_differ_from_display_name() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "displayName": {
                    "kind": "builtin",
                    "name": "string",
                    "args": [],
                    "fieldIdentity": "field:pkg.User.legal_name@1"
                }
            }
        });

        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("record plan should build");
        let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
            panic!("expected record plan");
        };

        assert_eq!(fields[0].name, "displayName");
        assert_eq!(fields[0].identity(), Some("field:pkg.User.legal_name@1"));
    }

    #[test]
    fn type_descriptor_union_branch_identity_can_differ_from_branch_label() {
        let descriptor = json!({
            "kind": "union",
            "identity": {
                "union": "union:pkg.Result@1"
            },
            "items": [
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "ok" },
                    "identity": {
                        "unionBranch": "branch:pkg.Result.success@1"
                    }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "err" },
                    "identity": {
                        "unionBranch": "branch:pkg.Result.failure@1"
                    }
                }
            ]
        });

        let plan = RuntimeTypePlan::from_descriptor(&descriptor).expect("union plan should build");
        let RuntimeTypeNode::Union(branches) = plan.node() else {
            panic!("expected union plan");
        };

        assert_eq!(plan.union_identity(), Some("union:pkg.Result@1"));
        assert_eq!(
            branches[0].union_branch_identity(),
            Some("branch:pkg.Result.success@1")
        );
        assert!(matches!(
            branches[0].node(),
            RuntimeTypeNode::LiteralString(value) if value == "ok"
        ));
    }

    #[test]
    fn type_descriptor_without_identity_keeps_identity_absent() {
        let descriptor = json!({
            "kind": "record",
            "fields": {
                "name": { "kind": "builtin", "name": "string", "args": [] }
            }
        });

        let plan =
            RuntimeTypePlan::from_descriptor(&descriptor).expect("old descriptor should build");
        let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
            panic!("expected record plan");
        };

        assert!(!plan.has_identity());
        assert_eq!(fields[0].identity(), None);
    }
}

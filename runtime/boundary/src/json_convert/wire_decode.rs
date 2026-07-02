use serde_json::{Map, Value};

use crate::{
    date_value,
    error::{Result, RuntimeError},
    request_heap::RequestHeap,
    runtime_value::{RuntimeMap, RuntimeObjectFields, RuntimeValue, RuntimeValueKey},
    stream::is_stream_value,
    type_descriptor::{
        unresolved_type_descriptor, RuntimeRecordFieldPlan as RecordField, RuntimeTypeNode,
        RuntimeTypePlan,
    },
    value::{decode_base64, BYTES_BASE64_KEY},
};

use super::{
    context::{StreamHandleScope, STREAM_HANDLE_SCOPE_ERROR},
    keys::runtime_key_from_wire_key_plan,
    numbers::max_safe_json_integer,
    record::{RecordProjectionValue, RuntimeRecordShape},
    runtime_json::reject_reserved_legacy_json_metadata_key,
};

pub(super) fn from_wire_inner_with_stream_scope(
    json: &Value,
    expected_type: &RuntimeTypePlan,
    heap: &mut RequestHeap,
    stream_scope: StreamHandleScope,
) -> Result<RuntimeValue> {
    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => {
            from_wire_inner_with_stream_scope(json, target, heap, stream_scope)
        }
        RuntimeTypeNode::Nullable(inner) => {
            if json.is_null() {
                Ok(RuntimeValue::Null)
            } else {
                from_wire_inner_with_stream_scope(json, inner, heap, stream_scope)
            }
        }
        RuntimeTypeNode::Union(types) => from_union_wire(json, types, heap, stream_scope),
        RuntimeTypeNode::LiteralString(literal) => match json.as_str() {
            Some(value) if value == literal => Ok(RuntimeValue::String(value.to_string())),
            _ => Err(RuntimeError::Decode(format!(
                "expected literal string {literal:?}"
            ))),
        },
        RuntimeTypeNode::Representation { payload, .. } => {
            from_wire_inner_with_stream_scope(json, payload, heap, StreamHandleScope::nested())
        }
        RuntimeTypeNode::Json => from_json_value(json, heap),
        RuntimeTypeNode::JsonObject => from_json_object(json, heap),
        RuntimeTypeNode::Bytes => decode_bytes_runtime_value(json, heap),
        RuntimeTypeNode::Date => json
            .as_str()
            .ok_or_else(|| RuntimeError::Decode("expected RFC3339 Date string".to_string()))
            .and_then(|value| date_value::parse_rfc3339_millis(value, "from_wire<Date>"))
            .map(RuntimeValue::Date),
        RuntimeTypeNode::String => json
            .as_str()
            .map(|value| RuntimeValue::String(value.to_string()))
            .ok_or_else(|| RuntimeError::Decode("expected string".to_string())),
        RuntimeTypeNode::Bool => json
            .as_bool()
            .map(RuntimeValue::Bool)
            .ok_or_else(|| RuntimeError::Decode("expected bool".to_string())),
        RuntimeTypeNode::Integer => integer_from_wire(json),
        RuntimeTypeNode::Number => number_from_wire(json),
        RuntimeTypeNode::Null => {
            if json.is_null() {
                Ok(RuntimeValue::Null)
            } else {
                Err(RuntimeError::Decode("expected null".to_string()))
            }
        }
        RuntimeTypeNode::Stream(_) => {
            if !stream_scope.allows_current_node() {
                return Err(RuntimeError::Decode(STREAM_HANDLE_SCOPE_ERROR.to_string()));
            }
            if is_stream_value(json) {
                from_json_value(json, heap)
            } else {
                Err(RuntimeError::Decode("expected Stream handle".to_string()))
            }
        }
        RuntimeTypeNode::Array(item_type) => {
            let items = json
                .as_array()
                .ok_or_else(|| RuntimeError::Decode("expected array".to_string()))?
                .iter()
                .map(|item| {
                    from_wire_inner_with_stream_scope(
                        item,
                        item_type,
                        heap,
                        StreamHandleScope::nested(),
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(RuntimeValue::Heap(heap.alloc_array(items)?))
        }
        RuntimeTypeNode::Map {
            key: key_type,
            value: value_type,
        } => {
            let object = json
                .as_object()
                .ok_or_else(|| RuntimeError::Decode("expected map object".to_string()))?;
            let mut map = RuntimeMap::new();
            for (key, value) in object {
                reject_reserved_legacy_json_metadata_key(key)?;
                map.insert(
                    runtime_key_from_wire_key_plan(key, key_type)?,
                    from_wire_inner_with_stream_scope(
                        value,
                        value_type,
                        heap,
                        StreamHandleScope::nested(),
                    )?,
                );
            }
            Ok(RuntimeValue::Heap(heap.alloc_map(map)?))
        }
        RuntimeTypeNode::Record { fields, .. } => {
            from_record_wire(json, expected_type, fields, heap, stream_scope)
        }
        RuntimeTypeNode::Unknown => Err(unresolved_type_descriptor(expected_type)),
    }
}

fn from_union_wire(
    json: &Value,
    types: &[RuntimeTypePlan],
    heap: &mut RequestHeap,
    stream_scope: StreamHandleScope,
) -> Result<RuntimeValue> {
    if json.is_null() && types.iter().any(is_null_plan) {
        return Ok(RuntimeValue::Null);
    }
    let mut errors = Vec::new();
    for ty in types {
        if is_null_plan(ty) {
            continue;
        }
        let checkpoint = heap.checkpoint();
        match from_wire_inner_with_stream_scope(json, ty, heap, stream_scope) {
            Ok(value) => return Ok(value),
            Err(error) => {
                heap.rollback_to_checkpoint(checkpoint);
                errors.push(error.to_string());
            }
        }
    }
    Err(RuntimeError::Decode(format!(
        "union value did not match any branch: {}",
        errors.join("; ")
    )))
}

fn from_json_value(json: &Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    match json {
        Value::Null => Ok(RuntimeValue::Null),
        Value::Bool(value) => Ok(RuntimeValue::Bool(*value)),
        Value::Number(_) => number_from_wire(json),
        Value::String(value) => Ok(RuntimeValue::String(value.clone())),
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| from_json_value(item, heap))
                .collect::<Result<Vec<_>>>()?;
            Ok(RuntimeValue::Heap(heap.alloc_array(items)?))
        }
        Value::Object(object) => {
            reject_reserved_legacy_json_object_keys(object)?;
            if object.contains_key(BYTES_BASE64_KEY) {
                return decode_bytes_runtime_value(json, heap);
            }
            let mut map = RuntimeMap::new();
            for (key, value) in object {
                map.insert(RuntimeValueKey::string(key), from_json_value(value, heap)?);
            }
            Ok(RuntimeValue::Heap(heap.alloc_map(map)?))
        }
    }
}

fn from_json_object(json: &Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let object = json
        .as_object()
        .ok_or_else(|| RuntimeError::Decode("expected JsonObject object".to_string()))?;
    reject_reserved_legacy_json_object_keys(object)?;
    let mut map = RuntimeMap::new();
    for (key, value) in object {
        map.insert(RuntimeValueKey::string(key), from_json_value(value, heap)?);
    }
    Ok(RuntimeValue::Heap(heap.alloc_map(map)?))
}

fn from_record_wire(
    json: &Value,
    expected_type: &RuntimeTypePlan,
    fields: &[RecordField],
    heap: &mut RequestHeap,
    stream_scope: StreamHandleScope,
) -> Result<RuntimeValue> {
    let object = json
        .as_object()
        .ok_or_else(|| RuntimeError::Decode("expected record object".to_string()))?;
    reject_reserved_legacy_json_object_keys(object)?;
    let shape = RuntimeRecordShape::for_plan(fields, expected_type.boundary_record_kind());
    let projection = shape.project_json_object(object)?;
    let mut runtime_fields = RuntimeObjectFields::new();
    for projected in projection.into_fields() {
        let value = match projected.value {
            RecordProjectionValue::Present(field_json) => from_wire_inner_with_stream_scope(
                field_json,
                &projected.field.ty,
                heap,
                stream_scope.record_field(expected_type, &projected.field.name),
            )?,
            RecordProjectionValue::MissingOptionalNull => RuntimeValue::Null,
        };
        runtime_fields.insert(projected.field.name.clone(), value);
    }
    Ok(RuntimeValue::Heap(
        heap.alloc_object(shape.runtime_object(runtime_fields))?,
    ))
}

fn reject_reserved_legacy_json_object_keys(object: &Map<String, Value>) -> Result<()> {
    for key in object.keys() {
        reject_reserved_legacy_json_metadata_key(key)?;
    }
    Ok(())
}

fn is_null_plan(expected_type: &RuntimeTypePlan) -> bool {
    match expected_type.node() {
        RuntimeTypeNode::Alias(target) => is_null_plan(target),
        RuntimeTypeNode::Null => true,
        _ => false,
    }
}

pub(super) fn decode_bytes_runtime_value(
    json: &Value,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let encoded = match json {
        Value::String(value) => value.as_str(),
        Value::Object(object) => object
            .get(BYTES_BASE64_KEY)
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RuntimeError::Decode("bytes object requires base64 metadata".to_string())
            })?,
        _ => {
            return Err(RuntimeError::Decode(
                "bytes value must be base64 string or metadata object".to_string(),
            ))
        }
    };
    let bytes = decode_base64(encoded).map_err(RuntimeError::Decode)?;
    Ok(RuntimeValue::Heap(heap.alloc_bytes(bytes)?))
}

fn number_from_wire(json: &Value) -> Result<RuntimeValue> {
    json.as_f64()
        .filter(|value| value.is_finite())
        .map(RuntimeValue::Number)
        .ok_or_else(|| RuntimeError::Decode("expected finite number".to_string()))
}

fn integer_from_wire(json: &Value) -> Result<RuntimeValue> {
    let value = json
        .as_i64()
        .map(|value| value as f64)
        .or_else(|| {
            json.as_u64()
                .and_then(|value| (value <= max_safe_json_integer() as u64).then_some(value as f64))
        })
        .or_else(|| {
            let value = json.as_f64()?;
            (value.is_finite() && value.fract() == 0.0 && value.abs() <= max_safe_json_integer())
                .then_some(value)
        })
        .ok_or_else(|| RuntimeError::Decode("expected safe integer".to_string()))?;
    Ok(RuntimeValue::Number(value))
}

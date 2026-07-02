use std::{borrow::Cow, collections::BTreeMap};

use serde_json::Value;
use skiff_runtime_boundary::{
    contract::RuntimeBoundaryContract,
    date_value,
    json::{decode_untyped_wire_json, encode_untyped_wire_json},
    plan::BoundaryUse,
};
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value, RequestHeap},
    runtime_value::{
        runtime_map_has as model_runtime_map_has, runtime_values_equal, HeapNode, RuntimeBytes,
        RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
    type_plan::RuntimeTypePlan,
};

use crate::error::{Result, RuntimeError};

use super::runtime_value_view::RuntimeValueView;

pub fn runtime_from_wire(value: &Value, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    Ok(decode_untyped_wire_json(value, heap)?)
}

pub fn runtime_from_wire_required_plan(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    runtime_from_wire_required_plan_with_use(
        value,
        expected_type,
        boundary,
        BoundaryUse::TypedJson,
        heap,
    )
}

pub fn runtime_from_wire_required_plan_with_use(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    use_case: BoundaryUse,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let expected_type = required_type(expected_type, boundary)?;
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, use_case, boundary)
        .from_wire_json(value, heap)?)
}

pub fn runtime_from_wire_internal_handle_required_plan(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let expected_type = required_type(expected_type, boundary)?;
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::NativeReturn, boundary)
        .from_wire_json_internal_handle(value, heap)?)
}

pub fn runtime_to_wire(value: &RuntimeValue, heap: &RequestHeap) -> Result<Value> {
    Ok(encode_untyped_wire_json(value, heap)?)
}

pub fn runtime_to_wire_required_plan(
    value: &RuntimeValue,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let expected_type = required_type(expected_type, boundary)?;
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::TypedJson, boundary)
        .to_wire_json(value, heap)?)
}

pub fn runtime_coerce_required_plan(
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::NativeArg, boundary)
        .coerce_runtime_value(value, heap)?)
}

pub fn runtime_response_value_required_plan(
    value: RuntimeValue,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<Value> {
    runtime_to_wire_required_plan(&value, expected_type, boundary, heap)
}

pub fn runtime_encode_json_text_required_plan(
    value: &RuntimeValue,
    expected_type: &RuntimeTypePlan,
    boundary: &str,
    use_case: BoundaryUse,
    heap: &mut RequestHeap,
) -> Result<String> {
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, use_case, boundary)
        .encode_json_text_value(value, heap)?)
}

pub fn runtime_encode_json_text_untyped(
    value: &RuntimeValue,
    boundary: impl Into<Cow<'static, str>>,
    use_case: BoundaryUse,
    heap: &mut RequestHeap,
) -> Result<String> {
    Ok(RuntimeBoundaryContract::default()
        .encode_json_text(value, None, use_case, boundary, heap)?)
}

pub fn runtime_decode_json_text_required_plan(
    input: &str,
    expected_type: &RuntimeTypePlan,
    boundary: &str,
    use_case: BoundaryUse,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, use_case, boundary)
        .decode_json_text(input, heap)?)
}

fn required_type<'a>(
    expected_type: Option<&'a RuntimeTypePlan>,
    boundary: &str,
) -> Result<&'a RuntimeTypePlan> {
    expected_type.ok_or_else(|| {
        RuntimeError::InvalidArtifact(format!(
            "{boundary} boundary is missing expected type descriptor"
        ))
    })
}

pub fn runtime_empty_object(heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let handle = heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))?;
    Ok(RuntimeValue::Heap(handle))
}

pub fn runtime_empty_array(heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let handle = heap.alloc_array(Vec::new())?;
    Ok(RuntimeValue::Heap(handle))
}

pub fn runtime_empty_map(heap: &mut RequestHeap) -> Result<RuntimeValue> {
    let handle = heap.alloc_map(RuntimeMap::new())?;
    Ok(RuntimeValue::Heap(handle))
}

pub fn runtime_debug_value_for_error(value: &RuntimeValue, heap: &RequestHeap) -> String {
    runtime_to_wire(value, heap)
        .map(|value| format!("{value:?}"))
        .unwrap_or_else(|_| format!("{value:?}"))
}

pub fn runtime_truthy(value: &RuntimeValue, heap: &RequestHeap) -> Result<bool> {
    Ok(match value {
        RuntimeValue::Null => false,
        RuntimeValue::Bool(value) => *value,
        RuntimeValue::Number(value) => *value != 0.0,
        RuntimeValue::Date(_) => true,
        RuntimeValue::String(value) => !value.is_empty(),
        RuntimeValue::ActorRef(_) => true,
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Bytes(bytes) => !bytes.is_empty(),
            HeapNode::Array(items) => !items.is_empty(),
            HeapNode::Object(object) => !object.fields().is_empty(),
            HeapNode::Map(map) => !map.is_empty(),
            HeapNode::Interface(value) => {
                return Err(RuntimeError::Decode(format!(
                    "{} cannot be coerced to bool",
                    value.diagnostic_label()
                )));
            }
        },
    })
}

pub fn runtime_numeric(value: &RuntimeValue) -> Result<f64> {
    match value {
        RuntimeValue::Number(value) => Ok(*value),
        _ => Err(RuntimeError::Decode("expected number".to_string())),
    }
}

pub fn runtime_number_value(value: f64) -> RuntimeValue {
    if value.is_finite() {
        RuntimeValue::Number(value)
    } else {
        RuntimeValue::Null
    }
}

pub fn runtime_stringify_key(value: &RuntimeValue, heap: &RequestHeap) -> Result<String> {
    Ok(match value {
        RuntimeValue::String(value) => value.clone(),
        RuntimeValue::Number(value) => stringify_number(*value),
        RuntimeValue::Date(ms) => date_value::format_epoch_millis(*ms, "Date stringify")?,
        RuntimeValue::Bool(value) => value.to_string(),
        RuntimeValue::Null => "null".to_string(),
        RuntimeValue::ActorRef(actor_ref) => format!(
            "{}#{}",
            actor_ref.actor_type_identity(),
            actor_ref.actor_id_hash()
        ),
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Bytes(bytes) => hex::encode(bytes.as_slice()),
            HeapNode::Interface(value) => {
                return Err(RuntimeError::Decode(format!(
                    "{} cannot be stringified",
                    value.diagnostic_label()
                )));
            }
            _ => serde_json::to_string(&runtime_to_wire(value, heap)?)?,
        },
    })
}

pub fn runtime_eval_binary(
    op: &str,
    left: RuntimeValue,
    right: RuntimeValue,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    match op {
        "==" => Ok(RuntimeValue::Bool(runtime_values_equal(
            heap, &left, &right,
        )?)),
        "!=" => Ok(RuntimeValue::Bool(!runtime_values_equal(
            heap, &left, &right,
        )?)),
        "<" => Ok(RuntimeValue::Bool(
            runtime_numeric(&left)? < runtime_numeric(&right)?,
        )),
        "<=" => Ok(RuntimeValue::Bool(
            runtime_numeric(&left)? <= runtime_numeric(&right)?,
        )),
        ">" => Ok(RuntimeValue::Bool(
            runtime_numeric(&left)? > runtime_numeric(&right)?,
        )),
        ">=" => Ok(RuntimeValue::Bool(
            runtime_numeric(&left)? >= runtime_numeric(&right)?,
        )),
        "+" => {
            if matches!(left, RuntimeValue::String(_)) || matches!(right, RuntimeValue::String(_)) {
                Ok(RuntimeValue::String(format!(
                    "{}{}",
                    runtime_stringify_key(&left, heap)?,
                    runtime_stringify_key(&right, heap)?
                )))
            } else {
                Ok(runtime_number_value(
                    runtime_numeric(&left)? + runtime_numeric(&right)?,
                ))
            }
        }
        "-" => Ok(runtime_number_value(
            runtime_numeric(&left)? - runtime_numeric(&right)?,
        )),
        "*" => Ok(runtime_number_value(
            runtime_numeric(&left)? * runtime_numeric(&right)?,
        )),
        "/" => Ok(runtime_number_value(
            runtime_numeric(&left)? / runtime_numeric(&right)?,
        )),
        other => Err(RuntimeError::Unsupported(format!(
            "unsupported binary operator {other}"
        ))),
    }
}

pub fn runtime_member_access(
    value: &RuntimeValue,
    field: &str,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    match value {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Object(object) => Ok(object
                .fields()
                .get(field)
                .cloned()
                .unwrap_or(RuntimeValue::Null)),
            HeapNode::Map(map) => Ok(map
                .get(&RuntimeValueKey::string(field))
                .cloned()
                .unwrap_or(RuntimeValue::Null)),
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "{} does not support ordinary member access",
                value.diagnostic_label()
            ))),
            _ => Ok(RuntimeValue::Null),
        },
        _ => Ok(RuntimeValue::Null),
    }
}

#[allow(dead_code)]
pub fn runtime_bytes_payload<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
) -> Result<Option<&'a RuntimeBytes>> {
    RuntimeValueView::bytes_payload_from(value, heap)
}

pub fn runtime_array_items(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<Option<Vec<RuntimeValue>>> {
    RuntimeValueView::new(value, heap).array_items()
}

pub fn runtime_map_get(
    receiver: &RuntimeValue,
    key: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    RuntimeValueView::new(receiver, heap).map_get(key)
}

pub fn runtime_map_has(
    receiver: &RuntimeValue,
    key: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<bool> {
    Ok(model_runtime_map_has(heap, receiver, key)?)
}

pub fn runtime_deep_clone(value: &RuntimeValue, heap: &mut RequestHeap) -> Result<RuntimeValue> {
    Ok(deep_clone_runtime_value(heap, value)?)
}

pub fn runtime_object_from_fields(
    fields: BTreeMap<String, RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let object = RuntimeObject::unshaped(fields);
    Ok(RuntimeValue::Heap(heap.alloc_object(object)?))
}

pub fn runtime_map_from_entries(
    entries: RuntimeMap,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeValue::Heap(heap.alloc_map(entries)?))
}

pub fn runtime_array_from_items(
    items: Vec<RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeValue::Heap(heap.alloc_array(items)?))
}

fn stringify_number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

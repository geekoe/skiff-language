use std::{borrow::Cow, collections::BTreeMap};

use serde_json::Value;
use skiff_runtime_boundary::{
    contract::RuntimeBoundaryContract,
    date_value,
    json::{decode_untyped_wire_json, encode_untyped_wire_json},
    plan::BoundaryUse,
};
use skiff_runtime_model::{
    request_heap::{
        deep_clone_runtime_value, deep_clone_runtime_value_carrier,
        deep_clone_runtime_value_carrier_between_heaps, RequestHeap,
    },
    runtime_value::{
        runtime_map_has as model_runtime_map_has, runtime_values_equal, HeapNode, RuntimeBytes,
        RuntimeMap, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
        RuntimeValueKey,
    },
    service_error::CatchIdentity,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use crate::error::{Result, RuntimeError};

use super::{
    exceptions::exact_target_accepts_catch_identity, runtime_value_view::RuntimeValueView,
};

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

pub fn runtime_carrier_from_wire_required_plan(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let expected_type = required_type(expected_type, boundary)?;
    let value = runtime_from_wire_required_plan(value, Some(expected_type), boundary, heap)?;
    runtime_carrier_for_plan(value, expected_type, boundary, heap)
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

pub fn runtime_carrier_for_plan(
    value: impl Into<RuntimeValueCarrier>,
    expected_type: &RuntimeTypePlan,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let value = value.into();
    if let (Some(actual), Some(expected)) = (value.catch_identity(), expected_type.catch_identity())
    {
        if !exact_target_accepts_catch_identity(actual, expected) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{boundary} materialized value carries an exact identity that does not match its linked type plan"
            )));
        }
    }
    let carrier = match expected_type.node() {
        RuntimeTypeNode::Alias(inner) | RuntimeTypeNode::Nullable(inner) => {
            if matches!(value.value(), RuntimeValue::Null) {
                value
            } else {
                runtime_carrier_for_plan(value, inner, boundary, heap)?
            }
        }
        RuntimeTypeNode::Union(branches) => {
            let matching = if let Some(identity) = value.catch_identity() {
                branches
                    .iter()
                    .filter(|branch| runtime_plan_accepts_identity(branch, identity))
                    .collect::<Vec<_>>()
            } else {
                let mut matching = Vec::new();
                for branch in branches {
                    let matches = RuntimeBoundaryContract::default()
                        .codec_for_expected(branch, BoundaryUse::TypedJson, boundary)
                        .to_wire_json(value.value(), heap)
                        .is_ok();
                    if matches {
                        matching.push(branch);
                    }
                }
                matching
            };
            match matching.as_slice() {
                [branch] => runtime_carrier_for_plan(value, branch, boundary, heap)?,
                [] => Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized value does not match any exact linked type-plan branch"
                )))?,
                _ => Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized value ambiguously matches multiple linked type-plan branches"
                )))?,
            }
        }
        RuntimeTypeNode::Representation { payload, .. } => runtime_carrier_for_plan(
            RuntimeValueCarrier::unidentified(value.into_value()),
            payload,
            boundary,
            heap,
        )?,
        RuntimeTypeNode::Array(item_plan) => {
            let (value, identity) = value.into_parts();
            let RuntimeValue::Heap(handle) = value else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized Array is not a heap value"
                )));
            };
            let item_count = match heap.get(handle)? {
                HeapNode::Array(items) => items.len(),
                _ => {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "{boundary} materialized Array has a non-array heap node"
                    )))
                }
            };
            for index in 0..item_count {
                let item = heap.array_item_carrier(handle, index)?.ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "{boundary} materialized Array item {index} is missing"
                    ))
                })?;
                let item = runtime_carrier_for_plan(item, item_plan, boundary, heap)?;
                heap.set_array_item_carrier(handle, index, item)?;
            }
            RuntimeValueCarrier::from_parts(RuntimeValue::Heap(handle), identity)
        }
        RuntimeTypeNode::Map {
            value: value_plan, ..
        } => {
            let (value, identity) = value.into_parts();
            let RuntimeValue::Heap(handle) = value else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized Map is not a heap value"
                )));
            };
            let keys = match heap.get(handle)? {
                HeapNode::Map(map) => map.keys().cloned().collect::<Vec<_>>(),
                _ => {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "{boundary} materialized Map has a non-map heap node"
                    )))
                }
            };
            for key in keys {
                let item = heap.map_entry_carrier(handle, &key)?.ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "{boundary} materialized Map entry is missing"
                    ))
                })?;
                let item = runtime_carrier_for_plan(item, value_plan, boundary, heap)?;
                heap.set_map_entry_carrier(handle, key, item)?;
            }
            RuntimeValueCarrier::from_parts(RuntimeValue::Heap(handle), identity)
        }
        RuntimeTypeNode::Record { fields, .. } => {
            let (value, identity) = value.into_parts();
            let RuntimeValue::Heap(handle) = value else {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized record is not a heap value"
                )));
            };
            if !matches!(heap.get(handle)?, HeapNode::Object(_) | HeapNode::Map(_)) {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "{boundary} materialized record has a non-record heap node"
                )));
            }
            for field in fields {
                let Some(value) = heap.object_field_carrier(handle, &field.name)? else {
                    continue;
                };
                let value = runtime_carrier_for_plan(value, &field.ty, boundary, heap)?;
                heap.set_object_field_carrier(handle, field.name.clone(), value)?;
            }
            RuntimeValueCarrier::from_parts(RuntimeValue::Heap(handle), identity)
        }
        _ => value,
    };
    match expected_type.catch_identity() {
        Some(identity) => Ok(RuntimeValueCarrier::identified(
            carrier.into_value(),
            identity.clone(),
        )),
        None => Ok(carrier),
    }
}

pub fn runtime_representation_wrap_for_plan(
    value: RuntimeValueCarrier,
    target: &RuntimeTypePlan,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let RuntimeTypeNode::Representation { payload, .. } = target.node() else {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{boundary} target did not resolve to an exact representation plan"
        )));
    };
    let Some(CatchIdentity::Nominal(target_identity)) = target.catch_identity() else {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{boundary} target representation is missing its exact nominal identity"
        )));
    };
    if runtime_plan_has_unidentified_representation(payload) {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{boundary} target representation payload plan is missing an exact identity"
        )));
    }

    if let Some(actual) = value.catch_identity() {
        if !runtime_plan_accepts_identity(payload, actual) {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{boundary} payload carries an exact identity that conflicts with the representation payload plan"
            )));
        }
    }
    let mut validation_heap = RequestHeap::new(heap.limits().clone());
    let validation_value =
        deep_clone_runtime_value_carrier_between_heaps(heap, &mut validation_heap, &value)?;
    let validated = runtime_carrier_for_plan(
        validation_value.clone(),
        payload,
        boundary,
        &mut validation_heap,
    )?;
    if value.catch_identity().is_none() && validated.catch_identity().is_some() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "{boundary} payload is missing the exact identity required by the representation payload plan"
        )));
    }
    let validation_plan = value
        .catch_identity()
        .and_then(|identity| runtime_plan_for_identity(payload, identity))
        .unwrap_or(payload);
    RuntimeBoundaryContract::default()
        .codec_for_expected(validation_plan, BoundaryUse::TypedJson, boundary)
        .to_wire_json(validation_value.value(), &mut validation_heap)?;

    Ok(RuntimeValueCarrier::identified(
        value.into_value(),
        CatchIdentity::Nominal(target_identity.clone()),
    ))
}

fn runtime_plan_has_unidentified_representation(plan: &RuntimeTypePlan) -> bool {
    match plan.node() {
        RuntimeTypeNode::Representation { .. } if plan.catch_identity().is_none() => true,
        RuntimeTypeNode::Alias(inner)
        | RuntimeTypeNode::Nullable(inner)
        | RuntimeTypeNode::Stream(inner)
        | RuntimeTypeNode::Array(inner)
        | RuntimeTypeNode::Representation { payload: inner, .. } => {
            runtime_plan_has_unidentified_representation(inner)
        }
        RuntimeTypeNode::Union(branches) => branches
            .iter()
            .any(runtime_plan_has_unidentified_representation),
        RuntimeTypeNode::Map { key, value } => {
            runtime_plan_has_unidentified_representation(key)
                || runtime_plan_has_unidentified_representation(value)
        }
        RuntimeTypeNode::Record { fields, .. } => fields
            .iter()
            .any(|field| runtime_plan_has_unidentified_representation(&field.ty)),
        _ => false,
    }
}

fn runtime_plan_accepts_identity(plan: &RuntimeTypePlan, identity: &CatchIdentity) -> bool {
    if plan
        .catch_identity()
        .is_some_and(|target| exact_target_accepts_catch_identity(identity, target))
    {
        return true;
    }
    match plan.node() {
        RuntimeTypeNode::Alias(inner) | RuntimeTypeNode::Nullable(inner) => {
            runtime_plan_accepts_identity(inner, identity)
        }
        RuntimeTypeNode::Union(branches) => branches
            .iter()
            .any(|branch| runtime_plan_accepts_identity(branch, identity)),
        _ => false,
    }
}

fn runtime_plan_for_identity<'a>(
    plan: &'a RuntimeTypePlan,
    identity: &CatchIdentity,
) -> Option<&'a RuntimeTypePlan> {
    if plan
        .catch_identity()
        .is_some_and(|target| exact_target_accepts_catch_identity(identity, target))
    {
        return Some(plan);
    }
    match plan.node() {
        RuntimeTypeNode::Alias(inner) | RuntimeTypeNode::Nullable(inner) => {
            runtime_plan_for_identity(inner, identity)
        }
        RuntimeTypeNode::Union(branches) => branches
            .iter()
            .find_map(|branch| runtime_plan_for_identity(branch, identity)),
        _ => None,
    }
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
            HeapNode::Exception(_) => {
                return Err(RuntimeError::Decode(
                    "request-local exception cannot be coerced to bool".to_string(),
                ));
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
            HeapNode::Exception(_) => {
                return Err(RuntimeError::Decode(
                    "request-local exception cannot be stringified".to_string(),
                ));
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
            HeapNode::Exception(exception) => match field {
                "error" => exception
                    .local_value()
                    .map(|payload| payload.value().clone())
                    .ok_or_else(|| {
                        RuntimeError::Decode(
                            "request-local Exception.error has no caller-local payload".to_string(),
                        )
                    }),
                _ => Err(RuntimeError::Decode(format!(
                    "unknown request-local Exception member `{field}`"
                ))),
            },
            _ => Ok(RuntimeValue::Null),
        },
        _ => Ok(RuntimeValue::Null),
    }
}

pub fn runtime_member_access_carrier(
    value: &RuntimeValueCarrier,
    field: &str,
    heap: &RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let RuntimeValue::Heap(handle) = value.value() else {
        return Ok(RuntimeValue::Null.into());
    };
    if let HeapNode::Exception(exception) = heap.get(*handle)? {
        return match field {
            "error" => exception.local_value().cloned().ok_or_else(|| {
                RuntimeError::Decode(
                    "request-local Exception.error has no caller-local payload".to_string(),
                )
            }),
            _ => Err(RuntimeError::Decode(format!(
                "unknown request-local Exception member `{field}`"
            ))),
        };
    }
    Ok(heap
        .object_field_carrier(*handle, field)?
        .unwrap_or_else(|| RuntimeValue::Null.into()))
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

pub fn runtime_array_item_carriers(
    value: &RuntimeValueCarrier,
    heap: &RequestHeap,
) -> Result<Option<Vec<RuntimeValueCarrier>>> {
    let RuntimeValue::Heap(handle) = value.value() else {
        return Ok(None);
    };
    let HeapNode::Array(items) = heap.get(*handle)? else {
        return Ok(None);
    };
    (0..items.len())
        .map(|index| heap.array_item_carrier(*handle, index))
        .collect::<skiff_runtime_model::error::Result<Vec<_>>>()
        .map(|items| Some(items.into_iter().flatten().collect()))
        .map_err(RuntimeError::from)
}

pub fn runtime_map_get(
    receiver: &RuntimeValue,
    key: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    RuntimeValueView::new(receiver, heap).map_get(key)
}

pub fn runtime_map_get_carrier(
    receiver: &RuntimeValueCarrier,
    key: &RuntimeValueCarrier,
    heap: &RequestHeap,
) -> Result<RuntimeValueCarrier> {
    let RuntimeValue::Heap(handle) = receiver.value() else {
        return Ok(RuntimeValue::Null.into());
    };
    let key = RuntimeValueKey::string(runtime_stringify_key(key.value(), heap)?);
    Ok(heap
        .map_entry_carrier(*handle, &key)?
        .unwrap_or_else(|| RuntimeValue::Null.into()))
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

pub fn runtime_deep_clone_carrier(
    value: &RuntimeValueCarrier,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    Ok(deep_clone_runtime_value_carrier(heap, value)?)
}

pub fn runtime_object_from_fields(
    fields: BTreeMap<String, RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let object = RuntimeObject::unshaped(fields);
    Ok(RuntimeValue::Heap(heap.alloc_object(object)?))
}

pub fn runtime_object_from_carriers(
    fields: BTreeMap<String, RuntimeValueCarrier>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    Ok(RuntimeValueCarrier::unidentified(RuntimeValue::Heap(
        heap.alloc_object_carriers(fields)?,
    )))
}

pub fn runtime_map_from_entries(
    entries: RuntimeMap,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeValue::Heap(heap.alloc_map(entries)?))
}

pub fn runtime_map_from_carriers(
    entries: BTreeMap<RuntimeValueKey, RuntimeValueCarrier>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    Ok(RuntimeValueCarrier::unidentified(RuntimeValue::Heap(
        heap.alloc_map_carriers(entries)?,
    )))
}

pub fn runtime_array_from_items(
    items: Vec<RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    Ok(RuntimeValue::Heap(heap.alloc_array(items)?))
}

pub fn runtime_array_from_carriers(
    items: Vec<RuntimeValueCarrier>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValueCarrier> {
    Ok(RuntimeValueCarrier::unidentified(RuntimeValue::Heap(
        heap.alloc_array_carriers(items)?,
    )))
}

fn stringify_number(value: f64) -> String {
    if value.is_finite() && value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests;

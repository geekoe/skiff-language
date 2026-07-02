use crate::{
    date_value,
    error::{Result, RuntimeError},
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceValue, RuntimeMap, RuntimeObjectFields, RuntimeValue, RuntimeValueKey,
    },
};

use super::{context::RuntimeCoerceContext, keys::require_plain_runtime_key};

pub(super) fn reject_reserved_legacy_json_metadata_key(key: &str) -> Result<()> {
    if is_reserved_skiff_metadata_key(key) {
        return Err(RuntimeError::Decode(format!(
            "reserved Skiff metadata field {key} is not allowed at JSON boundary"
        )));
    }
    Ok(())
}

fn is_reserved_skiff_metadata_key(key: &str) -> bool {
    key.strip_prefix("__skiff")
        .is_some_and(|suffix| suffix == "Type")
}

pub(super) fn validate_json_runtime_value(
    value: &RuntimeValue,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<()> {
    context.check_depth(depth)?;
    match value {
        RuntimeValue::Null
        | RuntimeValue::Bool(_)
        | RuntimeValue::Date(_)
        | RuntimeValue::String(_) => Ok(()),
        RuntimeValue::Number(value) if value.is_finite() => Ok(()),
        RuntimeValue::Number(_) => Err(RuntimeError::Decode("number is not finite".to_string())),
        RuntimeValue::ActorRef(actor_ref) => Err(RuntimeError::Decode(format!(
            "actor ref {} is not a JSON value",
            actor_ref.actor_type_identity()
        ))),
        RuntimeValue::Heap(handle) => context.with_active_handle(*handle, |context| {
            let node = heap.get(*handle)?.clone();
            match node {
                HeapNode::Bytes(_) => Ok(()),
                HeapNode::Array(items) => {
                    for item in &items {
                        validate_json_runtime_value(item, heap, context, depth + 1)?;
                    }
                    Ok(())
                }
                HeapNode::Object(object) => {
                    for (key, value) in object.fields() {
                        reject_reserved_legacy_json_metadata_key(key)?;
                        validate_json_runtime_value(value, heap, context, depth + 1)?;
                    }
                    Ok(())
                }
                HeapNode::Map(map) => {
                    for (key, value) in &map {
                        require_plain_runtime_key(key)?;
                        reject_reserved_legacy_json_metadata_key(key.string_payload())?;
                        validate_json_runtime_value(value, heap, context, depth + 1)?;
                    }
                    Ok(())
                }
                HeapNode::Interface(value) => Err(interface_json_error(&value)),
            }
        }),
    }
}

pub(super) fn strip_runtime_value_to_json(
    value: &RuntimeValue,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    context.check_depth(depth)?;
    match value {
        RuntimeValue::Null | RuntimeValue::Bool(_) | RuntimeValue::String(_) => Ok(value.clone()),
        RuntimeValue::Date(ms) => Ok(RuntimeValue::String(date_value::format_epoch_millis(
            *ms,
            "std.json Date encode",
        )?)),
        RuntimeValue::Number(value) if value.is_finite() => Ok(RuntimeValue::Number(*value)),
        RuntimeValue::Number(_) => Err(RuntimeError::Decode("number is not finite".to_string())),
        RuntimeValue::ActorRef(actor_ref) => Err(RuntimeError::Decode(format!(
            "actor ref {} is not a JSON value",
            actor_ref.actor_type_identity()
        ))),
        RuntimeValue::Heap(handle) => context.with_active_handle(*handle, |context| {
            let node = heap.get(*handle)?.clone();
            match node {
                HeapNode::Bytes(_) => Ok(value.clone()),
                HeapNode::Array(items) => {
                    strip_runtime_array_to_json(value, &items, heap, context, depth)
                }
                HeapNode::Object(object) => {
                    strip_runtime_fields_to_json_map(object.fields(), heap, context, depth)
                }
                HeapNode::Map(map) => strip_runtime_map_to_json(value, &map, heap, context, depth),
                HeapNode::Interface(value) => Err(interface_json_error(&value)),
            }
        }),
    }
}

fn strip_runtime_array_to_json(
    original: &RuntimeValue,
    items: &[RuntimeValue],
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    let mut changed = false;
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        let stripped = strip_runtime_value_to_json(item, heap, context, depth + 1)?;
        changed |= stripped != *item;
        output.push(stripped);
    }
    if !changed {
        return Ok(original.clone());
    }
    Ok(RuntimeValue::Heap(heap.alloc_array(output)?))
}

fn strip_runtime_fields_to_json_map(
    fields: &RuntimeObjectFields,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    let mut output = RuntimeMap::new();
    for (key, value) in fields {
        reject_reserved_legacy_json_metadata_key(key)?;
        output.insert(
            RuntimeValueKey::string(key),
            strip_runtime_value_to_json(value, heap, context, depth + 1)?,
        );
    }
    Ok(RuntimeValue::Heap(heap.alloc_map(output)?))
}

fn strip_runtime_map_to_json(
    original: &RuntimeValue,
    map: &RuntimeMap,
    heap: &mut RequestHeap,
    context: &mut RuntimeCoerceContext,
    depth: usize,
) -> Result<RuntimeValue> {
    let mut changed = false;
    let mut output = RuntimeMap::new();
    for (key, value) in map {
        reject_reserved_legacy_json_metadata_key(key.string_payload())?;
        let stripped = strip_runtime_value_to_json(value, heap, context, depth + 1)?;
        changed |= stripped != *value;
        output.insert(RuntimeValueKey::string(key.string_payload()), stripped);
    }
    if !changed {
        return Ok(original.clone());
    }
    Ok(RuntimeValue::Heap(heap.alloc_map(output)?))
}

fn interface_json_error(value: &InterfaceValue) -> RuntimeError {
    RuntimeError::Decode(format!(
        "{} is not an ordinary JSON value",
        value.diagnostic_label()
    ))
}

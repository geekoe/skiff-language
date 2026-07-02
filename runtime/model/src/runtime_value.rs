use crate::{
    error::{Result, RuntimeModelError as RuntimeError},
    request_heap::RequestHeap,
};

pub use crate::value::*;

pub fn runtime_values_equal(
    heap: &RequestHeap,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<bool> {
    runtime_values_equal_inner(heap, left, right)
}

pub fn runtime_map_has(
    heap: &RequestHeap,
    receiver: &RuntimeValue,
    key: &RuntimeValue,
) -> Result<bool> {
    match receiver {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Map(map) => {
                let key = map_key_from_runtime_value(heap, key)?;
                Ok(map.contains_key(&key))
            }
            HeapNode::Object(object) => {
                let key = object_key_from_runtime_value(key)?;
                Ok(object.fields().contains_key(&key))
            }
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "{} is not map-like",
                value.diagnostic_label()
            ))),
            _ => Ok(false),
        },
        _ => Ok(false),
    }
}

fn runtime_values_equal_inner(
    heap: &RequestHeap,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<bool> {
    if let Some(label) = interface_equality_label(heap, left, right)? {
        return Err(RuntimeError::Decode(format!(
            "interface value {label} does not define equality"
        )));
    }

    match (left, right) {
        (RuntimeValue::Null, RuntimeValue::Null) => Ok(true),
        (RuntimeValue::Bool(left), RuntimeValue::Bool(right)) => Ok(left == right),
        (RuntimeValue::Number(left), RuntimeValue::Number(right)) => Ok(left == right),
        (RuntimeValue::Date(left), RuntimeValue::Date(right)) => Ok(left == right),
        (RuntimeValue::String(left), RuntimeValue::String(right)) => Ok(left == right),
        (RuntimeValue::ActorRef(left), RuntimeValue::ActorRef(right)) => Ok(left == right),
        (RuntimeValue::Heap(left), RuntimeValue::Heap(right)) if left == right => Ok(true),
        (RuntimeValue::Heap(left), RuntimeValue::Heap(right)) => {
            match (heap.get(*left)?, heap.get(*right)?) {
                (HeapNode::Bytes(left), HeapNode::Bytes(right)) => Ok(left == right),
                _ => Ok(false),
            }
        }
        _ => Ok(false),
    }
}

fn interface_equality_label(
    heap: &RequestHeap,
    left: &RuntimeValue,
    right: &RuntimeValue,
) -> Result<Option<String>> {
    if let Some(label) = interface_value_label(heap, left)? {
        return Ok(Some(label));
    }
    interface_value_label(heap, right)
}

fn interface_value_label(heap: &RequestHeap, value: &RuntimeValue) -> Result<Option<String>> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(None);
    };
    Ok(match heap.get(*handle)? {
        HeapNode::Interface(value) => Some(value.diagnostic_label()),
        _ => None,
    })
}

fn map_key_from_runtime_value(heap: &RequestHeap, value: &RuntimeValue) -> Result<RuntimeValueKey> {
    let _ = heap;
    match value {
        RuntimeValue::String(value) => Ok(RuntimeValueKey::string(value)),
        _ => Err(invalid_map_key_error()),
    }
}

fn object_key_from_runtime_value(value: &RuntimeValue) -> Result<String> {
    match value {
        RuntimeValue::String(value) => Ok(value.clone()),
        _ => Err(invalid_map_key_error()),
    }
}

fn invalid_map_key_error() -> RuntimeError {
    RuntimeError::Decode("Map key must be string or representation-over-string".to_string())
}

#[cfg(test)]
mod tests;

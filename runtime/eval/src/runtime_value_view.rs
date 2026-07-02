//! Read-only RuntimeValue helpers that need heap access.

use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeBytes, RuntimeValue},
};

use crate::error::{Result, RuntimeError};

use super::mutable_path::map_key_from_runtime_value;

pub struct RuntimeValueView<'a> {
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
}

impl<'a> RuntimeValueView<'a> {
    pub fn new(value: &'a RuntimeValue, heap: &'a RequestHeap) -> Self {
        Self { value, heap }
    }

    pub fn string_payload(&self) -> Result<Option<&'a str>> {
        string_payload(self.value, self.heap)
    }

    pub fn bytes_payload(&self) -> Result<Option<&'a RuntimeBytes>> {
        bytes_payload(self.value, self.heap)
    }

    pub fn bytes_payload_from(
        value: &'a RuntimeValue,
        heap: &'a RequestHeap,
    ) -> Result<Option<&'a RuntimeBytes>> {
        bytes_payload(value, heap)
    }

    pub fn array_items(&self) -> Result<Option<Vec<RuntimeValue>>> {
        array_items(self.value, self.heap)
    }

    pub fn is_map_like(&self) -> Result<bool> {
        is_map_like(self.value, self.heap)
    }

    pub fn map_like_len(&self) -> Result<usize> {
        map_like_len(self.value, self.heap)
    }

    pub fn map_get(&self, key: &RuntimeValue) -> Result<RuntimeValue> {
        match self.value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Map(map) => {
                    let key = map_key_from_runtime_value(key, self.heap)?;
                    Ok(map.get(&key).cloned().unwrap_or(RuntimeValue::Null))
                }
                HeapNode::Object(object) => {
                    let key = object_key_from_runtime_value(key)?;
                    Ok(object
                        .fields()
                        .get(&key)
                        .cloned()
                        .unwrap_or(RuntimeValue::Null))
                }
                HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                    "{} is not map-like",
                    value.diagnostic_label()
                ))),
                _ => Ok(RuntimeValue::Null),
            },
            _ => Ok(RuntimeValue::Null),
        }
    }

    pub fn map_has(&self, key: &RuntimeValue) -> Result<bool> {
        match self.value {
            RuntimeValue::Heap(handle) => match self.heap.get(*handle)? {
                HeapNode::Map(map) => {
                    let key = map_key_from_runtime_value(key, self.heap)?;
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
}

fn string_payload<'a>(value: &'a RuntimeValue, heap: &'a RequestHeap) -> Result<Option<&'a str>> {
    match value {
        RuntimeValue::String(value) => Ok(Some(value.as_str())),
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn bytes_payload<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
) -> Result<Option<&'a RuntimeBytes>> {
    match value {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Bytes(bytes) => Ok(Some(bytes)),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn array_items(value: &RuntimeValue, heap: &RequestHeap) -> Result<Option<Vec<RuntimeValue>>> {
    match value {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Array(items) => Ok(Some(items.clone())),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn is_map_like(value: &RuntimeValue, heap: &RequestHeap) -> Result<bool> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(false);
    };
    match heap.get(*handle)? {
        HeapNode::Map(_) | HeapNode::Object(_) => Ok(true),
        HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
            "{} is not map-like",
            value.diagnostic_label()
        ))),
        _ => Ok(false),
    }
}

fn map_like_len(value: &RuntimeValue, heap: &RequestHeap) -> Result<usize> {
    let RuntimeValue::Heap(handle) = value else {
        return Ok(0);
    };
    match heap.get(*handle)? {
        HeapNode::Map(map) => Ok(map.len()),
        HeapNode::Object(object) => Ok(object.fields().len()),
        HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
            "{} is not map-like",
            value.diagnostic_label()
        ))),
        _ => Ok(0),
    }
}

fn object_key_from_runtime_value(value: &RuntimeValue) -> Result<String> {
    match value {
        RuntimeValue::String(value) => Ok(value.clone()),
        _ => Err(RuntimeError::Decode(
            "Map key must be string or representation-over-string".to_string(),
        )),
    }
}

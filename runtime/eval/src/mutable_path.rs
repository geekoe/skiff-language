use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, HeapNode, RuntimeValue, RuntimeValueKey},
};

use crate::error::{Result, RuntimeError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSegment {
    Field(String),
    ArrayIndex(usize),
    MapKey(RuntimeValueKey),
    CollectionReceiver,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CollectionMutation {
    ArrayPush(RuntimeValue),
    ArraySet {
        index: usize,
        value: RuntimeValue,
    },
    ArrayPop,
    MapSet {
        key: RuntimeValueKey,
        value: RuntimeValue,
    },
    MapDelete {
        key: RuntimeValueKey,
    },
    ObjectSetField {
        field: String,
        value: RuntimeValue,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CollectionMutationResult {
    Unit,
    Value(RuntimeValue),
    Existed(bool),
}

pub fn map_key_from_runtime_value(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<RuntimeValueKey> {
    let _ = heap;
    match value {
        RuntimeValue::String(value) => Ok(RuntimeValueKey::string(value)),
        _ => Err(invalid_map_key_error()),
    }
}

pub fn resolve_target_handle(
    root_value: &RuntimeValue,
    segments: &[PathSegment],
    heap: &RequestHeap,
) -> Result<HeapHandle> {
    let mut current = root_value.clone();
    for segment in segments {
        current = read_segment(&current, segment, heap)?;
    }
    current.as_heap_handle().ok_or_else(|| {
        RuntimeError::Decode("mutable target path does not resolve to a heap node".to_string())
    })
}

pub fn resolve_parent_handle_and_field(
    root_value: &RuntimeValue,
    segments: &[PathSegment],
    heap: &RequestHeap,
) -> Result<(HeapHandle, String)> {
    let Some((last, parent_segments)) = segments.split_last() else {
        return Err(RuntimeError::Decode(
            "mutable target path is missing a field segment".to_string(),
        ));
    };
    let PathSegment::Field(field) = last else {
        return Err(RuntimeError::Decode(format!(
            "field assignment target path must end with Field, got {}",
            last.label()
        )));
    };
    let parent = resolve_target_handle(root_value, parent_segments, heap)?;
    Ok((parent, field.clone()))
}

pub fn apply_collection_mutation(
    heap: &mut RequestHeap,
    target_handle: HeapHandle,
    mutation: CollectionMutation,
) -> Result<CollectionMutationResult> {
    match mutation {
        CollectionMutation::ArrayPush(value) => {
            heap.push_array_item(target_handle, value)?;
            Ok(CollectionMutationResult::Unit)
        }
        CollectionMutation::ArraySet { index, value } => {
            heap.set_array_item(target_handle, index, value)?;
            Ok(CollectionMutationResult::Unit)
        }
        CollectionMutation::ArrayPop => {
            let popped = heap.pop_array_item(target_handle)?;
            Ok(CollectionMutationResult::Value(popped))
        }
        CollectionMutation::MapSet { key, value } => {
            let existed = heap.set_map_entry(target_handle, key, value)?;
            Ok(CollectionMutationResult::Existed(existed))
        }
        CollectionMutation::MapDelete { key } => {
            let existed = heap.delete_map_entry(target_handle, &key)?;
            Ok(CollectionMutationResult::Existed(existed))
        }
        CollectionMutation::ObjectSetField { field, value } => {
            heap.set_object_field(target_handle, field, value)?;
            Ok(CollectionMutationResult::Unit)
        }
    }
}

fn read_segment(
    value: &RuntimeValue,
    segment: &PathSegment,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    if matches!(segment, PathSegment::CollectionReceiver) {
        return Ok(value.clone());
    }
    let handle = value.as_heap_handle().ok_or_else(|| {
        RuntimeError::Decode("mutable target path segment parent is not a heap node".to_string())
    })?;
    match (heap.get(handle)?, segment) {
        (HeapNode::Object(object), PathSegment::Field(field)) => {
            object.fields().get(field).cloned().ok_or_else(|| {
                RuntimeError::Decode(format!("mutable target field {field} is missing"))
            })
        }
        (HeapNode::Map(map), PathSegment::Field(field)) => map
            .get(&RuntimeValueKey::string(field))
            .cloned()
            .ok_or_else(|| {
                RuntimeError::Decode(format!("mutable target field {field} is missing"))
            }),
        (HeapNode::Array(items), PathSegment::ArrayIndex(index)) => {
            items.get(*index).cloned().ok_or_else(|| {
                RuntimeError::Decode(format!(
                    "mutable target array index {index} is out of bounds"
                ))
            })
        }
        (HeapNode::Map(map), PathSegment::MapKey(key)) => map.get(key).cloned().ok_or_else(|| {
            RuntimeError::Decode(format!("mutable target map key {} is missing", key.label()))
        }),
        (HeapNode::Object(object), PathSegment::MapKey(RuntimeValueKey::String(key))) => {
            object.fields().get(key).cloned().ok_or_else(|| {
                RuntimeError::Decode(format!("mutable target map key {key} is missing"))
            })
        }
        (_, PathSegment::Field(field)) => Err(RuntimeError::Decode(format!(
            "mutable target field {field} parent is not an object"
        ))),
        (_, PathSegment::ArrayIndex(index)) => Err(RuntimeError::Decode(format!(
            "mutable target array index {index} parent is not an array"
        ))),
        (_, PathSegment::MapKey(key)) => Err(RuntimeError::Decode(format!(
            "mutable target map key {} parent is not a map",
            key.label()
        ))),
        (_, PathSegment::CollectionReceiver) => unreachable!("CollectionReceiver handled above"),
    }
}

fn invalid_map_key_error() -> RuntimeError {
    RuntimeError::Decode("Map key must be string or representation-over-string".to_string())
}

impl PathSegment {
    fn label(&self) -> String {
        match self {
            Self::Field(name) => format!("Field({name})"),
            Self::ArrayIndex(index) => format!("ArrayIndex({index})"),
            Self::MapKey(key) => format!("MapKey({})", key.label()),
            Self::CollectionReceiver => "CollectionReceiver".to_string(),
        }
    }
}

trait RuntimeValueKeyLabel {
    fn label(&self) -> String;
}

impl RuntimeValueKeyLabel for RuntimeValueKey {
    fn label(&self) -> String {
        match self {
            RuntimeValueKey::String(value) => value.clone(),
        }
    }
}

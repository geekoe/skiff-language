use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapHandle, HeapNode, RuntimeValue, RuntimeValueCarrier},
};

use super::{runtime_ops::runtime_array_items, runtime_value_view::RuntimeValueView};
use crate::error::{Result, RuntimeError};

pub fn assign_program_index_target(
    heap: &mut RequestHeap,
    object: &RuntimeValue,
    index: &RuntimeValue,
    value: RuntimeValue,
) -> Result<()> {
    assign_program_index_target_carrier(heap, object, index, value.into())
}

pub fn assign_program_index_target_carrier(
    heap: &mut RequestHeap,
    object: &RuntimeValue,
    index: &RuntimeValue,
    value: RuntimeValueCarrier,
) -> Result<()> {
    let handle = program_mutable_receiver_handle(object, heap, "index assignment")?;
    if runtime_array_items(object, heap)?.is_some() {
        let index = runtime_u64(index).ok_or_else(|| {
            RuntimeError::Decode(
                "array index assignment index must be a non-negative number".to_string(),
            )
        })?;
        heap.set_array_item_carrier(handle, index as usize, value)?;
        return Ok(());
    }
    if RuntimeValueView::new(object, heap).is_map_like()? {
        let key = super::mutable_path::map_key_from_runtime_value(index, heap)?;
        heap.set_map_entry_carrier(handle, key, value)?;
        return Ok(());
    }
    Err(RuntimeError::Decode(
        "index assignment target must be an array, map, or object".to_string(),
    ))
}

pub fn program_mutable_receiver_handle(
    value: &RuntimeValue,
    heap: &RequestHeap,
    context: &str,
) -> Result<HeapHandle> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(RuntimeError::Decode(format!(
            "{context} receiver must be a heap value"
        )));
    };
    match heap.get(*handle)? {
        HeapNode::Array(_) | HeapNode::Map(_) | HeapNode::Object(_) => Ok(*handle),
        HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
            "{context} receiver {} is not mutable",
            value.diagnostic_label()
        ))),
        HeapNode::Bytes(_) | HeapNode::Exception(_) => Err(RuntimeError::Decode(format!(
            "{context} receiver is not mutable"
        ))),
    }
}

pub fn runtime_u64(value: &RuntimeValue) -> Option<u64> {
    match value {
        RuntimeValue::Number(value) if *value >= 0.0 && value.fract() == 0.0 => Some(*value as u64),
        _ => None,
    }
}

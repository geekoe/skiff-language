use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{decode_base64, HeapNode, RequestHeap, RuntimeValue};

pub(crate) fn runtime_string_arg<'a>(value: &'a RuntimeValue, label: &str) -> Result<&'a str> {
    match value {
        RuntimeValue::String(value) => Ok(value.as_str()),
        _ => Err(RuntimeError::Decode(format!("{label} must be a string"))),
    }
}

pub(crate) fn is_runtime_bytes_native(target: &str) -> bool {
    matches!(
        target,
        "core.bytes.fromBase64"
            | "core.bytes.fromHex"
            | "core.bytes.fromUtf8"
            | "core.bytes.concat"
    )
}

pub(crate) fn eval_program_bytes_native(
    target: &str,
    args: Vec<RuntimeValue>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    match target {
        "core.bytes.fromBase64" => {
            let value = runtime_string_arg(
                args.first().ok_or_else(|| {
                    RuntimeError::Decode("bytes.fromBase64 requires a string".to_string())
                })?,
                "bytes.fromBase64",
            )?;
            let bytes = decode_base64(value)
                .map_err(|error| RuntimeError::bytes_decode("bytes.fromBase64", error))?;
            Ok(RuntimeValue::Heap(heap.alloc_bytes(bytes)?))
        }
        "core.bytes.fromHex" => {
            let value = runtime_string_arg(
                args.first().ok_or_else(|| {
                    RuntimeError::Decode("bytes.fromHex requires a string".to_string())
                })?,
                "bytes.fromHex",
            )?;
            let bytes = hex::decode(value.trim()).map_err(|error| {
                RuntimeError::bytes_decode(
                    "bytes.fromHex",
                    format!("bytes.fromHex decode failed: {error}"),
                )
            })?;
            Ok(RuntimeValue::Heap(heap.alloc_bytes(bytes)?))
        }
        "core.bytes.fromUtf8" => {
            let value = runtime_string_arg(
                args.first().ok_or_else(|| {
                    RuntimeError::Decode("bytes.fromUtf8 requires a string".to_string())
                })?,
                "bytes.fromUtf8",
            )?;
            Ok(RuntimeValue::Heap(heap.alloc_bytes(value.as_bytes())?))
        }
        "core.bytes.concat" => {
            let parts = args.first().ok_or_else(|| {
                RuntimeError::Decode("bytes.concat requires an array".to_string())
            })?;
            let parts = runtime_array_items(parts, heap)?.ok_or_else(|| {
                RuntimeError::Decode("bytes.concat requires an array".to_string())
            })?;
            let mut decoded_parts = Vec::with_capacity(parts.len());
            let mut total_len = 0usize;
            for (index, part) in parts.iter().enumerate() {
                let bytes = runtime_bytes_payload(part, heap)?.ok_or_else(|| {
                    RuntimeError::Decode(format!("bytes.concat part {index} must be bytes"))
                })?;
                total_len = total_len.checked_add(bytes.len()).ok_or_else(|| {
                    RuntimeError::ResourceLimitExceeded {
                        resource: "bytes".to_string(),
                        reason: "concat output length overflow".to_string(),
                        limit: usize::MAX,
                        current: total_len,
                        requested_delta: bytes.len(),
                    }
                })?;
                decoded_parts.push(bytes);
            }
            let mut output = Vec::with_capacity(total_len);
            for part in decoded_parts {
                output.extend_from_slice(part);
            }
            Ok(RuntimeValue::Heap(heap.alloc_bytes(output)?))
        }
        _ => Err(RuntimeError::Unsupported(format!(
            "unsupported RuntimeProgram bytes native {target}"
        ))),
    }
}

fn runtime_array_items(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<Option<Vec<RuntimeValue>>> {
    match value {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Array(items) => Ok(Some(items.clone())),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn runtime_bytes_payload<'a>(
    value: &RuntimeValue,
    heap: &'a RequestHeap,
) -> Result<Option<&'a [u8]>> {
    match value {
        RuntimeValue::Heap(handle) => match heap.get(*handle)? {
            HeapNode::Bytes(bytes) => Ok(Some(bytes.as_slice())),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

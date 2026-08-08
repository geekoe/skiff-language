use super::{unsupported_native_target, RuntimeNativeInvocation};
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};

use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};
use skiff_runtime_model::runtime_value::{HeapNode, RuntimeValueKey};

pub(super) struct JsonNativeDispatch;

impl JsonNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.json.encode"
                | "std.json.decode"
                | "std.json.get"
                | "std.json.getString"
                | "std.json.getNumber"
                | "std.json.getBool"
                | "std.json.getArray"
        )
    }

    pub(super) fn dispatch(
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let binding_key = invocation.binding_key();
        let arg = args.first().ok_or_else(|| {
            RuntimeError::Decode(format!("{diagnostic_target} requires an argument"))
        })?;
        match binding_key {
            "std.json.encode" => {
                if invocation.plan().is_some() {
                    let native_boundary = invocation.native_boundary()?;
                    let encoded = RuntimeBoundaryContract::default()
                        .codec_for_expected(
                            invocation.arg_plan(0)?,
                            BoundaryUse::JsonValueProjection,
                            format!("{diagnostic_target} input"),
                        )
                        .encode_json_text_value(arg, heap)
                        .map_err(|error| {
                            json_codec_decode_error("std.json.encode", error.into())
                        })?;
                    return native_boundary.coerce_return(
                        &RuntimeValue::String(encoded),
                        &format!("{diagnostic_target} response"),
                        heap,
                    );
                }
                let encoded = RuntimeBoundaryContract::default()
                    .encode_json_text(
                        arg,
                        None,
                        BoundaryUse::JsonValueProjection,
                        format!("{diagnostic_target} input"),
                        heap,
                    )
                    .map_err(|error| json_codec_decode_error("std.json.encode", error.into()))?;
                Ok(RuntimeValue::String(encoded))
            }
            "std.json.decode" => {
                if invocation.plan().is_none() {
                    return Err(unsupported_native_target(binding_key));
                }
                let native_boundary = invocation.native_boundary()?;
                let input = native_boundary.coerce_arg(
                    0,
                    arg,
                    &format!("{diagnostic_target} input"),
                    heap,
                )?;
                let RuntimeValue::String(input) = input else {
                    return Err(RuntimeError::Decode(format!(
                        "{diagnostic_target} input must be a string"
                    )));
                };
                RuntimeBoundaryContract::default()
                    .codec_for_expected(
                        invocation.return_plan()?,
                        BoundaryUse::JsonValueProjection,
                        format!("{diagnostic_target} response"),
                    )
                    .decode_json_text(&input, heap)
                    .map_err(|error| json_codec_decode_error("std.json.decode", error.into()))
            }
            "std.json.get" | "std.json.getString" | "std.json.getNumber" | "std.json.getBool"
            | "std.json.getArray" => {
                if invocation.plan().is_none() {
                    return Err(unsupported_native_target(binding_key));
                }
                let native_boundary = invocation.native_boundary()?;
                let arg_count = invocation.arg_count()?;
                if args.len() != arg_count {
                    return Err(RuntimeError::Decode(format!(
                        "{diagnostic_target} expects {arg_count} argument(s), got {}",
                        args.len()
                    )));
                }
                let receiver = &args[0];
                let key = runtime_string_key(&args[1]).ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} key must be a string"))
                })?;
                let field = json_field_value(receiver, key, heap)?;
                let projected = project_json_field(binding_key, field, heap)?;
                native_boundary.coerce_return(
                    &projected,
                    &format!("{diagnostic_target} response"),
                    heap,
                )
            }
            _ => Err(unsupported_native_target(binding_key)),
        }
    }
}

fn runtime_string_key(value: &RuntimeValue) -> Option<&str> {
    match value {
        RuntimeValue::String(value) => Some(value.as_str()),
        _ => None,
    }
}

/// Zero-serialization field read on the shared runtime value: objects and
/// maps are consulted directly on the heap, and only the field itself is
/// cloned. Missing fields and non-map-like receivers yield null.
fn json_field_value(
    receiver: &RuntimeValue,
    key: &str,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    let RuntimeValue::Heap(handle) = receiver else {
        return Ok(RuntimeValue::Null);
    };
    match heap.get(*handle)? {
        HeapNode::Object(object) => Ok(object
            .fields()
            .get(key)
            .cloned()
            .unwrap_or(RuntimeValue::Null)),
        HeapNode::Map(map) => Ok(map
            .get(&RuntimeValueKey::String(key.to_string()))
            .cloned()
            .unwrap_or(RuntimeValue::Null)),
        _ => Ok(RuntimeValue::Null),
    }
}

/// Typed projection of a JSON field: only exact JSON types are kept, every
/// mismatch (including a missing field, which is null) yields null.
fn project_json_field(
    binding_key: &str,
    field: RuntimeValue,
    heap: &RequestHeap,
) -> Result<RuntimeValue> {
    match binding_key {
        "std.json.getString" => Ok(match field {
            RuntimeValue::String(_) => field,
            _ => RuntimeValue::Null,
        }),
        "std.json.getNumber" => Ok(match field {
            RuntimeValue::Number(_) => field,
            _ => RuntimeValue::Null,
        }),
        "std.json.getBool" => Ok(match field {
            RuntimeValue::Bool(_) => field,
            _ => RuntimeValue::Null,
        }),
        "std.json.getArray" => {
            let is_array = matches!(
                field.as_heap_handle().map(|handle| heap.get(handle)),
                Some(Ok(HeapNode::Array(_)))
            );
            Ok(if is_array { field } else { RuntimeValue::Null })
        }
        // std.json.get returns the raw field (null when missing).
        _ => Ok(field),
    }
}

pub(super) fn json_codec_decode_error(target: &'static str, error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::Decode(message) => RuntimeError::decode_target(target, message),
        other => other,
    }
}

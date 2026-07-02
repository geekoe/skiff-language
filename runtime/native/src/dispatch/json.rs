use super::{unsupported_native_target, RuntimeNativeInvocation};
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};

use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

pub(super) struct JsonNativeDispatch;

impl JsonNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(target, "std.json.encode" | "std.json.decode")
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
            _ => Err(unsupported_native_target(binding_key)),
        }
    }
}

pub(super) fn json_codec_decode_error(target: &'static str, error: RuntimeError) -> RuntimeError {
    match error {
        RuntimeError::Decode(message) => RuntimeError::decode_target(target, message),
        other => other,
    }
}

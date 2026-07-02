use super::{unsupported_native_target, RuntimeNativeInvocation};
use skiff_runtime_boundary::json::{decode_untyped_wire_json, encode_untyped_wire_json};

use crate::error::{Result, RuntimeError};
use crate::registry::NativeRegistry;
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

pub(super) struct ExternalNativeDispatch;

impl ExternalNativeDispatch {
    pub(super) fn is_registered(target: &str) -> bool {
        NativeRegistry.is_registered(target)
    }

    pub(super) fn dispatch(
        target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let args_json = args
            .iter()
            .map(|value| Ok(encode_untyped_wire_json(value, heap)?))
            .collect::<Result<Vec<_>>>()?;
        let value = NativeRegistry.dispatch(target, &args_json)?;
        let value = value.ok_or_else(|| unsupported_native_target(target))?;
        Ok(decode_untyped_wire_json(&value, heap)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_native_call(
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let binding_key = invocation.binding_key();
        let arg_count = invocation.arg_count()?;
        if args.len() != arg_count {
            return Err(RuntimeError::Decode(format!(
                "{diagnostic_target} expects {} argument(s), got {}",
                arg_count,
                args.len()
            )));
        }
        let native_boundary = invocation.native_boundary()?;
        let wire_args = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                native_boundary.to_wire_arg(
                    index,
                    arg,
                    &format!("{diagnostic_target} arg[{index}]"),
                    heap,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let value = NativeRegistry.dispatch(binding_key, &wire_args)?;
        let value = value.ok_or_else(|| unsupported_native_target(binding_key))?;
        native_boundary.from_wire_return(&value, &format!("{diagnostic_target} response"), heap)
    }
}

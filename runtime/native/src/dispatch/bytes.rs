use super::RuntimeNativeInvocation;
use crate::call_helpers::{eval_program_bytes_native, is_runtime_bytes_native};
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

pub(super) struct BytesNativeDispatch;

impl BytesNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        is_runtime_bytes_native(target)
    }

    pub(super) fn dispatch(
        target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        eval_program_bytes_native(target, args, heap)
    }

    pub(super) fn dispatch_native_call(
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let binding_key = invocation.binding_key();
        let arg_count = invocation.arg_count()?;
        if args.len() != arg_count {
            return Err(RuntimeError::InvalidArtifact(format!(
                "{diagnostic_target} evaluated {} args, but native signature resolved {} args",
                args.len(),
                arg_count
            )));
        }
        let native_boundary = invocation.native_boundary()?;

        let coerced_args = args
            .iter()
            .enumerate()
            .map(|(index, arg)| {
                let label = format!("{diagnostic_target} argument {index}");
                native_boundary.coerce_arg(index, arg, label.as_str(), heap)
            })
            .collect::<Result<Vec<_>>>()?;
        let output = eval_program_bytes_native(binding_key, coerced_args, heap)?;
        let label = format!("{diagnostic_target} response");
        native_boundary.coerce_return(&output, label.as_str(), heap)
    }
}

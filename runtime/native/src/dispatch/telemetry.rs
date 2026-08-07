use super::RuntimeNativeInvocation;
use crate::capability::NativeTelemetryCapability;
use crate::error::Result;
use crate::runtime_value_facade::{RequestHeap, RuntimeTypeNode, RuntimeTypePlan, RuntimeValue};
use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

pub(super) struct TelemetryNativeDispatch;

impl TelemetryNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(target, "std.telemetry.emit" | "root.telemetry.emit")
    }

    pub(super) fn dispatch_builtin<TelemetryContext>(
        telemetry_context: &TelemetryContext,
        target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        TelemetryContext: NativeTelemetryCapability,
    {
        // `root.telemetry.emit` (as emitted for `std.log.*`) is the
        // RuntimeProgram builtin spelling of the registered
        // `std.telemetry.emit` native binding. Route it through the same
        // dispatch using the registry-shaped plan.
        let invocation = RuntimeNativeInvocation::new(
            target.to_string(),
            "std.telemetry.emit",
            Some(telemetry_emit_native_plan()),
            None,
            None,
        );
        Self::dispatch(telemetry_context, &invocation, target, args, heap)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch<TelemetryContext>(
        telemetry_context: &TelemetryContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        TelemetryContext: NativeTelemetryCapability,
    {
        let binding_key = invocation.binding_key();
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
        let value = telemetry_context.emit_native(binding_key, &wire_args)?;
        native_boundary.from_wire_return(&value, &format!("{diagnostic_target} response"), heap)
    }
}

fn telemetry_emit_native_plan() -> NativeCallPlan {
    let string =
        RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new());
    NativeCallPlan::new(
        NativeBindingKey::from_static("std.telemetry.emit"),
        vec![
            string.clone(),
            string,
            RuntimeTypePlan::synthetic_nullable(RuntimeTypePlan::json_object_plan()),
        ],
        RuntimeTypePlan::synthetic_named_builtin("void", RuntimeTypeNode::Null, Vec::new()),
        NativeRequiredContext::Telemetry,
    )
}

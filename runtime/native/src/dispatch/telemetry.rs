use super::RuntimeNativeInvocation;
use crate::capability::NativeTelemetryCapability;
use crate::error::Result;
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};

pub(super) struct TelemetryNativeDispatch;

impl TelemetryNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        target == "std.telemetry.emit"
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

use serde_json::Value;

use super::RuntimeNativeInvocation;
use crate::call_helpers::runtime_string_arg;
use crate::capability::NativeWebsocketCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{bytes_payload, RequestHeap, RuntimeValue};

pub(super) struct WebsocketNativeDispatch;

impl WebsocketNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        matches!(
            target,
            "std.websocket.sendTextToBusinessIdentity"
                | "std.websocket.sendBinaryToBusinessIdentity"
                | "std.websocket.sendTextToConnection"
                | "std.websocket.sendBinaryToConnection"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch<WebsocketContext>(
        websocket_context: &WebsocketContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        WebsocketContext: NativeWebsocketCapability,
    {
        let binding_key = invocation.binding_key();
        let connection_target = Self::string_arg(diagnostic_target, &args, invocation, 0, heap)?;
        match binding_key {
            "std.websocket.sendTextToBusinessIdentity" => {
                let text = Self::string_arg(
                    &format!("{diagnostic_target} text"),
                    &args,
                    invocation,
                    1,
                    heap,
                )?;
                websocket_context
                    .send_connection_text_to_business_identity(connection_target, text)?;
            }
            "std.websocket.sendBinaryToBusinessIdentity" => {
                let bytes = Self::bytes_arg(
                    &format!("{diagnostic_target} value"),
                    &args,
                    invocation,
                    1,
                    heap,
                )?;
                websocket_context
                    .send_connection_binary_to_business_identity(connection_target, bytes)?;
            }
            "std.websocket.sendTextToConnection" => {
                let text = Self::string_arg(
                    &format!("{diagnostic_target} text"),
                    &args,
                    invocation,
                    1,
                    heap,
                )?;
                websocket_context.send_connection_text_to_connection(connection_target, text)?;
            }
            "std.websocket.sendBinaryToConnection" => {
                let bytes = Self::bytes_arg(
                    &format!("{diagnostic_target} value"),
                    &args,
                    invocation,
                    1,
                    heap,
                )?;
                websocket_context.send_connection_binary_to_connection(connection_target, bytes)?;
            }
            _ => unreachable!("websocket native target checked by caller"),
        }
        invocation.native_boundary()?.from_wire_return(
            &Value::Null,
            &format!("{diagnostic_target} response"),
            heap,
        )
    }

    fn string_arg(
        label: &str,
        args: &[RuntimeValue],
        invocation: &RuntimeNativeInvocation,
        index: usize,
        heap: &mut RequestHeap,
    ) -> Result<String> {
        let arg = args
            .get(index)
            .ok_or_else(|| RuntimeError::Decode(format!("{label} requires argument")))?;
        let coerced = invocation
            .native_boundary()?
            .coerce_arg(index, arg, label, heap)?;
        runtime_string_arg(&coerced, label).map(str::to_string)
    }

    fn bytes_arg(
        label: &str,
        args: &[RuntimeValue],
        invocation: &RuntimeNativeInvocation,
        index: usize,
        heap: &mut RequestHeap,
    ) -> Result<Vec<u8>> {
        let arg = args
            .get(index)
            .ok_or_else(|| RuntimeError::Decode(format!("{label} requires argument")))?;
        let payload = invocation
            .native_boundary()?
            .to_wire_arg(index, arg, label, heap)?;
        bytes_payload(&payload)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| RuntimeError::Decode(format!("{label} must be bytes")))
    }
}

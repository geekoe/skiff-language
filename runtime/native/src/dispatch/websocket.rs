use serde_json::Value;
use skiff_runtime_capability_context::ConnectionRequestTerminal;
use skiff_runtime_model::service_error::{
    NamedUnionOwnerIdentity, WebSocketRequestError, WebSocketRequestErrorKind,
};

use super::{
    prepared::run_prepared_native_call, PreparedExternalNativeOperation, PreparedNativeCall,
    RuntimeNativeInvocation,
};
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
                | "std.websocket.requestJsonToConnection"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare<'a, WebsocketContext>(
        websocket_context: WebsocketContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'a>>
    where
        WebsocketContext: NativeWebsocketCapability + Send + 'a,
    {
        let binding_key = invocation.binding_key().to_string();
        let connection_target = Self::string_arg(&diagnostic_target, &args, &invocation, 0, heap)?;
        if binding_key == "std.websocket.requestJsonToConnection" {
            return Self::prepare_request(
                websocket_context,
                invocation,
                diagnostic_target,
                connection_target,
                &args,
                heap,
            );
        }
        match binding_key.as_str() {
            "std.websocket.sendTextToBusinessIdentity" => {
                let text = Self::string_arg(
                    &format!("{diagnostic_target} text"),
                    &args,
                    &invocation,
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
                    &invocation,
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
                    &invocation,
                    1,
                    heap,
                )?;
                websocket_context.send_connection_text_to_connection(connection_target, text)?;
            }
            "std.websocket.sendBinaryToConnection" => {
                let bytes = Self::bytes_arg(
                    &format!("{diagnostic_target} value"),
                    &args,
                    &invocation,
                    1,
                    heap,
                )?;
                websocket_context.send_connection_binary_to_connection(connection_target, bytes)?;
            }
            _ => unreachable!("websocket native target checked by caller"),
        }
        let value = invocation.native_boundary()?.decode_wire_return(
            &Value::Null,
            &format!("{diagnostic_target} response"),
            heap,
        )?;
        Ok(PreparedNativeCall::Ready(value))
    }

    fn prepare_request<'a, WebsocketContext>(
        websocket_context: WebsocketContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        connection_id: String,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'a>>
    where
        WebsocketContext: NativeWebsocketCapability + Send + 'a,
    {
        // The ordinary error owner is an artifact fact, not a capability fact.
        // Validate it before local encoding or any Host/peer side effect.
        let error_owner = invocation.named_union_error_owner()?.clone();
        let method = Self::string_arg(
            &format!("{diagnostic_target} method"),
            args,
            &invocation,
            1,
            heap,
        )?;
        let value = args.get(2).ok_or_else(|| {
            RuntimeError::decode_target(
                "std.json.encode",
                format!("{diagnostic_target} requires value argument"),
            )
        })?;
        let params = invocation
            .native_boundary()?
            .to_wire_arg(2, value, &format!("{diagnostic_target} value"), heap)
            .map_err(|error| RuntimeError::decode_target("std.json.encode", error.to_string()))?;
        if !params.is_object() && !params.is_array() {
            return Err(RuntimeError::decode_target(
                "std.json.encode",
                "WebSocket request params must encode to a JSON object or array",
            ));
        }
        let payload = serde_json::to_vec(&params)
            .map_err(|error| RuntimeError::decode_target("std.json.encode", error.to_string()))?;
        Ok(PreparedNativeCall::ExternalWait(
            PreparedExternalNativeOperation::new(
                async move {
                    let terminal = websocket_context
                        .request_json_to_connection(connection_id, method, payload)
                        .await?;
                    websocket_terminal_value(&error_owner, terminal)
                },
                move |value, heap| {
                    invocation
                        .native_boundary()?
                        .decode_wire_return(&value, &format!("{diagnostic_target} response"), heap)
                        .map_err(|error| {
                            RuntimeError::decode_target("std.json.decode", error.to_string())
                        })
                },
            ),
        ))
    }

    #[allow(dead_code)]
    pub(super) async fn dispatch<WebsocketContext>(
        websocket_context: WebsocketContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        WebsocketContext: NativeWebsocketCapability + Send,
    {
        let prepared = Self::prepare(websocket_context, invocation, diagnostic_target, args, heap)?;
        run_prepared_native_call(prepared, heap).await
    }

    fn request_error(
        invocation: &RuntimeNativeInvocation,
        kind: WebSocketRequestErrorKind,
        message: &str,
        code: Option<i64>,
        data: Option<Value>,
    ) -> Result<RuntimeValue> {
        Self::request_error_for_owner(
            invocation.named_union_error_owner()?.clone(),
            kind,
            message,
            code,
            data,
        )
    }

    fn request_error_for_owner<T>(
        owner: NamedUnionOwnerIdentity,
        kind: WebSocketRequestErrorKind,
        message: &str,
        code: Option<i64>,
        data: Option<Value>,
    ) -> Result<T> {
        let error = WebSocketRequestError::new(owner, kind, message, code, data)
            .map_err(RuntimeError::InvalidArtifact)?;
        Err(RuntimeError::Opaque(Box::new(error)))
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

fn websocket_terminal_value(
    error_owner: &NamedUnionOwnerIdentity,
    terminal: ConnectionRequestTerminal,
) -> Result<Value> {
    match terminal {
        ConnectionRequestTerminal::Success(payload) => serde_json::from_slice::<Value>(&payload)
            .map_err(|error| RuntimeError::decode_target("std.json.decode", error.to_string())),
        ConnectionRequestTerminal::DeadlineExceeded => Err(RuntimeError::ExecutionBudgetExceeded {
            reason: crate::error::BudgetReason::DeadlineExceeded,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        }),
        ConnectionRequestTerminal::AncestorCancelled => Err(RuntimeError::Cancelled),
        ConnectionRequestTerminal::ConnectionUnavailable => {
            WebsocketNativeDispatch::request_error_for_owner(
                error_owner.clone(),
                WebSocketRequestErrorKind::ConnectionUnavailable,
                "WebSocket connection is unavailable",
                None,
                None,
            )
        }
        ConnectionRequestTerminal::TransportUnavailable => {
            WebsocketNativeDispatch::request_error_for_owner(
                error_owner.clone(),
                WebSocketRequestErrorKind::TransportUnavailable,
                "WebSocket transport is unavailable",
                None,
                None,
            )
        }
        ConnectionRequestTerminal::ProtocolError => {
            WebsocketNativeDispatch::request_error_for_owner(
                error_owner.clone(),
                WebSocketRequestErrorKind::ProtocolError,
                "WebSocket request protocol error",
                None,
                None,
            )
        }
        ConnectionRequestTerminal::ResourceLimit => {
            WebsocketNativeDispatch::request_error_for_owner(
                error_owner.clone(),
                WebSocketRequestErrorKind::ResourceLimit,
                "WebSocket request resource limit exceeded",
                None,
                None,
            )
        }
        ConnectionRequestTerminal::Remote {
            code,
            message,
            data,
        } => {
            let data = data
                .map(|payload| {
                    serde_json::from_slice(&payload).map_err(|_| {
                        RuntimeError::InvalidArtifact(
                            "strict remote WebSocket error data became invalid JSON".to_string(),
                        )
                    })
                })
                .transpose()?;
            WebsocketNativeDispatch::request_error_for_owner(
                error_owner.clone(),
                WebSocketRequestErrorKind::Remote,
                &message,
                Some(code),
                data,
            )
        }
    }
}

#[cfg(test)]
mod tests;

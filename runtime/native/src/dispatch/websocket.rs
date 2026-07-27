use serde_json::Value;
use skiff_runtime_capability_context::ConnectionRequestTerminal;
use skiff_runtime_model::service_error::{WebSocketRequestError, WebSocketRequestErrorKind};

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
                | "std.websocket.requestJsonToConnection"
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn dispatch<WebsocketContext>(
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
        if binding_key == "std.websocket.requestJsonToConnection" {
            return Self::dispatch_request(
                websocket_context,
                invocation,
                diagnostic_target,
                connection_target,
                &args,
                heap,
            )
            .await;
        }
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

    async fn dispatch_request<WebsocketContext>(
        websocket_context: &WebsocketContext,
        invocation: &RuntimeNativeInvocation,
        diagnostic_target: &str,
        connection_id: String,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        WebsocketContext: NativeWebsocketCapability,
    {
        let method = Self::string_arg(
            &format!("{diagnostic_target} method"),
            args,
            invocation,
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
        let terminal = websocket_context
            .request_json_to_connection(connection_id, method, payload)
            .await?;
        match terminal {
            ConnectionRequestTerminal::Success(payload) => {
                let value = serde_json::from_slice::<Value>(&payload).map_err(|error| {
                    RuntimeError::decode_target("std.json.decode", error.to_string())
                })?;
                invocation
                    .native_boundary()?
                    .from_wire_return(&value, &format!("{diagnostic_target} response"), heap)
                    .map_err(|error| {
                        RuntimeError::decode_target("std.json.decode", error.to_string())
                    })
            }
            ConnectionRequestTerminal::DeadlineExceeded => {
                Err(RuntimeError::ExecutionBudgetExceeded {
                    reason: crate::error::BudgetReason::DeadlineExceeded,
                    instruction_count: 0,
                    limit: None,
                    elapsed_ms: 0.0,
                })
            }
            ConnectionRequestTerminal::AncestorCancelled => Err(RuntimeError::Cancelled),
            ConnectionRequestTerminal::ConnectionUnavailable => Self::request_error(
                websocket_context,
                WebSocketRequestErrorKind::ConnectionUnavailable,
                "WebSocket connection is unavailable",
                None,
                None,
            ),
            ConnectionRequestTerminal::TransportUnavailable => Self::request_error(
                websocket_context,
                WebSocketRequestErrorKind::TransportUnavailable,
                "WebSocket transport is unavailable",
                None,
                None,
            ),
            ConnectionRequestTerminal::ProtocolError => Self::request_error(
                websocket_context,
                WebSocketRequestErrorKind::ProtocolError,
                "WebSocket request protocol error",
                None,
                None,
            ),
            ConnectionRequestTerminal::ResourceLimit => Self::request_error(
                websocket_context,
                WebSocketRequestErrorKind::ResourceLimit,
                "WebSocket request resource limit exceeded",
                None,
                None,
            ),
            ConnectionRequestTerminal::Remote {
                code,
                message,
                data,
            } => {
                let data = data
                    .map(|payload| {
                        serde_json::from_slice(&payload).map_err(|_| {
                            RuntimeError::InvalidArtifact(
                                "strict remote WebSocket error data became invalid JSON"
                                    .to_string(),
                            )
                        })
                    })
                    .transpose()?;
                Self::request_error(
                    websocket_context,
                    WebSocketRequestErrorKind::Remote,
                    &message,
                    Some(code),
                    data,
                )
            }
        }
    }

    fn request_error<WebsocketContext>(
        websocket_context: &WebsocketContext,
        kind: WebSocketRequestErrorKind,
        message: &str,
        code: Option<i64>,
        data: Option<Value>,
    ) -> Result<RuntimeValue>
    where
        WebsocketContext: NativeWebsocketCapability,
    {
        let owner = websocket_context
            .websocket_request_error_owner()
            .ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "std.websocket.requestJsonToConnection requires the linked \
                     std.websocket.WebSocketRequestError named-union owner"
                        .to_string(),
                )
            })?;
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

#[cfg(test)]
mod tests {
    use skiff_runtime_model::{
        addr::{FileAddr, TypeAddr, UnitAddr},
        service_error::{
            CatchIdentity, LocalExecutionTypeIdentity, NamedUnionBranchIdentity,
            NamedUnionOwnerIdentity, WebSocketRequestErrorKind,
        },
    };

    use super::*;

    struct TestWebsocketContext {
        owner: Option<NamedUnionOwnerIdentity>,
    }

    impl NativeWebsocketCapability for TestWebsocketContext {
        fn websocket_request_error_owner(&self) -> Option<NamedUnionOwnerIdentity> {
            self.owner.clone()
        }

        fn send_connection_text_to_business_identity(&self, _: String, _: String) -> Result<()> {
            unreachable!()
        }

        fn send_connection_binary_to_business_identity(&self, _: String, _: Vec<u8>) -> Result<()> {
            unreachable!()
        }

        fn send_connection_text_to_connection(&self, _: String, _: String) -> Result<()> {
            unreachable!()
        }

        fn send_connection_binary_to_connection(&self, _: String, _: Vec<u8>) -> Result<()> {
            unreachable!()
        }
    }

    fn local_owner() -> NamedUnionOwnerIdentity {
        NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 7,
            },
            type_arguments: Vec::new(),
        })
    }

    #[test]
    fn websocket_request_error_materialization_requires_linked_union_owner() {
        let error = WebsocketNativeDispatch::request_error(
            &TestWebsocketContext { owner: None },
            WebSocketRequestErrorKind::ProtocolError,
            "protocol",
            None,
            None,
        )
        .expect_err("missing exact owner must fail closed");

        assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
        assert!(error.to_string().contains("named-union owner"));
    }

    #[test]
    fn websocket_request_error_materialization_keeps_all_five_exact_branches() {
        let owner = local_owner();
        let context = TestWebsocketContext {
            owner: Some(owner.clone()),
        };
        for kind in WebSocketRequestErrorKind::ALL {
            let remote = kind == WebSocketRequestErrorKind::Remote;
            let error = WebsocketNativeDispatch::request_error(
                &context,
                kind,
                "sanitized",
                remote.then_some(-32603),
                remote.then(|| serde_json::json!({"peer": true})),
            )
            .expect_err("request branch is an ordinary error");
            let RuntimeError::Opaque(error) = error else {
                panic!("expected exact opaque error");
            };
            assert_eq!(
                error.catch_projection().unwrap().0,
                CatchIdentity::NamedUnionBranch {
                    union: owner.clone(),
                    branch: NamedUnionBranchIdentity::SyntheticDiscriminator {
                        discriminator_field: "kind".to_string(),
                        discriminator_value: kind.discriminator().to_string(),
                    },
                }
            );
        }
    }
}

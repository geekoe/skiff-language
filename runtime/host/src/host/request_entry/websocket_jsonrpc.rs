use std::sync::Arc;

use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, BytecodeRequestExecutionInput, RequestEnvelope,
    RequestExecutionOwnerInventorySnapshot, ResponseEnd, ResponseError, ResponseEvent,
    RouterWriterMessage,
};
use skiff_runtime_transport::{
    protocol::{
        BytecodeWebSocketJsonRpcRequestStartFrameHeader,
        BytecodeWebSocketJsonRpcResponseFrameHeader, BytecodeWebSocketJsonRpcResponseOutcome,
    },
    response_mapper::bytecode_websocket_jsonrpc_response_into_frame,
};
use tokio::sync::mpsc;
use tracing::error;

use super::{
    assembly::bytecode_request_execution_handles,
    assembly_wire::{
        production_bytecode_request_child_composition, AdmittedBytecodeWebSocketJsonRpcRequest,
    },
};
use crate::{
    host::{
        request_supervisor::{
            ActivationOutcome, CleanupPermit, CompletionTrace, RequestReservation,
            SupervisedRequest,
        },
        RuntimeHost,
    },
    loader::bytecode_admission::BytecodeRoute,
    telemetry::RequestTelemetryContext,
};

#[derive(Debug)]
#[allow(dead_code)]
enum WebSocketJsonRpcOutcome {
    Success { payload: Vec<u8> },
    InvalidParams,
    InternalError,
    DeadlineExceeded,
}

#[derive(Debug)]
enum WebSocketJsonRpcTerminal {
    Response(WebSocketJsonRpcOutcome),
    Cancelled,
}

impl RuntimeHost {
    pub(super) async fn task_bytecode_websocket_jsonrpc_request(
        &self,
        reservation: RequestReservation,
        request: AdmittedBytecodeWebSocketJsonRpcRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeWebSocketJsonRpcRequest {
            route,
            header,
            target,
            params,
            db_source,
        } = request;
        let request_envelope = bytecode_websocket_jsonrpc_request_envelope(&route, &header, params);
        let telemetry = bytecode_websocket_jsonrpc_telemetry_context(self, &header, &route);
        let observer = reservation.observer().clone();
        let activation_key = reservation.key().clone();
        let supervised_request =
            match reservation.activate(&activation_key, &request_envelope, telemetry) {
                ActivationOutcome::Activated(request) => request,
                ActivationOutcome::RevokedByCancel | ActivationOutcome::RevokedBySessionStop => {
                    return
                }
                ActivationOutcome::Invalid => {
                    self.send_http_gateway_admission_error(
                        &header.request_id,
                        "bytecode request reservation activation failed",
                        &sender,
                    );
                    return;
                }
            };
        route.publish_admission_observations();
        let cancellation = supervised_request.cancellation_token();
        let http_client =
            Some(self.bytecode_http_client_port(cancellation.clone(), http_response_max_bytes));
        let execution_budget = supervised_request.execution_budget();
        let handles = bytecode_request_execution_handles(self, http_response_max_bytes);
        let request_id = header.request_id.clone();
        let host = self.clone();
        let child_composition = production_bytecode_request_child_composition(
            self,
            target.image().as_ref(),
            db_source.as_ref(),
            &request_envelope.request_id,
        );
        tokio::spawn(async move {
            let request_runner::DrivenBytecodeRequest {
                result,
                retention,
                owner_inventory,
            } = request_runner::drive_runtime_bytecode_request_async(
                BytecodeRequestExecutionInput {
                    target,
                    request: request_envelope,
                    observer: observer.clone(),
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                    http_client,
                    server_stream_writer: None,
                    child_composition,
                    heap: None,
                },
            )
            .await;
            let owner_inventory = owner_inventory.into_snapshot();
            let terminal = match result {
                Ok(BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))) => {
                    WebSocketJsonRpcTerminal::Response(WebSocketJsonRpcOutcome::Success { payload })
                }
                Ok(_) => WebSocketJsonRpcTerminal::Response(WebSocketJsonRpcOutcome::InternalError),
                Err(error) if error.is_cancellation_terminal() => {
                    WebSocketJsonRpcTerminal::Cancelled
                }
                Err(_) => {
                    WebSocketJsonRpcTerminal::Response(WebSocketJsonRpcOutcome::InternalError)
                }
            };
            let cleanup_permit = host
                .finish_websocket_jsonrpc_request(
                    &supervised_request,
                    request_id,
                    owner_inventory,
                    terminal,
                    &sender,
                )
                .await;
            drop(retention);
            drop(execution_budget);
            drop(supervised_request);
            drop(route);
            if let Some(permit) = cleanup_permit {
                host.observe_bytecode_request_cleanup(permit);
            }
        });
    }

    async fn finish_websocket_jsonrpc_request(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        terminal: WebSocketJsonRpcTerminal,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Option<CleanupPermit> {
        let WebSocketJsonRpcTerminal::Response(outcome) = terminal else {
            return self
                .request_supervisor
                .complete_cancelled(
                    supervised_request,
                    owner_inventory,
                    CompletionTrace::RUNTIME,
                )
                .await;
        };
        let permit = match &outcome {
            WebSocketJsonRpcOutcome::Success { .. } => {
                self.request_supervisor
                    .complete_success(
                        supervised_request,
                        owner_inventory,
                        CompletionTrace::RUNTIME,
                    )
                    .await
            }
            failed => {
                self.request_supervisor
                    .complete_error(
                        supervised_request,
                        "request.error",
                        &websocket_jsonrpc_response_error(failed),
                        owner_inventory,
                        CompletionTrace::RUNTIME,
                    )
                    .await
            }
        };
        if !permit.as_ref().is_some_and(CleanupPermit::response_owned) {
            return permit;
        }
        if let Some(error) = permit.as_ref().and_then(CleanupPermit::response_override) {
            let override_outcome = if error.code == "TimeoutError" {
                WebSocketJsonRpcOutcome::DeadlineExceeded
            } else {
                WebSocketJsonRpcOutcome::InternalError
            };
            let (outcome, payload) = websocket_jsonrpc_response_parts(override_outcome);
            if let Ok(frame) = bytecode_websocket_jsonrpc_response_into_frame(
                request_id,
                BytecodeWebSocketJsonRpcResponseFrameHeader { outcome },
                payload,
            ) {
                let _ = sender.send(RouterWriterMessage::Binary(frame));
            }
            return permit;
        }
        let (outcome, payload) = websocket_jsonrpc_response_parts(outcome);
        match bytecode_websocket_jsonrpc_response_into_frame(
            request_id,
            BytecodeWebSocketJsonRpcResponseFrameHeader { outcome },
            payload,
        ) {
            Ok(frame) => {
                let _ = sender.send(RouterWriterMessage::Binary(frame));
            }
            Err(error) => {
                error!(event = "runtime.response_encode_error", error = %error);
            }
        }
        permit
    }
}

fn websocket_jsonrpc_response_error(outcome: &WebSocketJsonRpcOutcome) -> ResponseError {
    let (code, message) = match outcome {
        WebSocketJsonRpcOutcome::Success { .. } => {
            unreachable!("success JSON-RPC outcome is not an error")
        }
        WebSocketJsonRpcOutcome::InvalidParams => ("InvalidParams", "invalid JSON-RPC params"),
        WebSocketJsonRpcOutcome::InternalError => ("InternalError", "JSON-RPC execution failed"),
        WebSocketJsonRpcOutcome::DeadlineExceeded => {
            ("DeadlineExceeded", "JSON-RPC deadline exceeded")
        }
    };
    ResponseError {
        code: code.to_string(),
        message: message.to_string(),
        status: None,
        details: None,
    }
}

fn bytecode_websocket_jsonrpc_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeWebSocketJsonRpcRequestStartFrameHeader,
    params: Vec<u8>,
) -> RequestEnvelope {
    RequestEnvelope {
        request_id: header.request_id.clone(),
        mode: header.mode.clone(),
        target: route.target_label(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(route.deployment().service_id.clone()),
        build_id: route.build_id().to_string(),
        service_protocol_identity: route.service_protocol_identity().to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: Some(header.routing.ingress.method.clone()),
            path: header.routing.ingress.path.clone(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: params,
        extra: bytecode_websocket_jsonrpc_extra(header),
    }
}

fn bytecode_websocket_jsonrpc_extra(
    header: &BytecodeWebSocketJsonRpcRequestStartFrameHeader,
) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline)
                .expect("typed bytecode WebSocket JSON-RPC deadline remains serializable"),
        );
    }
    extra
}

fn bytecode_websocket_jsonrpc_telemetry_context(
    host: &RuntimeHost,
    header: &BytecodeWebSocketJsonRpcRequestStartFrameHeader,
    route: &BytecodeRoute,
) -> RequestTelemetryContext {
    let mut context = RequestTelemetryContext::new(host.telemetry.clone());
    context.service_id = Some(route.deployment().service_id.clone());
    context.build_id = Some(route.build_id().to_string());
    context.runtime_id = Some(host.base_runtime_id.clone());
    context.request_id = Some(header.request_id.clone());
    context.target = Some(route.target_label());
    context.trace_id = Some(header.trace.trace_id.clone());
    context.span_id = Some(header.trace.span_id.clone());
    context.parent_span_id = header.trace.parent_span_id.clone();
    context
}

fn websocket_jsonrpc_response_parts(
    outcome: WebSocketJsonRpcOutcome,
) -> (BytecodeWebSocketJsonRpcResponseOutcome, Vec<u8>) {
    match outcome {
        WebSocketJsonRpcOutcome::Success { payload } => {
            (BytecodeWebSocketJsonRpcResponseOutcome::Success, payload)
        }
        WebSocketJsonRpcOutcome::InvalidParams => (
            BytecodeWebSocketJsonRpcResponseOutcome::InvalidParams,
            Vec::new(),
        ),
        WebSocketJsonRpcOutcome::InternalError => (
            BytecodeWebSocketJsonRpcResponseOutcome::InternalError,
            Vec::new(),
        ),
        WebSocketJsonRpcOutcome::DeadlineExceeded => (
            BytecodeWebSocketJsonRpcResponseOutcome::DeadlineExceeded,
            Vec::new(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_jsonrpc_outcomes_preserve_success_null_and_omit_failure_payloads() {
        let (outcome, payload) =
            websocket_jsonrpc_response_parts(WebSocketJsonRpcOutcome::Success {
                payload: b"null".to_vec(),
            });
        assert_eq!(outcome, BytecodeWebSocketJsonRpcResponseOutcome::Success);
        assert_eq!(payload, b"null");

        for terminal in [
            WebSocketJsonRpcOutcome::InvalidParams,
            WebSocketJsonRpcOutcome::InternalError,
            WebSocketJsonRpcOutcome::DeadlineExceeded,
        ] {
            let (_, payload) = websocket_jsonrpc_response_parts(terminal);
            assert!(payload.is_empty());
        }
    }
}

use std::sync::Arc;

use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{
    BoundaryResponse, BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput,
    RequestEnvelope, ResponseEnd, ResponseEvent, RouterWriterMessage,
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
    assembly_wire::AdmittedBytecodeWebSocketJsonRpcRequest,
    resumable::{drive_bytecode_request, RejectingResponseEventSink},
};
use crate::{
    host::{
        request_supervisor::{CompletionTrace, SupervisedRequest},
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
        _router_session_id: String,
        request: AdmittedBytecodeWebSocketJsonRpcRequest,
        _http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeWebSocketJsonRpcRequest {
            route,
            header,
            target,
            params,
        } = request;
        let request_envelope = bytecode_websocket_jsonrpc_request_envelope(&route, &header, params);
        let telemetry = bytecode_websocket_jsonrpc_telemetry_context(self, &header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&request_envelope, telemetry)
            .await;
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let request_id = header.request_id.clone();
        let host = self.clone();
        tokio::spawn(async move {
            let result = drive_bytecode_request(
                BytecodeRequestExecutionInput {
                    target,
                    request: request_envelope,
                    cancelled: supervised_request.cancelled(),
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                },
                Arc::new(RejectingResponseEventSink),
            )
            .await;
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
            host.finish_websocket_jsonrpc_request(
                &supervised_request,
                request_id,
                terminal,
                &sender,
            )
            .await;
            drop(route);
        });
    }

    async fn finish_websocket_jsonrpc_request(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        terminal: WebSocketJsonRpcTerminal,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let WebSocketJsonRpcTerminal::Response(outcome) = terminal else {
            self.request_supervisor
                .complete_cancelled(supervised_request, CompletionTrace::RUNTIME)
                .await;
            return;
        };
        if !self
            .request_supervisor
            .complete_success(supervised_request, CompletionTrace::RUNTIME)
            .await
        {
            return;
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
        target: route.operation_id().as_str().to_string(),
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
    context.target = Some(route.operation_id().as_str().to_string());
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

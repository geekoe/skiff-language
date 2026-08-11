use std::sync::Arc;

use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_eval::{
    RuntimeWebSocketJsonRpcExecutionOutcome, RuntimeWebSocketJsonRpcExecutionTerminal,
};
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, BytecodeRequestExecutionHandles,
    BytecodeRequestExecutionInput, RequestEnvelope, ResponseEnd, ResponseEvent,
    RouterWriterMessage,
};
use skiff_runtime_transport::{
    response_mapper::runtime_assembly_websocket_jsonrpc_response_into_frame,
    runtime_assembly_request::{
        RuntimeAssemblyWebSocketJsonRpcResponseFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
    },
};
use tokio::sync::mpsc;
use tracing::error;

use super::assembly_wire::AdmittedBytecodeWebSocketJsonRpcRequest;
#[cfg(test)]
use super::assembly_wire::AdmittedWebSocketJsonRpcRequest;
#[cfg(test)]
use crate::loader::assembly_admission::ActiveAssemblyRoute;
use crate::{
    error::{Result, RuntimeError},
    host::{
        request_supervisor::{CompletionTrace, SupervisedRequest},
        RuntimeHost,
    },
    loader::bytecode_admission::BytecodeRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    #[cfg(test)]
    pub(super) async fn task_websocket_jsonrpc_on_resolved_route(
        &self,
        router_session_id: String,
        request: AdmittedWebSocketJsonRpcRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedWebSocketJsonRpcRequest {
            resolved,
            header,
            params,
        } = request;
        let route = resolved.method_route;
        let target = resolved.target;
        let request_id = header.request_id.clone();
        let handles = match self.websocket_jsonrpc_execution_handles(
            &route,
            &header,
            http_response_max_bytes,
            &sender,
            &router_session_id,
        ) {
            Ok(handles) => handles,
            Err(error) => {
                self.send_http_gateway_admission_error(&request_id, error, &sender);
                return;
            }
        };
        let telemetry = self.websocket_jsonrpc_telemetry_context(&header, &route);
        let supervisor_request = websocket_jsonrpc_supervisor_request(&header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&supervisor_request, telemetry)
            .await;
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let connection_id = header.websocket_json_rpc.connection_id.clone();
        let business_identity = header.websocket_json_rpc.business_identity.clone();
        let test_effects_enabled = header.test_effects_enabled;
        let host = self.clone();
        tokio::spawn(async move {
            let terminal = request_runner::execute_runtime_websocket_jsonrpc(
                request_runner::RuntimeWebSocketJsonRpcExecutionInput {
                    target,
                    params,
                    connection_id,
                    business_identity,
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    test_effects_enabled,
                    handles,
                },
            )
            .await;
            host.finish_websocket_jsonrpc_request(
                &supervised_request,
                request_id,
                terminal,
                &sender,
            )
            .await;
            // The old method route owns every capability fact through execution and terminal
            // settlement. Active replacement cannot substitute current assembly context.
            drop(route);
        });
    }

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
            let result =
                request_runner::execute_runtime_bytecode_request(BytecodeRequestExecutionInput {
                    target,
                    request: request_envelope,
                    cancelled: supervised_request.cancelled(),
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                });
            let terminal = match result {
                Ok(BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Payload(payload)))) => {
                    RuntimeWebSocketJsonRpcExecutionTerminal::Response(
                        RuntimeWebSocketJsonRpcExecutionOutcome::Success { payload },
                    )
                }
                Ok(_) => RuntimeWebSocketJsonRpcExecutionTerminal::Response(
                    RuntimeWebSocketJsonRpcExecutionOutcome::InternalError,
                ),
                Err(error) if error.is_cancellation_terminal() => {
                    RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
                }
                Err(_) => RuntimeWebSocketJsonRpcExecutionTerminal::Response(
                    RuntimeWebSocketJsonRpcExecutionOutcome::InternalError,
                ),
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

    #[cfg(test)]
    fn websocket_jsonrpc_execution_handles(
        &self,
        route: &ActiveAssemblyRoute,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        http_response_max_bytes: usize,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        router_session_id: &str,
    ) -> Result<request_runner::RuntimeWebSocketJsonRpcExecutionHandles> {
        let telemetry = self.websocket_jsonrpc_telemetry_context(header, route);
        let eval_adapter = crate::eval_capability_adapter::websocket_jsonrpc_eval_adapter(
            crate::eval_capability_adapter::RuntimeWebSocketJsonRpcEvalAdapterInput {
                context: self.runtime_assembly_eval_adapter_context(
                    route,
                    telemetry,
                    sender,
                    router_session_id,
                    http_response_max_bytes,
                )?,
                header: header.clone(),
            },
        )
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        Ok(request_runner::RuntimeWebSocketJsonRpcExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
            eval_adapter,
        })
    }

    async fn finish_websocket_jsonrpc_request(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        terminal: RuntimeWebSocketJsonRpcExecutionTerminal,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let RuntimeWebSocketJsonRpcExecutionTerminal::Response(outcome) = terminal else {
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
        match runtime_assembly_websocket_jsonrpc_response_into_frame(
            request_id,
            RuntimeAssemblyWebSocketJsonRpcResponseFrameHeader { outcome },
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

    #[cfg(test)]
    fn websocket_jsonrpc_telemetry_context(
        &self,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        route: &ActiveAssemblyRoute,
    ) -> RequestTelemetryContext {
        let mut context = RequestTelemetryContext::new(self.telemetry.clone());
        context.service_id = Some(route.activation().identity().deployment.service_id.clone());
        context.build_id = Some(
            route
                .activation()
                .implementation_package_build_id()
                .as_str()
                .to_string(),
        );
        context.activation_identity = Some(route.activation().activation_id().as_str().to_string());
        context.runtime_id = Some(self.base_runtime_id.clone());
        context.request_id = Some(header.request_id.clone());
        context.target = Some(route.gateway_entry_key().as_str().to_string());
        context.trace_id = Some(header.trace.trace_id.clone());
        context.span_id = Some(header.trace.span_id.clone());
        context.parent_span_id = header.trace.parent_span_id.clone();
        context
    }
}

fn bytecode_websocket_jsonrpc_request_envelope(
    route: &BytecodeRoute,
    header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    params: Vec<u8>,
) -> RequestEnvelope {
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline)
                .expect("typed bytecode WebSocket JSON-RPC deadline remains serializable"),
        );
    }
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
        extra,
    }
}

fn bytecode_websocket_jsonrpc_telemetry_context(
    host: &RuntimeHost,
    header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
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

#[cfg(test)]
fn websocket_jsonrpc_supervisor_request(
    header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    route: &ActiveAssemblyRoute,
) -> RequestEnvelope {
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline)
                .expect("typed WebSocket JSON-RPC deadline remains serializable"),
        );
    }
    RequestEnvelope {
        request_id: header.request_id.clone(),
        mode: header.mode.clone(),
        target: route.gateway_entry_key().as_str().to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some(route.activation().identity().deployment.service_id.clone()),
        build_id: route
            .activation()
            .implementation_package_build_id()
            .as_str()
            .to_string(),
        service_protocol_identity: route.service_protocol_identity().as_str().to_string(),
        contract_identity: None,
        activation_identity: Some(route.activation().activation_id().as_str().to_string()),
        ingress_selector: Some(route.selector().clone()),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra,
    }
}

fn websocket_jsonrpc_response_parts(
    outcome: RuntimeWebSocketJsonRpcExecutionOutcome,
) -> (RuntimeAssemblyWebSocketJsonRpcResponseOutcome, Vec<u8>) {
    match outcome {
        RuntimeWebSocketJsonRpcExecutionOutcome::Success { payload } => (
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success,
            payload,
        ),
        RuntimeWebSocketJsonRpcExecutionOutcome::InvalidParams => (
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InvalidParams,
            Vec::new(),
        ),
        RuntimeWebSocketJsonRpcExecutionOutcome::InternalError => (
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InternalError,
            Vec::new(),
        ),
        RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded => (
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::DeadlineExceeded,
            Vec::new(),
        ),
    }
}

#[cfg(test)]
mod tests;

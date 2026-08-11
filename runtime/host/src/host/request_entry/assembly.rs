use std::sync::{Arc, Mutex};

use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, BytecodeRequestExecutionHandles,
    BytecodeRequestExecutionInput, RequestEnvelope, RequestError, ResponseEventSink,
    ResponseStreamEvent, RouterWriterMessage,
};
use skiff_runtime_transport::{
    response_mapper::OrdinaryResponseEvent,
    protocol::{
        BytecodeRequestStartFrameHeader, BytecodeTaskRequestStartFrameHeader,
        BytecodeWebSocketConnectRequestStartFrameHeader,
        BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
    },
};
use tokio::sync::mpsc;
use tracing::error;

use super::{
    assembly_wire::{
        AdmittedBytecodeHttpRequest, AdmittedBytecodeTaskRequest,
        AdmittedBytecodeWebSocketConnectRequest, AdmittedBytecodeWebSocketConnectionClosedRequest,
    },
    request_error_into_runtime_error, response_event_into_transport_message,
    response_into_transport_message,
};
use crate::{
    error::RuntimeError,
    host::{
        http_response_ceiling::HttpResponseCeiling,
        request_supervisor::{CompletionTrace, SupervisedRequest},
        RuntimeHost,
    },
    loader::bytecode_admission::BytecodeRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    pub(super) async fn task_bytecode_http_request(
        &self,
        _router_session_id: String,
        request: AdmittedBytecodeHttpRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeHttpRequest {
            route,
            header,
            body,
            target,
        } = request;
        let request_envelope = bytecode_http_request_envelope(&route, &header, body);
        let telemetry = bytecode_http_telemetry_context(self, &header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&request_envelope, telemetry)
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let response_sink = Arc::new(HostHttpGatewayResponseSink::new(
            sender.clone(),
            http_response_max_bytes,
        ));
        let request_id = header.request_id.clone();
        let host = self.clone();
        tokio::spawn(async move {
            let result =
                request_runner::execute_runtime_bytecode_request(BytecodeRequestExecutionInput {
                    target,
                    request: request_envelope,
                    cancelled,
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                });
            host.finish_http_gateway_request(
                &supervised_request,
                &request_id,
                result,
                http_response_max_bytes,
                &response_sink,
                &sender,
            )
            .await;
            drop(route);
        });
    }

    pub(super) async fn task_bytecode_task_request(
        &self,
        _router_session_id: String,
        request: AdmittedBytecodeTaskRequest,
        _http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeTaskRequest {
            route,
            header,
            target,
            payload,
        } = request;
        let request_envelope = bytecode_task_request_envelope(&route, &header, payload);
        let telemetry = bytecode_task_telemetry_context(self, &header, &route);
        let Some(supervised_request) = self.request_supervisor.begin_task(&header, telemetry).await
        else {
            self.send_http_gateway_admission_error(
                &header.request_id,
                "duplicate active task requestId",
                &sender,
            );
            return;
        };
        let cancelled = supervised_request.cancelled();
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
                    cancelled,
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                });
            match result {
                Ok(response) => {
                    if !host
                        .request_supervisor
                        .complete_success(&supervised_request, CompletionTrace::RUNTIME)
                        .await
                    {
                        return;
                    }
                    match response_into_transport_message(request_id, response) {
                        Ok(Some(message)) => {
                            let _ = sender.send(message);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            error!(event = "runtime.response_encode_error", error = %error)
                        }
                    }
                }
                Err(error) => {
                    host.finish_direct_task_error(&supervised_request, request_id, error, &sender)
                        .await;
                }
            }
            drop(route);
        });
    }

    pub(super) async fn task_bytecode_websocket_connect_request(
        &self,
        _router_session_id: String,
        request: AdmittedBytecodeWebSocketConnectRequest,
        _http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeWebSocketConnectRequest {
            route,
            header,
            target,
        } = request;
        let request_envelope = bytecode_websocket_connect_request_envelope(&route, &header);
        let telemetry = bytecode_websocket_connect_telemetry_context(self, &header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&request_envelope, telemetry)
            .await;
        let cancelled = supervised_request.cancelled();
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
                    cancelled,
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                });
            let mapped_error = match result {
                Ok(_) => RequestError::Unsupported(
                    "bytecode WebSocket connect response mapping is not supported; refusing legacy ActiveAssemblyRoute fallback"
                        .to_string(),
                ),
                Err(error) => error,
            };
            host.finish_websocket_connect_error(
                &supervised_request,
                request_id,
                mapped_error,
                &sender,
            )
            .await;
            drop(route);
        });
    }

    pub(super) async fn task_bytecode_websocket_connection_closed_request(
        &self,
        _router_session_id: String,
        request: AdmittedBytecodeWebSocketConnectionClosedRequest,
        _http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeWebSocketConnectionClosedRequest {
            route,
            header,
            target,
        } = request;
        let request_envelope =
            bytecode_websocket_connection_closed_request_envelope(&route, &header);
        let telemetry =
            bytecode_websocket_connection_closed_telemetry_context(self, &header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&request_envelope, telemetry)
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let host = self.clone();
        tokio::spawn(async move {
            let result =
                request_runner::execute_runtime_bytecode_request(BytecodeRequestExecutionInput {
                    target,
                    request: request_envelope,
                    cancelled,
                    cancellation,
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                });
            let error = match result {
                Ok(_) => RequestError::Unsupported(
                    "bytecode WebSocket connection close response mapping is not supported; refusing legacy ActiveAssemblyRoute fallback"
                        .to_string(),
                ),
                Err(error) => error,
            };
            host.finish_websocket_connection_closed_error(&supervised_request, error)
                .await;
            drop(route);
        });
    }

    async fn finish_http_gateway_request(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: &str,
        result: request_runner::RequestResult<BoundaryResponse>,
        http_response_max_bytes: usize,
        response_sink: &HostHttpGatewayResponseSink,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let message = match result {
            Ok(response) => {
                if let Err(response_error) =
                    super::super::http_response_ceiling::validate_unary_response(
                        &response,
                        http_response_max_bytes,
                        true,
                    )
                {
                    let response_event = OrdinaryResponseEvent::try_error(&response_error)
                        .expect("response ceiling failure is ordinary");
                    let ordinary_owner = self
                        .request_supervisor
                        .complete_error(
                            supervised_request,
                            "request.error",
                            response_event
                                .response_error()
                                .expect("ordinary error event carries response error"),
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if !ordinary_owner {
                        response_sink.cancel_without_response();
                        return;
                    }
                    return response_sink.send_terminal_response(request_id, response_event);
                }
                if !self
                    .request_supervisor
                    .complete_success(supervised_request, CompletionTrace::RUNTIME)
                    .await
                {
                    response_sink.cancel_without_response();
                    return;
                }
                if matches!(response, BoundaryResponse::StreamSent) {
                    response_sink.send_pending_stream_terminal();
                    return;
                }
                response_into_transport_message(request_id.to_string(), response)
            }
            Err(request_error) => {
                if request_error.is_cancellation_terminal() {
                    self.request_supervisor
                        .complete_cancelled(supervised_request, CompletionTrace::RUNTIME)
                        .await;
                    response_sink.cancel_without_response();
                    return;
                }
                if let Some(response_event) = response_sink.pending_ordinary_error() {
                    let ordinary_owner = self
                        .request_supervisor
                        .complete_error(
                            supervised_request,
                            "request.error",
                            response_event
                                .response_error()
                                .expect("pending ordinary event carries response error"),
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if !ordinary_owner {
                        response_sink.cancel_without_response();
                        return;
                    }
                    return response_sink.send_terminal_response(request_id, response_event);
                }
                if let Some(failure) = request_error.fixed_service_response_failure() {
                    error!(
                        event = "runtime.assembly_fixed_service_failure",
                        request_id,
                        trace_id = %failure.error().envelope().trace_id(),
                        error_id = %failure.error().envelope().error_id(),
                    );
                    let ordinary_owner = self
                        .request_supervisor
                        .complete_fixed_service_failure(
                            supervised_request,
                            "request.error",
                            failure.error(),
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if !ordinary_owner {
                        response_sink.cancel_without_response();
                        return;
                    }
                    return response_sink.send_terminal_response(
                        request_id,
                        OrdinaryResponseEvent::FixedServiceFailure(failure),
                    );
                }
                let response_event = OrdinaryResponseEvent::try_error(&request_error)
                    .expect("cancellation was split before ordinary response mapping");
                let response_error = request_error
                    .ordinary_response_error()
                    .expect("cancellation was split before ordinary response mapping");
                let runtime_error = request_error_into_runtime_error(request_error);
                error!(
                    event = "runtime.assembly_request_error",
                    request_id,
                    error = %runtime_error
                );
                let ordinary_owner = self
                    .request_supervisor
                    .complete_error(
                        supervised_request,
                        "request.error",
                        &response_error,
                        CompletionTrace::RUNTIME,
                    )
                    .await;
                if !ordinary_owner {
                    response_sink.cancel_without_response();
                    return;
                }
                return response_sink.send_terminal_response(request_id, response_event);
            }
        };
        match message {
            Ok(Some(message)) => {
                let _ = sender.send(message);
            }
            Ok(None) => {}
            Err(error) => error!(event = "runtime.response_encode_error", error = %error),
        }
    }

    pub(super) fn send_http_gateway_admission_error(
        &self,
        request_id: &str,
        error: impl std::fmt::Display,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let error = RuntimeError::Decode(error.to_string());
        let response_event =
            OrdinaryResponseEvent::try_error(&error).expect("admission failure is ordinary");
        match response_event_into_transport_message(request_id.to_string(), response_event) {
            Ok(message) => {
                let _ = sender.send(message);
            }
            Err(error) => error!(event = "runtime.response_encode_error", error = %error),
        }
    }

    async fn finish_websocket_connect_error(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        request_error: RequestError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        if request_error.is_cancellation_terminal() {
            self.request_supervisor
                .complete_cancelled(supervised_request, CompletionTrace::RUNTIME)
                .await;
            return;
        }
        let response_event = OrdinaryResponseEvent::try_error(&request_error)
            .expect("cancellation was split before ordinary response mapping");
        let response_error = request_error
            .ordinary_response_error()
            .expect("cancellation was split before ordinary response mapping");
        let ordinary_owner = self
            .request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                CompletionTrace::RUNTIME,
            )
            .await;
        if !ordinary_owner {
            return;
        }
        if let Ok(message) = response_event_into_transport_message(request_id, response_event) {
            let _ = sender.send(message);
        }
    }

    async fn finish_direct_task_error(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        request_error: RequestError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        if request_error.is_cancellation_terminal() {
            self.request_supervisor
                .complete_cancelled(supervised_request, CompletionTrace::RUNTIME)
                .await;
            return;
        }
        if let Some(failure) = request_error.fixed_service_response_failure() {
            error!(
                event = "runtime.assembly_task_fixed_service_failure",
                request_id,
                trace_id = %failure.error().envelope().trace_id(),
                error_id = %failure.error().envelope().error_id(),
            );
            if !self
                .request_supervisor
                .complete_fixed_service_failure(
                    supervised_request,
                    "request.error",
                    failure.error(),
                    CompletionTrace::RUNTIME,
                )
                .await
            {
                return;
            }
            if let Ok(message) = response_event_into_transport_message(
                request_id,
                OrdinaryResponseEvent::FixedServiceFailure(failure),
            ) {
                let _ = sender.send(message);
            }
            return;
        }
        let response_event = OrdinaryResponseEvent::try_error(&request_error)
            .expect("cancellation was split before ordinary response mapping");
        let response_error = request_error
            .ordinary_response_error()
            .expect("cancellation was split before ordinary response mapping");
        let runtime_error = request_error_into_runtime_error(request_error);
        error!(
            event = "runtime.assembly_task_request_error",
            request_id,
            error = %runtime_error
        );
        if !self
            .request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                CompletionTrace::RUNTIME,
            )
            .await
        {
            return;
        }
        if let Ok(message) = response_event_into_transport_message(request_id, response_event) {
            let _ = sender.send(message);
        }
    }

    async fn finish_websocket_connection_closed_error(
        &self,
        supervised_request: &SupervisedRequest,
        request_error: RequestError,
    ) {
        if request_error.is_cancellation_terminal() {
            self.request_supervisor
                .complete_cancelled(supervised_request, CompletionTrace::RUNTIME)
                .await;
            return;
        }
        let response_error = request_error
            .ordinary_response_error()
            .expect("cancellation was split before ordinary response mapping");
        self.request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                CompletionTrace::RUNTIME,
            )
            .await;
    }
}

fn bytecode_http_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeRequestStartFrameHeader,
    body: Vec<u8>,
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
            protocol: IngressProtocol::Http,
            method: Some(header.routing.ingress.method.clone()),
            path: header.routing.ingress.path.clone(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: body,
        extra: bytecode_deadline_extra(header.deadline.as_ref()),
    }
}

fn bytecode_task_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeTaskRequestStartFrameHeader,
    payload: Vec<u8>,
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
        ingress_selector: None,
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: payload,
        extra: bytecode_deadline_extra(header.deadline.as_ref()),
    }
}

fn bytecode_websocket_connect_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeWebSocketConnectRequestStartFrameHeader,
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
            method: None,
            path: header.routing.ingress.path.clone(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: bytecode_deadline_extra(header.deadline.as_ref()),
    }
}

fn bytecode_websocket_connection_closed_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
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
            method: None,
            path: header.routing.ingress.path.clone(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: bytecode_deadline_extra(header.deadline.as_ref()),
    }
}

fn bytecode_http_telemetry_context(
    host: &RuntimeHost,
    header: &BytecodeRequestStartFrameHeader,
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

fn bytecode_task_telemetry_context(
    host: &RuntimeHost,
    header: &BytecodeTaskRequestStartFrameHeader,
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

fn bytecode_websocket_connect_telemetry_context(
    host: &RuntimeHost,
    header: &BytecodeWebSocketConnectRequestStartFrameHeader,
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

fn bytecode_websocket_connection_closed_telemetry_context(
    host: &RuntimeHost,
    header: &BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
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

fn bytecode_deadline_extra(
    deadline: Option<&skiff_runtime_transport::protocol::BytecodeRequestDeadlineFrameHeader>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline).expect("typed bytecode deadline remains serializable"),
        );
    }
    extra
}

struct HostHttpGatewayResponseSink {
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    state: Mutex<HostHttpGatewayResponseState>,
}

struct HostHttpGatewayResponseState {
    ceiling: HttpResponseCeiling,
    accepting_stream_events: bool,
    terminal_settled: bool,
    pending_stream_terminal: Option<RouterWriterMessage>,
    pending_ordinary_error: Option<OrdinaryResponseEvent>,
}

impl HostHttpGatewayResponseSink {
    fn new(
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        http_response_max_bytes: usize,
    ) -> Self {
        Self {
            sender,
            state: Mutex::new(HostHttpGatewayResponseState {
                ceiling: HttpResponseCeiling::new(http_response_max_bytes),
                accepting_stream_events: true,
                terminal_settled: false,
                pending_stream_terminal: None,
                pending_ordinary_error: None,
            }),
        }
    }

    fn send_terminal_response(&self, request_id: &str, event: OrdinaryResponseEvent) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.terminal_settled {
            return;
        }
        state.accepting_stream_events = false;
        state.terminal_settled = true;
        state.pending_stream_terminal = None;
        state.pending_ordinary_error = None;
        if let Ok(message) = response_event_into_transport_message(request_id.to_string(), event) {
            let _ = self.sender.send(message);
        }
    }

    fn pending_ordinary_error(&self) -> Option<OrdinaryResponseEvent> {
        self.state.lock().ok()?.pending_ordinary_error.clone()
    }

    fn send_pending_stream_terminal(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.terminal_settled {
            return;
        }
        let Some(message) = state.pending_stream_terminal.take() else {
            return;
        };
        state.terminal_settled = true;
        let _ = self.sender.send(message);
    }

    fn cancel_without_response(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.accepting_stream_events = false;
            state.terminal_settled = true;
            state.pending_stream_terminal = None;
            state.pending_ordinary_error = None;
        }
    }
}

impl ResponseEventSink for HostHttpGatewayResponseSink {
    fn send_stream_event(
        &self,
        request_id: &str,
        event: ResponseStreamEvent,
    ) -> request_runner::RequestResult<()> {
        let mut state = self.state.lock().map_err(|_| {
            RequestError::Decode("HTTP gateway response sink lock is poisoned".to_string())
        })?;
        if !state.accepting_stream_events {
            return Err(RequestError::protocol(
                request_id,
                "HTTP gateway response emitted after its terminal frame",
            ));
        }
        if let Err(error) = state.ceiling.account_stream_event(&event) {
            state.pending_ordinary_error = Some(
                OrdinaryResponseEvent::try_error(&error)
                    .expect("response ceiling failure is ordinary"),
            );
            let payload = error
                .ordinary_payload()
                .expect("response ceiling failure is ordinary");
            let request_error = RequestError::external_error_payload(
                payload.code,
                payload.message,
                payload.status,
                payload.details,
            );
            state.accepting_stream_events = false;
            return Err(request_error);
        }
        let is_terminal = matches!(event, ResponseStreamEvent::End);
        let frame = skiff_runtime_transport::response_mapper::response_stream_event_into_frame(
            request_id, event,
        )
        .map_err(|error| RequestError::Decode(error.to_string()))?;
        let message = RouterWriterMessage::Binary(frame);
        if is_terminal {
            state.accepting_stream_events = false;
            state.pending_stream_terminal = Some(message);
            return Ok(());
        }
        if self.sender.send(message).is_err() {
            state.accepting_stream_events = false;
            state.terminal_settled = true;
            return Err(RequestError::Cancelled);
        }
        Ok(())
    }
}

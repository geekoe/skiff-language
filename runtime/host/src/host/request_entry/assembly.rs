use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use skiff_runtime_capability_context::{ConnectionRequestSession, ExecutionBudgetReason};
use skiff_runtime_eval::RuntimeWebSocketConnectResult;
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, RequestEnvelope, RequestError, ResponseEventSink,
    ResponseStreamEvent, RouterWriterMessage,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    encode_websocket_generation_lifecycle_frame, WebSocketGenerationLifecycleDirection,
};
use skiff_runtime_transport::{
    response_mapper::{
        runtime_assembly_websocket_connect_response_into_frame, OrdinaryResponseEvent,
    },
    runtime_assembly_request::{
        RuntimeAssemblyWebSocketConnectResponseFrameHeader,
        RuntimeAssemblyWebSocketConnectionPolicyFrameHeader,
        RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader,
    },
};
use tokio::{sync::mpsc, time::Duration};
use tracing::error;

use super::{
    assembly_wire::{AdmittedHttpGatewayRequest, AdmittedWebSocketConnectRequest},
    request_error_into_runtime_error, response_event_into_transport_message,
    response_into_transport_message,
};
use crate::{
    error::{Result, RuntimeError},
    host::{
        http_response_ceiling::HttpResponseCeiling,
        request_supervisor::{CompletionTrace, SupervisedRequest},
        RuntimeHost,
    },
    loader::assembly_admission::ActiveAssemblyRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    pub(super) async fn spawn_websocket_connect_on_active_assembly_route(
        &self,
        router_session_id: String,
        request: AdmittedWebSocketConnectRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedWebSocketConnectRequest { route, header } = request;
        let request_id = header.request_id.clone();
        if route.entry().optional_handler().is_none() {
            let connection_id = header.websocket_connect.connection_id.clone();
            let websocket_entry_id = header
                .websocket_connect
                .websocket_entry_id
                .as_str()
                .to_string();
            let receipt = match self.queue_websocket_generation_acquire(
                &route,
                &router_session_id,
                &websocket_entry_id,
                &connection_id,
                &sender,
            ) {
                Ok(receipt) => receipt,
                Err(error) => {
                    self.send_http_gateway_admission_error(&request_id, error, &sender);
                    return;
                }
            };
            let host = self.clone();
            tokio::spawn(async move {
                if let Err(error) = receipt.wait().await {
                    host.send_http_gateway_admission_error(&request_id, error, &sender);
                    return;
                }
                match websocket_connect_result_into_message(
                    request_id,
                    RuntimeWebSocketConnectResult::Accept {
                        business_identity: None,
                        connection_policy: None,
                    },
                ) {
                    Ok(message) => {
                        let _ = sender.send(message);
                    }
                    Err(error) => {
                        error!(event = "runtime.response_encode_error", error = %error)
                    }
                }
                drop(route);
            });
            return;
        }
        let target = match route.websocket_connect_target() {
            Ok(target) => target,
            Err(error) => {
                self.send_http_gateway_admission_error(&request_id, error, &sender);
                return;
            }
        };
        let handles = match self.websocket_connect_execution_handles(
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
        let telemetry = self.websocket_connect_telemetry_context(&header, &route);
        let supervisor_request = websocket_connect_supervisor_request(&header, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&supervisor_request, telemetry, "request.start")
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let timeout_ms = header
            .deadline
            .as_ref()
            .map(|deadline| deadline.timeout_ms)
            .into_iter()
            .chain(route.deployment_policy().timeout_ms)
            .min();
        let deadline = runtime_deadline(&execution_budget, timeout_ms);
        let host = self.clone();
        tokio::spawn(async move {
            let connection_id = header.websocket_connect.connection_id.clone();
            let websocket_entry_id = header
                .websocket_connect
                .websocket_entry_id
                .as_str()
                .to_string();
            let execution = request_runner::execute_runtime_websocket_connect(
                request_runner::RuntimeWebSocketConnectExecutionInput {
                    target,
                    header,
                    cancelled,
                    cancellation: cancellation.clone(),
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                },
            );
            tokio::pin!(execution);
            let cancel_wait = cancellation.clone();
            let result = match deadline {
                Some(deadline) => {
                    tokio::select! {
                        biased;
                        _ = cancel_wait.wait_cancelled() => Err(RequestError::Cancelled),
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                            cancellation.cancel();
                            execution_budget.record_deadline_exceeded();
                            Err(deadline_exceeded_error())
                        },
                        result = &mut execution => {
                            prefer_cancel_then_deadline(
                                result,
                                deadline,
                                &cancellation,
                                &execution_budget,
                            )
                        },
                    }
                }
                None => {
                    tokio::select! {
                        biased;
                        _ = cancel_wait.wait_cancelled() => Err(RequestError::Cancelled),
                        result = &mut execution => result,
                    }
                }
            };
            match result {
                Ok(result) => {
                    if matches!(result, RuntimeWebSocketConnectResult::Accept { .. }) {
                        let receipt = match host.queue_websocket_generation_acquire(
                            &route,
                            &router_session_id,
                            &websocket_entry_id,
                            &connection_id,
                            &sender,
                        ) {
                            Ok(receipt) => receipt,
                            Err(error) => {
                                host.finish_websocket_connect_error(
                                    &supervised_request,
                                    request_id,
                                    RequestError::Decode(error.to_string()),
                                    &sender,
                                )
                                .await;
                                return;
                            }
                        };
                        if let Err(error) = receipt.wait().await {
                            host.finish_websocket_connect_error(
                                &supervised_request,
                                request_id,
                                RequestError::Decode(error.to_string()),
                                &sender,
                            )
                            .await;
                            return;
                        }
                    }
                    if !host
                        .request_supervisor
                        .complete_success(
                            &supervised_request,
                            "request.end",
                            CompletionTrace::RUNTIME,
                        )
                        .await
                    {
                        return;
                    }
                    match websocket_connect_result_into_message(request_id, result) {
                        Ok(message) => {
                            let _ = sender.send(message);
                        }
                        Err(error) => {
                            error!(event = "runtime.response_encode_error", error = %error)
                        }
                    }
                }
                Err(error) => {
                    host.finish_websocket_connect_error(
                        &supervised_request,
                        request_id,
                        error,
                        &sender,
                    )
                    .await;
                }
            }
            drop(route);
        });
    }

    pub(super) async fn spawn_request_on_active_assembly_route(
        &self,
        router_session_id: String,
        request: AdmittedHttpGatewayRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedHttpGatewayRequest {
            route,
            header,
            body,
        } = request;
        let target = match route.request_target() {
            Ok(target) => target,
            Err(error) => {
                self.send_http_gateway_admission_error(&header.request_id, error, &sender);
                return;
            }
        };
        let response_sink = Arc::new(HostHttpGatewayResponseSink::new(
            sender.clone(),
            http_response_max_bytes,
        ));
        let handles = match self.http_gateway_execution_handles(
            &route,
            &header,
            http_response_max_bytes,
            &sender,
            Arc::clone(&response_sink),
            &router_session_id,
        ) {
            Ok(handles) => handles,
            Err(error) => {
                self.send_http_gateway_admission_error(&header.request_id, error, &sender);
                return;
            }
        };
        let telemetry = self.http_gateway_telemetry_context(&header, &route);
        let supervised_request = self
            .request_supervisor
            .begin_http_gateway(&header, telemetry, "request.start")
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let timeout_ms = header.deadline.as_ref().map(|deadline| deadline.timeout_ms);
        let deadline = runtime_deadline(&execution_budget, timeout_ms);
        let host = self.clone();
        tokio::spawn(async move {
            let request_id = header.request_id.clone();
            let execution = request_runner::execute_runtime_http_gateway_request(
                request_runner::RuntimeHttpGatewayExecutionInput {
                    target,
                    header,
                    body,
                    cancelled,
                    cancellation: cancellation.clone(),
                    execution_budget: Arc::clone(&execution_budget),
                    handles,
                },
            );
            tokio::pin!(execution);
            let cancel_wait = cancellation.clone();
            let result = match deadline {
                Some(deadline) => {
                    tokio::select! {
                        biased;
                        _ = cancel_wait.wait_cancelled() => Err(RequestError::Cancelled),
                        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
                            cancellation.cancel();
                            execution_budget.record_deadline_exceeded();
                            Err(deadline_exceeded_error())
                        },
                        result = &mut execution => {
                            prefer_cancel_then_deadline(
                                result,
                                deadline,
                                &cancellation,
                                &execution_budget,
                            )
                        },
                    }
                }
                None => {
                    tokio::select! {
                        biased;
                        _ = cancel_wait.wait_cancelled() => Err(RequestError::Cancelled),
                        result = &mut execution => result,
                    }
                }
            };
            host.finish_http_gateway_request(
                &supervised_request,
                &request_id,
                result,
                http_response_max_bytes,
                &response_sink,
                &sender,
            )
            .await;
            // The route pins the complete generation through execution, cancellation, terminal
            // response mapping and supervision.
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
                    .complete_success(supervised_request, "request.end", CompletionTrace::RUNTIME)
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

    fn send_http_gateway_admission_error(
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

    fn http_gateway_execution_handles(
        &self,
        route: &ActiveAssemblyRoute,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestStartFrameHeader,
        http_response_max_bytes: usize,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        response_sink: Arc<HostHttpGatewayResponseSink>,
        router_session_id: &str,
    ) -> Result<request_runner::RuntimeHttpGatewayExecutionHandles> {
        let telemetry = self.http_gateway_telemetry_context(header, route);
        let eval_adapter = crate::eval_capability_adapter::http_gateway_eval_adapter(
            crate::eval_capability_adapter::RuntimeHttpGatewayEvalAdapterInput {
                runtime_id: self.base_runtime_id.clone(),
                activation: Arc::clone(route.activation()),
                execution_image: Arc::clone(route.execution_image()),
                header: header.clone(),
                gateway_entry_key: route.gateway_entry_key().as_str().to_string(),
                service_protocol_identity: route.service_protocol_identity().as_str().to_string(),
                ingress_selector: route.selector().clone(),
                db_source: route
                    .db_source()
                    .map_err(|error| RuntimeError::Decode(error.to_string()))?,
                file_source: crate::capability_context::FileCapabilitySource::new(
                    self.file_runtime(),
                ),
                http_options: self.http_runtime_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                actor_method_outbound: Arc::clone(&self.actor_method_outbound),
                spawn_workers: Arc::clone(&self.spawn_workers),
                telemetry_context: Some(telemetry),
                router_sender: Some(sender.clone()),
                connection_requests: Arc::clone(&self.connection_requests),
                router_session: ConnectionRequestSession::new(router_session_id.to_string())
                    .map_err(RuntimeError::Decode)?,
                http_response_max_bytes,
            },
        )
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        Ok(request_runner::RuntimeHttpGatewayExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
            eval_adapter,
            response_events: response_sink,
        })
    }

    fn http_gateway_telemetry_context(
        &self,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestStartFrameHeader,
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

    fn websocket_connect_execution_handles(
        &self,
        route: &ActiveAssemblyRoute,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        http_response_max_bytes: usize,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        router_session_id: &str,
    ) -> Result<request_runner::RuntimeWebSocketConnectExecutionHandles> {
        let telemetry = self.websocket_connect_telemetry_context(header, route);
        let eval_adapter = crate::eval_capability_adapter::websocket_connect_eval_adapter(
            crate::eval_capability_adapter::RuntimeWebSocketConnectEvalAdapterInput {
                runtime_id: self.base_runtime_id.clone(),
                activation: Arc::clone(route.activation()),
                execution_image: Arc::clone(route.execution_image()),
                header: header.clone(),
                gateway_entry_key: route.gateway_entry_key().as_str().to_string(),
                service_protocol_identity: route.service_protocol_identity().as_str().to_string(),
                ingress_selector: route.selector().clone(),
                db_source: route
                    .db_source()
                    .map_err(|error| RuntimeError::Decode(error.to_string()))?,
                file_source: crate::capability_context::FileCapabilitySource::new(
                    self.file_runtime(),
                ),
                http_options: self.http_runtime_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                actor_method_outbound: Arc::clone(&self.actor_method_outbound),
                spawn_workers: Arc::clone(&self.spawn_workers),
                telemetry_context: Some(telemetry),
                router_sender: Some(sender.clone()),
                connection_requests: Arc::clone(&self.connection_requests),
                router_session: ConnectionRequestSession::new(router_session_id.to_string())
                    .map_err(RuntimeError::Decode)?,
                http_response_max_bytes,
            },
        )
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        Ok(request_runner::RuntimeWebSocketConnectExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
            eval_adapter,
        })
    }

    fn websocket_connect_telemetry_context(
        &self,
        header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
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

    fn queue_websocket_generation_acquire(
        &self,
        route: &ActiveAssemblyRoute,
        router_session_id: &str,
        websocket_entry_id: &str,
        connection_id: &str,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<super::super::websocket_generation::WebSocketGenerationAcquireReceipt> {
        let (request, receipt) = self.websocket_generations.begin_acquire_with_receipt(
            router_session_id,
            route.clone(),
            websocket_entry_id.to_string(),
            connection_id.to_string(),
        )?;
        let frame = match encode_websocket_generation_lifecycle_frame(
            WebSocketGenerationLifecycleDirection::RuntimeToRouter,
            &request,
        ) {
            Ok(frame) => frame,
            Err(error) => {
                self.websocket_generations.rollback_acquire(&request)?;
                return Err(RuntimeError::Decode(error.to_string()));
            }
        };
        if sender.send(RouterWriterMessage::Binary(frame)).is_err() {
            self.websocket_generations.rollback_acquire(&request)?;
            return Err(RuntimeError::Decode(
                "failed to queue WebSocket generation acquire".to_string(),
            ));
        }
        Ok(receipt)
    }

    #[cfg(test)]
    pub(crate) fn queue_websocket_generation_acquire_for_test(
        &self,
        route: &ActiveAssemblyRoute,
        router_session_id: &str,
        websocket_entry_id: &str,
        connection_id: &str,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        self.queue_websocket_generation_acquire(
            route,
            router_session_id,
            websocket_entry_id,
            connection_id,
            sender,
        )
        .map(|_| ())
    }
}

fn websocket_connect_supervisor_request(
    header: &skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    route: &ActiveAssemblyRoute,
) -> RequestEnvelope {
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline)
                .expect("typed WebSocket connect deadline remains serializable"),
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

fn websocket_connect_result_into_message(
    request_id: String,
    result: RuntimeWebSocketConnectResult,
) -> Result<RouterWriterMessage> {
    let response = match result {
        RuntimeWebSocketConnectResult::Accept {
            business_identity,
            connection_policy,
        } => RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
            business_identity,
            connection_policy: connection_policy.map(|policy| {
                RuntimeAssemblyWebSocketConnectionPolicyFrameHeader {
                    max_connections: policy.max_connections,
                    overflow: match policy.overflow {
                        skiff_runtime_capability_context::WebSocketConnectionPolicyOverflowControl::CloseOldest => {
                            RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader::CloseOldest
                        }
                        skiff_runtime_capability_context::WebSocketConnectionPolicyOverflowControl::RejectNew => {
                            RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader::RejectNew
                        }
                    },
                    close_code: policy.close_code,
                    close_reason: policy.close_reason,
                }
            }),
        },
        RuntimeWebSocketConnectResult::Reject { code, reason } => {
            RuntimeAssemblyWebSocketConnectResponseFrameHeader::Reject { code, reason }
        }
    };
    runtime_assembly_websocket_connect_response_into_frame(request_id, response)
        .map(RouterWriterMessage::Binary)
        .map_err(super::transport_error_into_runtime_error)
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
            // The stream operation has selected its semantic terminal, but root
            // cancellation can still be racing it. Keep the wire terminal
            // private until RequestSupervisor grants success ownership.
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

fn deadline_exceeded_error() -> RequestError {
    RequestError::ExecutionBudgetExceeded {
        reason: ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 0,
        limit: None,
        elapsed_ms: 0.0,
    }
}

fn runtime_deadline(
    execution_budget: &skiff_runtime_request::ExecutionBudget,
    timeout_ms: Option<u64>,
) -> Option<Instant> {
    let timeout_deadline = timeout_ms
        .and_then(|timeout_ms| Instant::now().checked_add(Duration::from_millis(timeout_ms)));
    execution_budget
        .deadline()
        .into_iter()
        .chain(timeout_deadline)
        .min()
}

fn prefer_cancel_then_deadline<T>(
    result: request_runner::RequestResult<T>,
    deadline: Instant,
    cancellation: &skiff_runtime_request::cancellation::CancellationToken,
    execution_budget: &skiff_runtime_request::ExecutionBudget,
) -> request_runner::RequestResult<T> {
    // A ready execution lane can wake the task before Tokio's timer driver has
    // marked an equally-expired Sleep ready. Recheck the two semantic owners
    // here so scheduler timing cannot bypass the biased cancel/deadline order.
    if cancellation.is_cancelled() {
        execution_budget.record_cancelled();
        return Err(RequestError::Cancelled);
    }
    if Instant::now() >= deadline {
        cancellation.cancel();
        execution_budget.record_deadline_exceeded();
        return Err(deadline_exceeded_error());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_terminal_waits_for_root_owner_and_cancel_discards_it() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let sink = HostHttpGatewayResponseSink::new(sender, 1024);

        sink.send_stream_event("request-stream-race", ResponseStreamEvent::End)
            .expect("stream producer may select its end");
        assert!(
            receiver.try_recv().is_err(),
            "stream end must remain private until root grants success ownership"
        );

        sink.cancel_without_response();
        sink.send_pending_stream_terminal();
        assert!(
            receiver.try_recv().is_err(),
            "root cancellation must discard the losing stream terminal"
        );
    }

    #[test]
    fn stream_terminal_flushes_after_root_success_owner() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let sink = HostHttpGatewayResponseSink::new(sender, 1024);

        sink.send_stream_event("request-stream-success", ResponseStreamEvent::End)
            .expect("stream producer may select its end");
        sink.send_pending_stream_terminal();

        let message = receiver
            .try_recv()
            .expect("root success owner must flush the pending stream terminal");
        let RouterWriterMessage::Binary(frame) = message else {
            panic!("stream terminal must use the binary transport")
        };
        let (header, payload): (
            skiff_runtime_transport::protocol::ResponseEndFrameHeader,
            Vec<u8>,
        ) = skiff_runtime_transport::protocol::decode_typed_binary_frame(&frame)
            .expect("pending stream terminal must remain canonical");
        assert_eq!(header.request_id, "request-stream-success");
        assert!(payload.is_empty());
        assert!(receiver.try_recv().is_err());
    }
}

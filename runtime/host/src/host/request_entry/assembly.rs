use std::sync::Arc;

use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, RequestEnvelope, ResponseEnd, ResponseEvent,
    RouterWriterMessage, WebSocketResponse,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    encode_websocket_generation_lifecycle_frame, WebSocketGenerationLifecycleDirection,
};
use tokio::sync::mpsc;
use tracing::error;

use super::{
    request_error_into_runtime_error, response_event_into_transport_message,
    response_into_transport_message, websocket_generation::WebSocketConnectGenerationPin,
};
use crate::{
    error::{Result, RuntimeError},
    host::{
        request_supervisor::{CompletionTrace, SupervisedRequest},
        RuntimeHost,
    },
    loader::assembly_admission::ActiveAssemblyRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    pub(super) async fn spawn_request_on_active_assembly_route(
        &self,
        route: ActiveAssemblyRoute,
        request: RequestEnvelope,
        connect_pin: Option<WebSocketConnectGenerationPin>,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let target = match route.request_target() {
            Ok(target) => target,
            Err(error) => {
                let error = RuntimeError::Decode(error.to_string());
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        let handles = match self.assembly_request_execution_handles(
            &route,
            &request,
            http_response_max_bytes,
            &sender,
        ) {
            Ok(handles) => handles,
            Err(error) => {
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        self.spawn_assembly_request(
            route,
            target,
            handles,
            request,
            connect_pin,
            http_response_max_bytes,
            sender,
        )
        .await;
    }

    async fn spawn_assembly_request(
        &self,
        route: ActiveAssemblyRoute,
        target: request_runner::RuntimeAssemblyRequestTarget,
        handles: request_runner::AssemblyRequestExecutionHandles,
        request: RequestEnvelope,
        connect_pin: Option<WebSocketConnectGenerationPin>,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let telemetry_context = self.assembly_request_telemetry_context(&request, &route);
        let supervised_request = self
            .request_supervisor
            .begin(&request, telemetry_context, "request.start")
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let is_http_ingress = request.ingress_selector.as_ref().is_some_and(|selector| {
            selector.protocol == skiff_artifact_model::IngressProtocol::Http
        });
        let host = self.clone();
        tokio::spawn(async move {
            let request_id = request.request_id.clone();
            let result = request_runner::execute_runtime_assembly_request(
                request_runner::AssemblyRequestExecutionInput {
                    target,
                    request,
                    cancelled,
                    cancellation,
                    execution_budget,
                    handles,
                },
            )
            .await;
            let writer_message = match result {
                Ok(response) => {
                    if let Err(response_error) =
                        super::super::http_response_ceiling::validate_unary_response(
                            &response,
                            http_response_max_bytes,
                            is_http_ingress,
                        )
                    {
                        host.request_supervisor
                            .complete_error(
                                &supervised_request,
                                "request.error",
                                &response_error,
                                CompletionTrace::RUNTIME,
                            )
                            .await;
                        let message = response_event_into_transport_message(
                            request_id,
                            ResponseEvent::Error(response_error),
                        )
                        .map(Some);
                        drop(route);
                        if let Ok(Some(message)) = message {
                            let _ = sender.send(message);
                        }
                        return;
                    }
                    if websocket_connect_accepted(&response) {
                        if let Some(connect_pin) = connect_pin {
                            if let Err(error) = host.queue_websocket_generation_acquire(
                                &route,
                                connect_pin,
                                &sender,
                            ) {
                                host.send_websocket_generation_acquire_error(
                                    &supervised_request,
                                    request_id,
                                    &error,
                                    &sender,
                                )
                                .await;
                                return;
                            }
                        }
                    }
                    host.request_supervisor
                        .complete_success(
                            &supervised_request,
                            "request.end",
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    response_into_transport_message(request_id, response)
                }
                Err(request_error) => {
                    let response_error = request_error.response_error();
                    let error = request_error_into_runtime_error(request_error);
                    error!(
                        event = "runtime.assembly_request_error",
                        request_id = %request_id,
                        error = %error
                    );
                    let event_name = if error.is_request_cancelled() {
                        "request.cancel"
                    } else {
                        "request.error"
                    };
                    host.request_supervisor
                        .complete_error(
                            &supervised_request,
                            event_name,
                            &response_error,
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    response_event_into_transport_message(
                        request_id,
                        ResponseEvent::Error(response_error),
                    )
                    .map(Some)
                }
            };
            match writer_message {
                Ok(Some(message)) => {
                    let _ = sender.send(message);
                }
                Ok(None) => {}
                Err(error) => error!(event = "runtime.response_encode_error", error = %error),
            }
            // `route` is retained by this task so reload cannot drop the activation/context set
            // while cancellation, response encoding, or request supervision is still active.
            drop(route);
        });
    }

    async fn send_websocket_generation_acquire_error(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        error: &RuntimeError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let response_error = crate::capability_context::response_error_from_runtime_error(error);
        self.request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                CompletionTrace::RUNTIME,
            )
            .await;
        match response_event_into_transport_message(
            request_id,
            ResponseEvent::Error(response_error),
        ) {
            Ok(message) => {
                let _ = sender.send(message);
            }
            Err(error) => {
                error!(
                    event = "runtime.response_encode_error",
                    error = %error
                );
            }
        }
    }

    fn queue_websocket_generation_acquire(
        &self,
        route: &ActiveAssemblyRoute,
        connect_pin: WebSocketConnectGenerationPin,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<()> {
        let request = self.websocket_generations.begin_acquire(
            &connect_pin.router_session_id,
            route.clone(),
            connect_pin.websocket_entry_id,
            connect_pin.connection_id,
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
        Ok(())
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
            WebSocketConnectGenerationPin {
                router_session_id: router_session_id.to_string(),
                websocket_entry_id: websocket_entry_id.to_string(),
                connection_id: connection_id.to_string(),
            },
            sender,
        )
    }

    fn assembly_request_execution_handles(
        &self,
        route: &ActiveAssemblyRoute,
        request: &RequestEnvelope,
        http_response_max_bytes: usize,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<request_runner::AssemblyRequestExecutionHandles> {
        let telemetry = self.assembly_request_telemetry_context(request, route);
        let eval_adapter = crate::eval_capability_adapter::assembly_request_eval_adapter(
            crate::eval_capability_adapter::RuntimeAssemblyRequestEvalAdapterInput {
                runtime_id: self.base_runtime_id.clone(),
                activation: Arc::clone(route.activation()),
                execution_image: Arc::clone(route.execution_image()),
                db_source: route
                    .db_source()
                    .map_err(|error| RuntimeError::Decode(error.to_string()))?,
                file_source: crate::capability_context::FileCapabilitySource::new(
                    self.file_runtime(),
                ),
                http_options: self.http_runtime_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                spawn_workers: Arc::clone(&self.spawn_workers),
                telemetry_context: Some(telemetry),
                router_sender: Some(sender.clone()),
                http_response_max_bytes,
            },
        )
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        Ok(request_runner::AssemblyRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
            eval_adapter,
        })
    }

    fn assembly_request_telemetry_context(
        &self,
        request: &RequestEnvelope,
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
        context.request_id = Some(request.request_id.clone());
        context.target = Some(
            route
                .operation_descriptor()
                .operation_id
                .as_str()
                .to_string(),
        );
        context
    }
}

fn websocket_connect_accepted(response: &BoundaryResponse) -> bool {
    matches!(
        response,
        BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::WebSocket(
            WebSocketResponse::ConnectAccept(_)
        )))
    )
}

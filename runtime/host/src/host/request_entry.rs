use std::sync::{atomic::AtomicBool, Arc};

use skiff_artifact_model::IngressSelector;
use skiff_runtime_linked_program::ExecutableAddr;
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, ExecutionBudget, RequestCancel, RequestEnvelope,
    RequestError, RequestOperationContext, RequestResult, ResponseError, ResponseEvent,
    ResponseEventSink, ResponseStreamEvent, RouterWriterMessage, RuntimeOperation,
};
use skiff_runtime_transport::{response_mapper, TransportError};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    loader::assembly_admission::ActiveAssemblyRoute,
    telemetry::RequestTelemetryContext,
};

#[cfg(test)]
use super::request_supervisor::CompletionTrace;
use super::{route_registry, RuntimeHost, ServiceOperationContext, ServiceRuntimeContext};

mod assembly;
mod assembly_wire;
mod websocket_generation;

struct RouterResponseEventSink {
    sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
}

impl RouterResponseEventSink {
    fn new(sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>) -> Self {
        Self { sender }
    }
}

impl ResponseEventSink for RouterResponseEventSink {
    fn send_stream_event(&self, request_id: &str, event: ResponseStreamEvent) -> RequestResult<()> {
        let frame = response_mapper::response_stream_event_into_frame(request_id, event)
            .map_err(request_error_from_transport_error)?;
        let sender = self.sender.as_ref().ok_or_else(|| {
            RequestError::protocol(
                request_id.to_string(),
                "serverStream request is missing router sender",
            )
        })?;
        sender
            .send(RouterWriterMessage::Binary(frame))
            .map_err(|_| RequestError::Cancelled)
    }
}

impl RuntimeHost {
    #[cfg(test)]
    pub(crate) async fn spawn_resolved_request_for_test(
        &self,
        operation_context: ServiceOperationContext,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        error_log_event: &'static str,
    ) {
        let service = operation_context.service.clone();
        let build_guard = match self.begin_build_execution(&service.build_id) {
            Ok(guard) => guard,
            Err(error) => {
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };

        let telemetry_context = self.request_telemetry_context(&request, &service);
        let supervised_request = self
            .request_supervisor
            .begin(&request, telemetry_context.clone(), "request.start")
            .await;
        let cancelled = supervised_request.cancelled();
        let cancellation = supervised_request.cancellation_token();
        let execution_budget = supervised_request.execution_budget();
        let request_operation_context = operation_context.request_operation_context();

        let host = self.clone();
        tokio::spawn(async move {
            let _build_guard = build_guard;
            let request_id = request.request_id.clone();
            let request_target = request.target.clone();
            let diagnostic_context = request_operation_context.clone();
            let result =
                request_runner::execute_runtime_request(request_runner::RequestExecutionInput {
                    operation_context: request_operation_context,
                    request,
                    cancelled,
                    cancellation,
                    execution_budget: execution_budget.clone(),
                    handles: host.request_execution_handles(
                        service.clone(),
                        Some(telemetry_context),
                        Some(sender.clone()),
                    ),
                })
                .await;
            let writer_message = match result {
                Ok(response) => {
                    host.request_supervisor
                        .complete_success(
                            &supervised_request,
                            "request.end",
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    response_into_transport_message(request_id, response)
                }
                Err(error) => {
                    let (error, response_error) =
                        request_execution_error_into_runtime_error_and_response(
                            &diagnostic_context,
                            request_target.as_str(),
                            error,
                        );
                    error!(
                        event = error_log_event,
                        request_id = %request_id,
                        runtime_id = %service.runtime_id,
                        service_id = %service.service_id,
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
                Err(error) => {
                    error!(event = "runtime.response_encode_error", error = %error);
                }
            }
        });
    }

    fn request_execution_handles(
        &self,
        service: Arc<ServiceRuntimeContext>,
        telemetry_context: Option<RequestTelemetryContext>,
        router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    ) -> request_runner::RequestExecutionHandles {
        let streaming_available = router_sender.is_some();
        let response_events = Arc::new(RouterResponseEventSink::new(router_sender.clone()));
        let eval_adapter = crate::eval_capability_adapter::request_eval_adapter(
            crate::eval_capability_adapter::RuntimeRequestEvalAdapterInput {
                service,
                file_source: crate::capability_context::FileCapabilitySource::new(
                    self.file_runtime(),
                ),
                http_options: self.http_runtime_options.clone(),
                outbound_requests: self.outbound_requests.clone(),
                spawn_workers: self.spawn_workers.clone(),
                telemetry_context,
                router_sender,
            },
        );
        request_runner::RequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
            streaming_available,
            response_events,
            eval_adapter,
        }
    }

    pub(super) fn send_request_error_response(
        &self,
        request: &RequestEnvelope,
        error: &RuntimeError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        match response_event_into_transport_message(
            request.request_id.clone(),
            ResponseEvent::Error(response_error_from_runtime_error(error)),
        ) {
            Ok(message) => {
                let _ = sender.send(message);
            }
            Err(error) => {
                error!(event = "runtime.response_encode_error", error = %error);
            }
        }
    }

    fn lookup_operation_in_state(
        &self,
        request: &RequestEnvelope,
    ) -> Result<ServiceOperationContext> {
        let state = self.state.read().map_err(|_| {
            RuntimeError::Decode("runtime service route state lock is poisoned".to_string())
        })?;
        let build_id = request.build_id();
        if build_id.is_empty() {
            return Err(RuntimeError::Unsupported(
                "request.start buildId is required".to_string(),
            ));
        }
        let operation = route_registry::lookup_operation_by_build_id(&state, request, build_id)?;
        self.loaded_builds.touch(build_id);
        Ok(operation)
    }

    #[allow(dead_code)]
    pub(crate) fn lookup_operation(
        &self,
        request: &RequestEnvelope,
    ) -> Result<ServiceOperationContext> {
        self.lookup_operation_in_state(request)
    }

    /// Resolves a canonical ingress only from one immutable active assembly generation.
    ///
    /// The canonical wire bridge supplies the selector. This entry performs no artifact access or
    /// candidate mutation and returns the activation template plus descriptor pinned by
    /// `ActiveAssemblyRoute`.
    pub(crate) fn lookup_active_assembly_request_route(
        &self,
        selector: &IngressSelector,
    ) -> Result<ActiveAssemblyRoute> {
        let route = self
            .active_runtime_assembly_route(selector)
            .map_err(|error| RuntimeError::Decode(error.to_string()))?
            .ok_or_else(|| {
                RuntimeError::Unsupported(format!(
                    "no active assembly ingress matches {selector:?}"
                ))
            })?;
        Ok(route)
    }

    #[cfg(test)]
    pub(crate) fn lookup_request_operation(
        &self,
        request: &RequestEnvelope,
    ) -> Result<ServiceOperationContext> {
        self.lookup_operation_in_state(request)
    }

    pub(crate) async fn cancel_request(&self, cancel: RequestCancel) {
        if self.request_supervisor.cancel(&cancel).await {
            info!(
                event = "runtime.request_cancelled",
                request_id = %cancel.request_id,
                reason = cancel.reason.as_deref().unwrap_or("unknown")
            );
        }
    }

    pub(crate) async fn execute_runtime_request(
        &self,
        service: Arc<ServiceRuntimeContext>,
        operation: RuntimeOperation,
        addr: ExecutableAddr,
        request: RequestEnvelope,
        cancelled: Arc<AtomicBool>,
        cancellation: skiff_runtime_request::cancellation::CancellationToken,
        execution_budget: Arc<ExecutionBudget>,
        router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    ) -> Result<request_runner::RuntimeResponse> {
        let telemetry_context = self.request_telemetry_context(&request, service.as_ref());
        let operation_context = ServiceOperationContext::new(service, operation, addr);
        let request_operation_context = operation_context.request_operation_context();
        let request_target = request.target.clone();
        request_runner::execute_runtime_request(request_runner::RequestExecutionInput {
            operation_context: request_operation_context.clone(),
            request,
            cancelled,
            cancellation,
            execution_budget,
            handles: self.request_execution_handles(
                operation_context.service.clone(),
                Some(telemetry_context),
                router_sender,
            ),
        })
        .await
        .map_err(|error| {
            request_execution_error_into_runtime_error(
                &request_operation_context,
                request_target.as_str(),
                error,
            )
        })
    }
}

fn request_execution_error_into_runtime_error(
    operation_context: &RequestOperationContext,
    request_target: &str,
    error: request_runner::RequestExecutionError,
) -> RuntimeError {
    request_execution_error_into_runtime_error_and_response(
        operation_context,
        request_target,
        error,
    )
    .0
}

fn request_execution_error_into_runtime_error_and_response(
    operation_context: &RequestOperationContext,
    request_target: &str,
    error: request_runner::RequestExecutionError,
) -> (RuntimeError, ResponseError) {
    let attach_request_diagnostic = error.attach_request_diagnostic();
    let request_error = error.into_error();
    if !attach_request_diagnostic {
        let response_error = request_error.response_error();
        return (
            request_error_into_runtime_error(request_error),
            response_error,
        );
    }
    let error = request_error_into_runtime_error(request_error);
    let error = crate::eval_capability_adapter::attach_request_error_diagnostic_frame(
        error,
        operation_context.eval_program.as_ref(),
        operation_context.operation.operation.as_str(),
        request_target,
        operation_context.metadata.build_id.as_str(),
        &operation_context.addr,
    );
    let response_error = response_error_from_runtime_error(&error);
    (error, response_error)
}

fn request_error_into_runtime_error(error: RequestError) -> RuntimeError {
    RuntimeError::Opaque(Box::new(error))
}

fn request_error_from_transport_error(error: TransportError) -> RequestError {
    RequestError::Decode(error.to_string())
}

pub(crate) fn transport_error_into_runtime_error(error: TransportError) -> RuntimeError {
    RuntimeError::Decode(error.to_string())
}

fn response_into_transport_message(
    request_id: String,
    response: BoundaryResponse,
) -> Result<Option<RouterWriterMessage>> {
    match response {
        BoundaryResponse::Event(event) => {
            response_event_into_transport_message(request_id, event).map(Some)
        }
        BoundaryResponse::StreamSent => Ok(None),
    }
}

fn response_event_into_transport_message(
    request_id: String,
    event: ResponseEvent,
) -> Result<RouterWriterMessage> {
    response_mapper::response_event_into_frame(request_id, event)
        .map(RouterWriterMessage::Binary)
        .map_err(transport_error_into_runtime_error)
}

#[cfg(test)]
mod tests {
    use skiff_runtime_capability_context::ExecutionBudgetReason;

    use crate::error::{RuntimeError, TypeIdentity, WirePayload};

    use super::*;

    #[test]
    fn request_error_bridge_boxes_and_delegates_payload_and_catch_projection() {
        let request_error = RequestError::protocol("svc.account", "bad frame");
        let expected_payload = request_error.payload();
        let expected_catch_projection = request_error.catch_projection();

        let error = request_error_into_runtime_error(request_error);

        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert_eq!(error.payload(), expected_payload);
        assert_eq!(
            WirePayload::catch_projection(&error),
            expected_catch_projection
        );
        assert_eq!(
            WirePayload::catch_projection(&error),
            Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                serde_json::json!({
                    "target": "svc.account",
                    "message": "bad frame",
                })
            ))
        );
    }

    #[test]
    fn request_error_bridge_preserves_carried_cancellation_detection() {
        let error = request_error_into_runtime_error(RequestError::Cancelled);
        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert!(error.is_request_cancelled());

        let error = request_error_into_runtime_error(RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::Cancelled,
            instruction_count: 0,
            limit: None,
            elapsed_ms: 0.0,
        });
        assert!(matches!(error, RuntimeError::Opaque(_)));
        assert!(error.is_request_cancelled());
    }
}

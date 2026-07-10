use std::sync::{atomic::AtomicBool, Arc};

use skiff_runtime_linked_program::ExecutableAddr;
use skiff_runtime_request::{
    self as request_runner, BoundaryResponse, ExecutionBudget, RequestCancel, RequestEnvelope,
    RequestError, RequestOperationContext, RequestResult, ResponseError, ResponseEvent,
    ResponseEventSink, ResponseStreamEvent, RouterWriterMessage, RuntimeOperation,
};
use skiff_runtime_transport::{response_mapper, TransportError};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    loader::load_service_build_from_artifact_roots_with_caches,
    telemetry::RequestTelemetryContext,
};

use super::{
    control_plane::apply_control_config, package_test_entry, request_supervisor::CompletionTrace,
    route_registry, spawn_worker, RuntimeHost, ServiceOperationContext, ServiceRuntimeContext,
};

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
    pub(crate) async fn spawn_request(
        &self,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let operation_context = match self
            .lookup_or_load_operation(&request, sender.clone())
            .await
        {
            Ok(operation_context) => operation_context,
            Err(error) => {
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };

        self.spawn_resolved_request(operation_context, request, sender, "runtime.request_error")
            .await;
    }

    pub(super) async fn spawn_resolved_request(
        &self,
        operation_context: ServiceOperationContext,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        error_log_event: &'static str,
    ) {
        self.spawn_resolved_request_inner(
            operation_context,
            request,
            sender,
            error_log_event,
            None,
        )
        .await;
    }

    pub(super) async fn spawn_resolved_package_test_request(
        &self,
        operation_context: ServiceOperationContext,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        error_log_event: &'static str,
        pending_start: package_test_entry::PackageTestPendingStart,
    ) {
        self.spawn_resolved_request_inner(
            operation_context,
            request,
            sender,
            error_log_event,
            Some(pending_start),
        )
        .await;
    }

    async fn spawn_resolved_request_inner(
        &self,
        operation_context: ServiceOperationContext,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        error_log_event: &'static str,
        pending_start: Option<package_test_entry::PackageTestPendingStart>,
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
        if let Some(pending_start) = pending_start {
            if pending_start.finish() {
                let response_error = response_error_from_runtime_error(&RuntimeError::cancelled());
                self.request_supervisor
                    .complete_error(
                        &supervised_request,
                        "request.cancel",
                        &response_error,
                        CompletionTrace::RUNTIME,
                    )
                    .await;
                match response_event_into_transport_message(
                    request.request_id.clone(),
                    ResponseEvent::Error(response_error),
                ) {
                    Ok(message) => {
                        let _ = sender.send(message);
                    }
                    Err(error) => {
                        error!(event = "runtime.response_encode_error", error = %error);
                    }
                }
                return;
            }
        }
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

    pub(crate) fn submit_package_test_start(
        &self,
        header: skiff_runtime_transport::protocol::PackageTestStartFrameHeader,
        payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        package_test_entry::spawn_package_test_start(self, header, payload, sender);
    }

    #[cfg(test)]
    pub(crate) async fn load_package_test_runtime_program(
        &self,
        header: &skiff_runtime_transport::protocol::PackageTestStartFrameHeader,
    ) -> Result<skiff_runtime_package_test::LoadedPackageTestRuntimeProgram> {
        package_test_entry::load_package_test_runtime_program(self, header).await
    }

    #[cfg(test)]
    pub(crate) fn package_test_service_context(
        &self,
        loaded: &skiff_runtime_package_test::LoadedPackageTestRuntimeProgram,
        header: &skiff_runtime_transport::protocol::PackageTestStartFrameHeader,
    ) -> Result<Arc<ServiceRuntimeContext>> {
        package_test_entry::package_test_service_context(self, loaded, header)
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

    pub(crate) async fn lookup_or_load_operation(
        &self,
        request: &RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<ServiceOperationContext> {
        match self.lookup_operation_in_state(request) {
            Ok(operation) => return Ok(operation),
            Err(error) if request.service_id.is_none() => return Err(error),
            Err(_) => {}
        }

        let loaded = self.lazy_load_request_service(request).await?;
        if !loaded.is_empty() {
            self.queue_service_registers(sender.clone(), &loaded)?;
            spawn_worker::start_spawn_workers_for_services(self.clone(), sender.clone(), loaded);
        }
        self.lookup_operation_in_state(request)
    }

    async fn lazy_load_request_service(
        &self,
        request: &RequestEnvelope,
    ) -> Result<Vec<Arc<ServiceRuntimeContext>>> {
        let Some(service_id) = request.service_id.as_deref() else {
            return Ok(Vec::new());
        };
        let build_id = request.build_id();
        if !route_registry::is_service_build_id(build_id) {
            return Err(RuntimeError::Unsupported(format!(
                "request.start buildId must be skiff-service-build-v1:sha256:<64 lowercase hex>, got {}",
                build_id
            )));
        }

        let load_state = self.artifact_load_state.lock().await;
        if self.lookup_operation_in_state(request).is_ok() {
            return Ok(Vec::new());
        }
        if load_state.artifact_roots.is_empty() {
            return Err(RuntimeError::Unsupported(format!(
                "no artifact roots are configured for lazy loading serviceId {} buildId {}",
                service_id, build_id
            )));
        }
        let has_control_config = load_state
            .service_config
            .iter()
            .any(|config| config.service_id == service_id && config.build_id == build_id);
        let mut services = match load_service_build_from_artifact_roots_with_caches(
            &load_state.artifact_roots,
            service_id,
            build_id,
            &self.base_runtime_id,
            self.default_http_response_max_bytes,
            &load_state.load_options,
            &self.artifact_caches,
            has_control_config,
        )
        .await
        {
            Ok(services) => services,
            Err(error) => {
                warn!(
                    event = "runtime.lazy_load_service_failed",
                    service_id,
                    build_id,
                    stage = "load_artifact",
                    error = %error
                );
                return Err(RuntimeError::invalid_artifact(error.to_string()));
            }
        };
        services = match apply_control_config(services, &load_state.service_config) {
            Ok(services) => services,
            Err(error) => {
                warn!(
                    event = "runtime.lazy_load_service_failed",
                    service_id,
                    build_id,
                    stage = "apply_control_config",
                    error = %error
                );
                return Err(RuntimeError::invalid_artifact(error.to_string()));
            }
        };
        drop(load_state);

        let loaded_count = services.len();
        let added = self.add_services(services).map_err(|error| {
            warn!(
                event = "runtime.lazy_load_service_failed",
                service_id,
                build_id,
                stage = "add_services",
                error = %error
            );
            RuntimeError::invalid_artifact(error.to_string())
        })?;
        if loaded_count > 0 && added.is_empty() {
            warn!(
                event = "runtime.lazy_load_service_noop",
                service_id, build_id, loaded_count
            );
        }
        Ok(added)
    }

    pub(crate) async fn cancel_request(&self, cancel: RequestCancel) {
        if self.request_supervisor.cancel(&cancel).await {
            info!(
                event = "runtime.request_cancelled",
                request_id = %cancel.request_id,
                reason = cancel.reason.as_deref().unwrap_or("unknown")
            );
        } else if self
            .package_test_start_executor
            .cancel_pending(&cancel.request_id)
        {
            info!(
                event = "runtime.package_test_start_cancelled",
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

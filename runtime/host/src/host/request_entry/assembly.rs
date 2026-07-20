use std::sync::Arc;

use skiff_runtime_request::{
    self as request_runner, RequestEnvelope, ResponseEvent, RouterWriterMessage,
};
use tokio::sync::mpsc;
use tracing::error;

use super::{
    request_error_into_runtime_error, response_event_into_transport_message,
    response_into_transport_message,
};
use crate::{
    error::{Result, RuntimeError},
    host::{request_supervisor::CompletionTrace, spawn_worker, RuntimeHost},
    loader::assembly_admission::ActiveAssemblyRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    pub(super) async fn spawn_request_inner(
        &self,
        request: RequestEnvelope,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        _registration: Option<&spawn_worker::SpawnWorkerRegistration>,
    ) {
        let selector = match request.ingress_selector.as_ref() {
            Some(selector) => selector,
            None => {
                let error = RuntimeError::Unsupported(
                    "request.start canonical ingress selector is required".to_string(),
                );
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        let route = match self.lookup_active_assembly_request_route(selector) {
            Ok(route) => route,
            Err(error) => {
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        let target = match route.request_target() {
            Ok(target) => target,
            Err(error) => {
                let error = RuntimeError::Decode(error.to_string());
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        let handles = match self.assembly_request_execution_handles(&route, &request, &sender) {
            Ok(handles) => handles,
            Err(error) => {
                self.emit_request_route_error(&request, &error);
                self.send_request_error_response(&request, &error, &sender);
                return;
            }
        };
        self.spawn_assembly_request(route, target, handles, request, sender)
            .await;
    }

    async fn spawn_assembly_request(
        &self,
        route: ActiveAssemblyRoute,
        target: request_runner::RuntimeAssemblyRequestTarget,
        handles: request_runner::AssemblyRequestExecutionHandles,
        request: RequestEnvelope,
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
        let host = self.clone();
        tokio::spawn(async move {
            // `route` is retained by this task so reload cannot drop the activation/context set
            // while cancellation, response encoding, or request supervision is still active.
            let _pinned_route = route;
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
        });
    }

    fn assembly_request_execution_handles(
        &self,
        route: &ActiveAssemblyRoute,
        request: &RequestEnvelope,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<request_runner::AssemblyRequestExecutionHandles> {
        let telemetry = self.assembly_request_telemetry_context(request, route);
        let eval_adapter = crate::eval_capability_adapter::assembly_request_eval_adapter(
            crate::eval_capability_adapter::RuntimeAssemblyRequestEvalAdapterInput {
                runtime_id: self.base_runtime_id.clone(),
                activation: Arc::clone(route.activation()),
                file_source: crate::capability_context::FileCapabilitySource::new(
                    self.file_runtime(),
                ),
                http_options: self.http_runtime_options.clone(),
                outbound_requests: Arc::clone(&self.outbound_requests),
                spawn_workers: Arc::clone(&self.spawn_workers),
                telemetry_context: Some(telemetry),
                router_sender: Some(sender.clone()),
                http_response_max_bytes: self.default_http_response_max_bytes,
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

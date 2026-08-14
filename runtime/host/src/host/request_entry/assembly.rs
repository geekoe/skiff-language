use std::sync::{Arc, Mutex};

use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{
    self as request_runner, BinaryHttpRequest, BinaryHttpRequestMetadata, BoundaryResponse,
    BytecodeRequestExecutionHandles, BytecodeRequestExecutionInput, HttpNameValue, RequestEnvelope,
    RequestError, RequestExecutionOwnerInventorySnapshot, RouterWriterMessage,
};
use skiff_runtime_transport::{
    protocol::{
        BytecodeRequestStartFrameHeader, BytecodeTaskRequestStartFrameHeader,
        BytecodeWebSocketConnectRequestStartFrameHeader,
        BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
    },
    response_mapper::OrdinaryResponseEvent,
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
        request_supervisor::{
            ActivationOutcome, CleanupPermit, CompletionTrace, RequestReservation,
            SupervisedRequest,
        },
        RuntimeHost,
    },
    loader::bytecode_admission::BytecodeRoute,
    telemetry::RequestTelemetryContext,
};

impl RuntimeHost {
    pub(super) async fn task_bytecode_http_request(
        &self,
        reservation: RequestReservation,
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
        let request_envelope = match bytecode_http_request_envelope(&route, &header, body) {
            Ok(envelope) => envelope,
            Err(error) => {
                error!(event = "runtime.http_adapter_error", request_id = %header.request_id, error = %error);
                self.send_http_gateway_admission_error(
                    &header.request_id,
                    &error.to_string(),
                    &sender,
                );
                return;
            }
        };
        let telemetry = bytecode_http_telemetry_context(self, &header, &route);
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
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let response_sink = Arc::new(HostHttpGatewayResponseSink::new(sender.clone()));
        let request_id = header.request_id.clone();
        let host = self.clone();
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
                    heap: None,
                },
            )
            .await;
            let owner_inventory = owner_inventory.into_snapshot();
            let cleanup_permit = host
                .finish_http_gateway_request(
                    &supervised_request,
                    &request_id,
                    result,
                    owner_inventory,
                    http_response_max_bytes,
                    &response_sink,
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

    pub(super) async fn task_bytecode_task_request(
        &self,
        reservation: RequestReservation,
        request: AdmittedBytecodeTaskRequest,
        http_response_max_bytes: usize,
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
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let request_id = header.request_id.clone();
        let host = self.clone();
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
                    heap: None,
                },
            )
            .await;
            let owner_inventory = owner_inventory.into_snapshot();
            let cleanup_permit = match result {
                Ok(response) => {
                    let permit = host
                        .request_supervisor
                        .complete_success(
                            &supervised_request,
                            owner_inventory,
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if send_transport_override_or_allow_candidate(
                        permit.as_ref(),
                        &request_id,
                        &sender,
                    ) {
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
                    permit
                }
                Err(error) => {
                    host.finish_direct_task_error(
                        &supervised_request,
                        request_id,
                        owner_inventory,
                        error,
                        &sender,
                    )
                    .await
                }
            };
            drop(retention);
            drop(execution_budget);
            drop(supervised_request);
            drop(route);
            if let Some(permit) = cleanup_permit {
                host.observe_bytecode_request_cleanup(permit);
            }
        });
    }

    pub(super) async fn task_bytecode_websocket_connect_request(
        &self,
        reservation: RequestReservation,
        request: AdmittedBytecodeWebSocketConnectRequest,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let AdmittedBytecodeWebSocketConnectRequest {
            route,
            header,
            target,
        } = request;
        let request_envelope = bytecode_websocket_connect_request_envelope(&route, &header);
        let telemetry = bytecode_websocket_connect_telemetry_context(self, &header, &route);
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
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let request_id = header.request_id.clone();
        let host = self.clone();
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
                    heap: None,
                },
            )
            .await;
            let owner_inventory = owner_inventory.into_snapshot();
            let mapped_error = match result {
                Ok(_) => RequestError::Unsupported(
                    "bytecode WebSocket connect response mapping is not supported; refusing legacy ActiveAssemblyRoute fallback"
                        .to_string(),
                ),
                Err(error) => error,
            };
            let cleanup_permit = host
                .finish_websocket_connect_error(
                    &supervised_request,
                    request_id,
                    owner_inventory,
                    mapped_error,
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

    pub(super) async fn task_bytecode_websocket_connection_closed_request(
        &self,
        reservation: RequestReservation,
        request: AdmittedBytecodeWebSocketConnectionClosedRequest,
        http_response_max_bytes: usize,
        _sender: mpsc::UnboundedSender<RouterWriterMessage>,
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
        let observer = reservation.observer().clone();
        let activation_key = reservation.key().clone();
        let supervised_request =
            match reservation.activate(&activation_key, &request_envelope, telemetry) {
                ActivationOutcome::Activated(request) => request,
                ActivationOutcome::RevokedByCancel | ActivationOutcome::RevokedBySessionStop => {
                    return
                }
                ActivationOutcome::Invalid => return,
            };
        route.publish_admission_observations();
        let cancellation = supervised_request.cancellation_token();
        let http_client =
            Some(self.bytecode_http_client_port(cancellation.clone(), http_response_max_bytes));
        let execution_budget = supervised_request.execution_budget();
        let handles = BytecodeRequestExecutionHandles {
            request_heap_limits: self.request_heap_limits(),
        };
        let host = self.clone();
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
                    heap: None,
                },
            )
            .await;
            let owner_inventory = owner_inventory.into_snapshot();
            let error = match result {
                Ok(_) => RequestError::Unsupported(
                    "bytecode WebSocket connection close response mapping is not supported; refusing legacy ActiveAssemblyRoute fallback"
                        .to_string(),
                ),
                Err(error) => error,
            };
            let cleanup_permit = host
                .finish_websocket_connection_closed_error(
                    &supervised_request,
                    owner_inventory,
                    error,
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

    async fn finish_http_gateway_request(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: &str,
        result: request_runner::RequestResult<BoundaryResponse>,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        http_response_max_bytes: usize,
        response_sink: &HostHttpGatewayResponseSink,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Option<CleanupPermit> {
        match result {
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
                    let permit = self
                        .request_supervisor
                        .complete_error(
                            supervised_request,
                            "request.error",
                            response_event
                                .response_error()
                                .expect("ordinary error event carries response error"),
                            owner_inventory,
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if allow_http_candidate_response(permit.as_ref(), request_id, response_sink) {
                        response_sink.send_terminal_response(request_id, response_event);
                    }
                    return permit;
                }

                let permit = self
                    .request_supervisor
                    .complete_success(
                        supervised_request,
                        owner_inventory,
                        CompletionTrace::RUNTIME,
                    )
                    .await;
                if !allow_http_candidate_response(permit.as_ref(), request_id, response_sink) {
                    return permit;
                }
                match response_into_transport_message(request_id.to_string(), response) {
                    Ok(Some(message)) => {
                        let _ = sender.send(message);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        error!(event = "runtime.response_encode_error", error = %error)
                    }
                }
                permit
            }
            Err(request_error) => {
                if request_error.is_cancellation_terminal() {
                    let permit = self
                        .request_supervisor
                        .complete_cancelled(
                            supervised_request,
                            owner_inventory,
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if allow_http_candidate_response(permit.as_ref(), request_id, response_sink) {
                        response_sink.cancel_without_response();
                    }
                    return permit;
                }
                if let Some(failure) = request_error.fixed_service_response_failure() {
                    error!(
                        event = "runtime.assembly_fixed_service_failure",
                        request_id,
                        trace_id = %failure.error().envelope().trace_id(),
                        error_id = %failure.error().envelope().error_id(),
                    );
                    let permit = self
                        .request_supervisor
                        .complete_fixed_service_failure(
                            supervised_request,
                            "request.error",
                            failure.error(),
                            owner_inventory,
                            CompletionTrace::RUNTIME,
                        )
                        .await;
                    if allow_http_candidate_response(permit.as_ref(), request_id, response_sink) {
                        response_sink.send_terminal_response(
                            request_id,
                            OrdinaryResponseEvent::FixedServiceFailure(failure),
                        );
                    }
                    return permit;
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
                let permit = self
                    .request_supervisor
                    .complete_error(
                        supervised_request,
                        "request.error",
                        &response_error,
                        owner_inventory,
                        CompletionTrace::RUNTIME,
                    )
                    .await;
                if allow_http_candidate_response(permit.as_ref(), request_id, response_sink) {
                    response_sink.send_terminal_response(request_id, response_event);
                }
                permit
            }
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
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        request_error: RequestError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Option<CleanupPermit> {
        if request_error.is_cancellation_terminal() {
            let permit = self
                .request_supervisor
                .complete_cancelled(
                    supervised_request,
                    owner_inventory,
                    CompletionTrace::RUNTIME,
                )
                .await;
            let _ =
                send_transport_override_or_allow_candidate(permit.as_ref(), &request_id, sender);
            return permit;
        }
        let response_event = OrdinaryResponseEvent::try_error(&request_error)
            .expect("cancellation was split before ordinary response mapping");
        let response_error = request_error
            .ordinary_response_error()
            .expect("cancellation was split before ordinary response mapping");
        let permit = self
            .request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                owner_inventory,
                CompletionTrace::RUNTIME,
            )
            .await;
        if send_transport_override_or_allow_candidate(permit.as_ref(), &request_id, sender) {
            if let Ok(message) = response_event_into_transport_message(request_id, response_event) {
                let _ = sender.send(message);
            }
        }
        permit
    }

    async fn finish_direct_task_error(
        &self,
        supervised_request: &SupervisedRequest,
        request_id: String,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        request_error: RequestError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Option<CleanupPermit> {
        if request_error.is_cancellation_terminal() {
            let permit = self
                .request_supervisor
                .complete_cancelled(
                    supervised_request,
                    owner_inventory,
                    CompletionTrace::RUNTIME,
                )
                .await;
            let _ =
                send_transport_override_or_allow_candidate(permit.as_ref(), &request_id, sender);
            return permit;
        }
        if let Some(failure) = request_error.fixed_service_response_failure() {
            error!(
                event = "runtime.assembly_task_fixed_service_failure",
                request_id,
                trace_id = %failure.error().envelope().trace_id(),
                error_id = %failure.error().envelope().error_id(),
            );
            let permit = self
                .request_supervisor
                .complete_fixed_service_failure(
                    supervised_request,
                    "request.error",
                    failure.error(),
                    owner_inventory,
                    CompletionTrace::RUNTIME,
                )
                .await;
            if send_transport_override_or_allow_candidate(permit.as_ref(), &request_id, sender) {
                if let Ok(message) = response_event_into_transport_message(
                    request_id,
                    OrdinaryResponseEvent::FixedServiceFailure(failure),
                ) {
                    let _ = sender.send(message);
                }
            }
            return permit;
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
        let permit = self
            .request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                owner_inventory,
                CompletionTrace::RUNTIME,
            )
            .await;
        if send_transport_override_or_allow_candidate(permit.as_ref(), &request_id, sender) {
            if let Ok(message) = response_event_into_transport_message(request_id, response_event) {
                let _ = sender.send(message);
            }
        }
        permit
    }

    async fn finish_websocket_connection_closed_error(
        &self,
        supervised_request: &SupervisedRequest,
        owner_inventory: RequestExecutionOwnerInventorySnapshot,
        request_error: RequestError,
    ) -> Option<CleanupPermit> {
        if request_error.is_cancellation_terminal() {
            return self
                .request_supervisor
                .complete_cancelled(
                    supervised_request,
                    owner_inventory,
                    CompletionTrace::RUNTIME,
                )
                .await;
        }
        let response_error = request_error
            .ordinary_response_error()
            .expect("cancellation was split before ordinary response mapping");
        self.request_supervisor
            .complete_error(
                supervised_request,
                "request.error",
                &response_error,
                owner_inventory,
                CompletionTrace::RUNTIME,
            )
            .await
    }
}

impl RuntimeHost {
    pub(super) fn observe_bytecode_request_cleanup(&self, permit: CleanupPermit) {
        permit.observe_cleanup();
    }
}

fn allow_http_candidate_response(
    permit: Option<&CleanupPermit>,
    request_id: &str,
    response_sink: &HostHttpGatewayResponseSink,
) -> bool {
    let Some(permit) = permit else {
        response_sink.cancel_without_response();
        return false;
    };
    if !permit.response_owned() {
        response_sink.cancel_without_response();
        return false;
    }
    let Some(error) = permit.response_override() else {
        return true;
    };
    let event = OrdinaryResponseEvent::Error(error);
    response_sink.send_terminal_response(request_id, event);
    false
}

fn send_transport_override_or_allow_candidate(
    permit: Option<&CleanupPermit>,
    request_id: &str,
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> bool {
    let Some(permit) = permit else {
        return false;
    };
    if !permit.response_owned() {
        return false;
    }
    let Some(error) = permit.response_override() else {
        return true;
    };
    let event = OrdinaryResponseEvent::Error(error);
    if let Ok(message) = response_event_into_transport_message(request_id.to_string(), event) {
        let _ = sender.send(message);
    }
    false
}

fn bytecode_http_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeRequestStartFrameHeader,
    body: Vec<u8>,
) -> anyhow::Result<RequestEnvelope> {
    let query = header
        .http_request
        .query
        .iter()
        .map(|item| HttpNameValue {
            name: item.name.clone(),
            value: item.value.clone(),
        })
        .collect::<Vec<_>>();
    let headers = header
        .http_request
        .headers
        .iter()
        .map(|item| HttpNameValue {
            name: item.name.clone(),
            value: item.value.clone(),
        })
        .collect::<Vec<_>>();
    let adapter = route.http_adapter()?;
    Ok(RequestEnvelope {
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
            protocol: IngressProtocol::Http,
            method: Some(header.routing.ingress.method.clone()),
            path: header.routing.ingress.path.clone(),
        }),
        binary_http: Some(BinaryHttpRequest {
            metadata: BinaryHttpRequestMetadata {
                method: header.http_request.method.clone(),
                url: header.http_request.url.clone(),
                path: header.http_request.path.clone(),
                query,
                headers,
            },
            body,
        }),
        http_adapter: Some(adapter),
        test_effects_enabled: header.test_effects_enabled || header.test_case_capability.is_some(),
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: bytecode_deadline_extra(header.deadline.as_ref()),
    })
}

fn bytecode_task_request_envelope(
    route: &BytecodeRoute,
    header: &BytecodeTaskRequestStartFrameHeader,
    payload: Vec<u8>,
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
        ingress_selector: None,
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: header.test_effects_enabled || header.test_case_capability.is_some(),
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
    context.target = Some(route.target_label());
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
    context.target = Some(route.target_label());
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
    context.target = Some(route.target_label());
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
    context.target = Some(route.target_label());
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
    terminal_settled: Mutex<bool>,
}

impl HostHttpGatewayResponseSink {
    fn new(sender: mpsc::UnboundedSender<RouterWriterMessage>) -> Self {
        Self {
            sender,
            terminal_settled: Mutex::new(false),
        }
    }

    fn send_terminal_response(&self, request_id: &str, event: OrdinaryResponseEvent) {
        let Ok(mut terminal_settled) = self.terminal_settled.lock() else {
            return;
        };
        if *terminal_settled {
            return;
        }
        *terminal_settled = true;
        if let Ok(message) = response_event_into_transport_message(request_id.to_string(), event) {
            let _ = self.sender.send(message);
        }
    }

    fn cancel_without_response(&self) {
        if let Ok(mut terminal_settled) = self.terminal_settled.lock() {
            *terminal_settled = true;
        }
    }
}

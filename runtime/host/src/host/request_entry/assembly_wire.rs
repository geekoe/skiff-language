use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyIdentity, DeploymentRevision, IngressProtocol, IngressSelector, ServiceDeploymentRef,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, DbCapabilitySource, DbProviderBuildInput, DbProviderConfig,
    ExecutionBudgetReason,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionCorrelation, BytecodeExecutionObserver,
};
use skiff_runtime_request::{
    execution_budget::admit_request_deadline, BytecodeRequestChildComposition, RequestError,
    RouterWriterMessage,
};
use skiff_runtime_transport::protocol::{
    BytecodeRequestDeadlineFrameHeader, BytecodeRequestIngressProtocol,
    BytecodeRequestStartFrameHeader, BytecodeRequestStartFrameWireHeader,
    BytecodeTaskRequestStartFrameHeader, BytecodeWebSocketConnectRequestStartFrameHeader,
    BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
    BytecodeWebSocketJsonRpcRequestStartFrameHeader,
};
use skiff_runtime_transport::response_mapper::OrdinaryResponseEvent;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;
use tracing::error;
use url::Url;

use super::{request_error_into_runtime_error, response_event_into_transport_message};
use crate::{
    error::{Result, RuntimeError},
    host::{
        request_supervisor::{RequestExecutionKey, RequestId, RouterSessionEpoch},
        router_session::ConnectionBootstrap,
        RuntimeHost,
    },
    loader::bytecode_admission::{BytecodeRoute, BytecodeRouteSelector},
};

pub(super) struct AdmittedBytecodeHttpRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: BytecodeRequestStartFrameHeader,
    pub(super) body: Vec<u8>,
    pub(super) target: DeploymentExecutionEntry,
    pub(super) db_source: Option<DbCapabilitySource>,
}

pub(super) struct AdmittedBytecodeWebSocketConnectRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: BytecodeWebSocketConnectRequestStartFrameHeader,
    pub(super) target: DeploymentExecutionEntry,
    pub(super) db_source: Option<DbCapabilitySource>,
}

pub(super) struct AdmittedBytecodeWebSocketConnectionClosedRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
    pub(super) target: DeploymentExecutionEntry,
    pub(super) db_source: Option<DbCapabilitySource>,
}

pub(super) struct AdmittedBytecodeWebSocketJsonRpcRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: BytecodeWebSocketJsonRpcRequestStartFrameHeader,
    pub(super) target: DeploymentExecutionEntry,
    pub(super) params: Vec<u8>,
    pub(super) db_source: Option<DbCapabilitySource>,
}

pub(super) struct AdmittedBytecodeTaskRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: BytecodeTaskRequestStartFrameHeader,
    pub(super) target: DeploymentExecutionEntry,
    pub(super) payload: Vec<u8>,
    pub(super) db_source: Option<DbCapabilitySource>,
}

enum AdmittedBytecodeRequest {
    Http(AdmittedBytecodeHttpRequest),
    WebSocketConnect(AdmittedBytecodeWebSocketConnectRequest),
    WebSocketConnectionClosed(AdmittedBytecodeWebSocketConnectionClosedRequest),
    WebSocketJsonRpc(AdmittedBytecodeWebSocketJsonRpcRequest),
    Task(AdmittedBytecodeTaskRequest),
}

impl RuntimeHost {
    pub(crate) async fn spawn_bytecode_request(
        &self,
        router_session: &RouterSessionEpoch,
        header: BytecodeRequestStartFrameWireHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let request_id = match &header {
            BytecodeRequestStartFrameWireHeader::Http(header) => header.request_id.clone(),
            BytecodeRequestStartFrameWireHeader::WebSocketConnect(header) => {
                header.request_id.clone()
            }
            BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
                header.request_id.clone()
            }
            BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
                header.request_id.clone()
            }
            BytecodeRequestStartFrameWireHeader::Task(header) => header.request_id.clone(),
        };
        if let Err(error) = admit_synchronous_http_lane(&header) {
            self.send_bytecode_wire_admission_error(&request_id, &error, &sender);
            return;
        }
        let request_id_typed = match RequestId::parse(request_id.clone()) {
            Ok(request_id) => request_id,
            Err(error) => {
                self.send_http_gateway_admission_error(&request_id, error, &sender);
                return;
            }
        };
        let request_key = RequestExecutionKey::new(router_session.clone(), request_id_typed);
        let admitted_deadline = match admit_request_deadline(&wire_deadline_extra(&header)) {
            Ok(deadline) => deadline,
            Err(error) => {
                self.send_http_gateway_admission_error(&request_id, error, &sender);
                return;
            }
        };
        if admitted_deadline.is_some_and(|deadline| std::time::Instant::now() >= deadline.at()) {
            self.send_bytecode_wire_admission_error(&request_id, &deadline_exceeded(), &sender);
            return;
        }
        let observer = BytecodeExecutionObserver::new(
            Arc::clone(&self.bytecode_execution_event_sink),
            BytecodeExecutionCorrelation {
                router_session_id: router_session.as_str().to_string(),
                request_id: request_id.clone(),
            },
        );
        let Some(reservation) =
            self.request_supervisor
                .reserve(request_key, observer.clone(), admitted_deadline)
        else {
            self.send_http_gateway_admission_error(
                &request_id,
                "duplicate active bytecode requestId",
                &sender,
            );
            return;
        };
        let build_id = wire_routing_build_id(&header);
        let was_loaded = if let Some(build_id) = build_id.as_deref() {
            self.bytecode_deployments.is_loaded_build_id(build_id).await
        } else {
            false
        };
        let result = match header {
            BytecodeRequestStartFrameWireHeader::Http(header) => {
                self.http_gateway_request_from_wire(header, body, bootstrap, &observer)
                    .await
            }
            BytecodeRequestStartFrameWireHeader::WebSocketConnect(header) => {
                self.websocket_connect_request_from_wire(header, body, bootstrap, &observer)
                    .await
            }
            BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
                self.websocket_connection_closed_request_from_wire(
                    header, body, bootstrap, &observer,
                )
                .await
            }
            BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
                self.websocket_jsonrpc_request_from_wire(header, body, bootstrap, &observer)
                    .await
            }
            BytecodeRequestStartFrameWireHeader::Task(header) => {
                self.task_request_from_wire(header, body, bootstrap, &observer)
                    .await
            }
        };
        if !was_loaded {
            let _ = self.queue_runtime_capabilities(sender.clone());
        }
        match result {
            Ok(AdmittedBytecodeRequest::Http(request)) => {
                self.task_bytecode_http_request(
                    reservation,
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedBytecodeRequest::WebSocketConnect(request)) => {
                self.task_bytecode_websocket_connect_request(
                    reservation,
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedBytecodeRequest::WebSocketConnectionClosed(request)) => {
                self.task_bytecode_websocket_connection_closed_request(
                    reservation,
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedBytecodeRequest::WebSocketJsonRpc(request)) => {
                self.task_bytecode_websocket_jsonrpc_request(
                    reservation,
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedBytecodeRequest::Task(request)) => {
                self.task_bytecode_task_request(
                    reservation,
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Err(runtime_error) => {
                drop(reservation);
                self.send_bytecode_wire_admission_error(&request_id, &runtime_error, &sender);
            }
        }
    }

    fn send_bytecode_wire_admission_error(
        &self,
        request_id: &str,
        runtime_error: &RuntimeError,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        error!(
            event = "runtime.assembly_wire_rejected",
            request_id,
            error = %runtime_error
        );
        let response_event = OrdinaryResponseEvent::try_error(runtime_error)
            .expect("wire admission rejection is ordinary");
        match response_event_into_transport_message(request_id.to_string(), response_event) {
            Ok(message) => {
                let _ = sender.send(message);
            }
            Err(encode_error) => {
                error!(event = "runtime.response_encode_error", error = %encode_error);
            }
        }
    }

    async fn resolve_bytecode_request_route(
        &self,
        deployment: &ServiceDeploymentRef,
        bootstrap: &ConnectionBootstrap,
        selector: BytecodeRouteSelector,
        observer: &BytecodeExecutionObserver,
    ) -> Result<Option<BytecodeRoute>> {
        let route = self
            .bytecode_deployments
            .route(
                deployment,
                bootstrap.resolver.store().root(),
                selector,
                observer,
            )
            .await
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        if route.is_none() {
            return Err(bytecode_required_error(deployment));
        }
        Ok(route)
    }

    fn db_source_for_route(
        &self,
        route: &BytecodeRoute,
        bootstrap: &ConnectionBootstrap,
    ) -> Option<DbCapabilitySource> {
        let service_db = self.db_service_db()?;
        let config = DbProviderConfig::mongo(service_db.mongo_url.clone()).ok()?;
        self.db_provider
            .build(DbProviderBuildInput {
                environment: bootstrap.activation.profile.clone(),
                service_id: route.deployment().service_id.clone(),
                config,
                runtime_program_db: Vec::new(),
            })
            .ok()
    }

    async fn websocket_connect_request_from_wire(
        &self,
        mut header: BytecodeWebSocketConnectRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        observer: &BytecodeExecutionObserver,
    ) -> Result<AdmittedBytecodeRequest> {
        validate_websocket_connect_header(&header, &body)?;
        let route = self
            .resolve_bytecode_request_route(
                &header.routing.deployment,
                bootstrap,
                BytecodeRouteSelector::Gateway {
                    ingress: IngressSelector {
                        protocol: IngressProtocol::WebSocket,
                        method: None,
                        path: header.routing.ingress.path.clone(),
                    },
                    gateway_entry_identity: header.routing.gateway_entry_identity.clone(),
                },
                observer,
            )
            .await?
            .expect("bytecode route is required after resolution");
        validate_bytecode_build_id(
            &header.routing.deployment,
            header.routing.build_id.as_deref(),
            &header.request_id,
        )?;
        header.deadline =
            effective_request_deadline(header.deadline.as_ref(), "WebSocket connect")?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let target = bytecode_route_target(&route)?;
        let db_source = self.db_source_for_route(&route, bootstrap);
        Ok(AdmittedBytecodeRequest::WebSocketConnect(
            AdmittedBytecodeWebSocketConnectRequest {
                route,
                header,
                target,
                db_source,
            },
        ))
    }

    async fn websocket_connection_closed_request_from_wire(
        &self,
        mut header: BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        observer: &BytecodeExecutionObserver,
    ) -> Result<AdmittedBytecodeRequest> {
        validate_websocket_connection_closed_header(&header, &body)?;
        let route = self
            .resolve_bytecode_request_route(
                &header.routing.deployment,
                bootstrap,
                BytecodeRouteSelector::Gateway {
                    ingress: IngressSelector {
                        protocol: IngressProtocol::WebSocket,
                        method: None,
                        path: header.routing.ingress.path.clone(),
                    },
                    gateway_entry_identity: header.routing.gateway_entry_identity.clone(),
                },
                observer,
            )
            .await?
            .expect("bytecode route is required after resolution");
        validate_bytecode_build_id(
            &header.routing.deployment,
            header.routing.build_id.as_deref(),
            &header.request_id,
        )?;
        header.deadline =
            effective_request_deadline(header.deadline.as_ref(), "WebSocket connection close")?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let target = bytecode_route_target(&route)?;
        let db_source = self.db_source_for_route(&route, bootstrap);
        Ok(AdmittedBytecodeRequest::WebSocketConnectionClosed(
            AdmittedBytecodeWebSocketConnectionClosedRequest {
                route,
                header,
                target,
                db_source,
            },
        ))
    }

    async fn http_gateway_request_from_wire(
        &self,
        mut header: BytecodeRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        observer: &BytecodeExecutionObserver,
    ) -> Result<AdmittedBytecodeRequest> {
        let route = self
            .resolve_bytecode_request_route(
                &header.routing.deployment,
                bootstrap,
                BytecodeRouteSelector::Gateway {
                    ingress: IngressSelector {
                        protocol: IngressProtocol::Http,
                        method: Some(header.routing.ingress.method.clone()),
                        path: header.routing.ingress.path.clone(),
                    },
                    gateway_entry_identity: header.routing.gateway_entry_identity.clone(),
                },
                observer,
            )
            .await?
            .expect("bytecode route is required after resolution");
        validate_bytecode_build_id(
            &header.routing.deployment,
            header.routing.build_id.as_deref(),
            &header.request_id,
        )?;
        let target = bytecode_route_target(&route)?;
        header.deadline = effective_deadline(&header)?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let db_source = self.db_source_for_route(&route, bootstrap);
        Ok(AdmittedBytecodeRequest::Http(AdmittedBytecodeHttpRequest {
            route,
            header,
            body,
            target,
            db_source,
        }))
    }

    async fn task_request_from_wire(
        &self,
        mut header: BytecodeTaskRequestStartFrameHeader,
        payload: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        observer: &BytecodeExecutionObserver,
    ) -> Result<AdmittedBytecodeRequest> {
        validate_task_header(&header, &payload)?;
        let deployment = &header.routing.deployment;
        if let Some(build_id) = &header.routing.build_id {
            if build_id != deployment.deployment_artifact_identity.as_str() {
                return Err(RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: "task routing buildId does not match its exact deployment".to_string(),
                });
            }
        }
        header.deadline =
            effective_request_deadline(header.deadline.as_ref(), "durable task request")?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let route = self
            .resolve_bytecode_request_route(
                &header.routing.deployment,
                bootstrap,
                BytecodeRouteSelector::PackageFunction {
                    target: header.invocation.target.clone(),
                },
                observer,
            )
            .await?
            .expect("bytecode route is required after resolution");
        let target = bytecode_route_target(&route)?;
        let db_source = self.db_source_for_route(&route, bootstrap);
        Ok(AdmittedBytecodeRequest::Task(AdmittedBytecodeTaskRequest {
            route,
            header,
            target,
            payload,
            db_source,
        }))
    }

    async fn websocket_jsonrpc_request_from_wire(
        &self,
        mut header: BytecodeWebSocketJsonRpcRequestStartFrameHeader,
        params: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        observer: &BytecodeExecutionObserver,
    ) -> Result<AdmittedBytecodeRequest> {
        validate_websocket_jsonrpc_header(&header, &params)?;
        let route = self
            .resolve_bytecode_request_route(
                &header.routing.deployment,
                bootstrap,
                BytecodeRouteSelector::Gateway {
                    ingress: IngressSelector {
                        protocol: IngressProtocol::WebSocket,
                        method: Some(header.routing.ingress.method.clone()),
                        path: header.routing.ingress.path.clone(),
                    },
                    gateway_entry_identity: header.routing.gateway_entry_identity.clone(),
                },
                observer,
            )
            .await?
            .expect("bytecode route is required after resolution");
        validate_bytecode_build_id(
            &header.routing.deployment,
            header.routing.build_id.as_deref(),
            &header.request_id,
        )?;
        header.deadline =
            effective_request_deadline(header.deadline.as_ref(), "WebSocket JSON-RPC")?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let target = bytecode_route_target(&route)?;
        let db_source = self.db_source_for_route(&route, bootstrap);
        Ok(AdmittedBytecodeRequest::WebSocketJsonRpc(
            AdmittedBytecodeWebSocketJsonRpcRequest {
                route,
                header,
                target,
                params,
                db_source,
            },
        ))
    }
}

fn bytecode_route_target(route: &BytecodeRoute) -> Result<DeploymentExecutionEntry> {
    route
        .execution_entry()
        .map_err(|error| RuntimeError::Decode(error.to_string()))
}

pub(super) fn production_bytecode_request_child_composition(
    host: &RuntimeHost,
    image: &DeploymentExecutionImage,
    db_source: Option<&DbCapabilitySource>,
    request_id: &str,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    activation_identity: Option<ActivationIdentityControl>,
) -> BytecodeRequestChildComposition {
    crate::host::bytecode_capability_adapter::bytecode_request_child_composition(
        host,
        image,
        db_source,
        request_id,
        sender,
        activation_identity,
    )
}

pub(super) fn request_activation_identity(
    assembly_identity: Option<&AssemblyIdentity>,
    assembly_generation: Option<u64>,
    deployment_revision: &DeploymentRevision,
    runtime_replica_id: &str,
) -> Option<ActivationIdentityControl> {
    Some(ActivationIdentityControl {
        assembly_identity: assembly_identity?.clone(),
        generation: assembly_generation?,
        runtime_replica_id: runtime_replica_id.to_string(),
        deployment_revision: deployment_revision.clone(),
    })
}

fn bytecode_required_error(deployment: &ServiceDeploymentRef) -> RuntimeError {
    RuntimeError::Protocol {
        target: deployment.deployment_artifact_identity.as_str().to_string(),
        message: "bytecode is required for this deployment; legacy assembly routes are disabled"
            .to_string(),
    }
}

fn validate_bytecode_build_id(
    deployment: &ServiceDeploymentRef,
    build_id: Option<&str>,
    target: &str,
) -> Result<()> {
    if build_id.is_some_and(|build_id| build_id != deployment.deployment_artifact_identity.as_str())
    {
        return Err(RuntimeError::Protocol {
            target: target.to_string(),
            message: "bytecode routing buildId does not match its exact deployment".to_string(),
        });
    }
    Ok(())
}

fn wire_routing_build_id(header: &BytecodeRequestStartFrameWireHeader) -> Option<String> {
    let deployment = match header {
        BytecodeRequestStartFrameWireHeader::Http(header) => &header.routing.deployment,
        BytecodeRequestStartFrameWireHeader::WebSocketConnect(header) => &header.routing.deployment,
        BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
            &header.routing.deployment
        }
        BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(header) => &header.routing.deployment,
        BytecodeRequestStartFrameWireHeader::Task(header) => &header.routing.deployment,
    };
    Some(deployment.deployment_artifact_identity.as_str().to_string())
}

fn admit_synchronous_http_lane(header: &BytecodeRequestStartFrameWireHeader) -> Result<()> {
    match header {
        BytecodeRequestStartFrameWireHeader::Http(header) => validate_http_header(header),
        BytecodeRequestStartFrameWireHeader::WebSocketConnect(_)
        | BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(_)
        | BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(_) => Err(
            RuntimeError::Unsupported(
                "bytecode request admission supports only exact HTTP gateway requests; the WebSocket request lane is disabled"
                    .to_string(),
            ),
        ),
        BytecodeRequestStartFrameWireHeader::Task(_) => Ok(()),
    }
}

fn validate_http_header(header: &BytecodeRequestStartFrameHeader) -> Result<()> {
    if header.request_id.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical request.start requestId must be non-empty".to_string(),
        ));
    }
    if !matches!(header.mode.as_str(), "unary" | "serverStream") {
        return Err(RuntimeError::Unsupported(format!(
            "bytecode HTTP ingress supports only unary or serverStream request.start, got {}",
            header.mode
        )));
    }
    if header.caller.kind != "gateway" {
        return Err(RuntimeError::Unsupported(
            "canonical HTTP gateway request requires caller.kind gateway".to_string(),
        ));
    }
    if header.routing.ingress.protocol != BytecodeRequestIngressProtocol::Http {
        return Err(RuntimeError::Unsupported(
            "Bytecode request bridge accepts only canonical HTTP gateway requests".to_string(),
        ));
    }
    if header.test_effects_enabled != header.test_case_capability.is_some() {
        return Err(RuntimeError::Decode(
            "canonical HTTP testEffectsEnabled must be true exactly when testCaseCapability is present"
                .to_string(),
        ));
    }
    if header.test_case_parent_request_id.is_some() && header.test_case_capability.is_none() {
        return Err(RuntimeError::Decode(
            "canonical HTTP testCaseParentRequestId requires testCaseCapability".to_string(),
        ));
    }
    if header.client_session.is_some() {
        return Err(RuntimeError::Unsupported(
            "bytecode request admission supports only the synchronous HTTP gateway lane; client-session requests are disabled"
                .to_string(),
        ));
    }
    if header.test_case_parent_request_id.is_some() {
        return Err(RuntimeError::Unsupported(
            "bytecode request admission supports only the synchronous HTTP gateway lane; child requests are disabled"
                .to_string(),
        ));
    }
    if header.test_effects_enabled {
        return Err(RuntimeError::Unsupported(
            "bytecode request admission supports only the synchronous HTTP gateway lane; host test-effect requests are disabled"
                .to_string(),
        ));
    }
    let ingress = &header.routing.ingress;
    let request = &header.http_request;
    if request.method != ingress.method || request.path != ingress.path {
        return Err(RuntimeError::Decode(
            "httpRequest method/path does not match canonical routing ingress".to_string(),
        ));
    }
    let url = Url::parse(&request.url).map_err(|error| {
        RuntimeError::Decode(format!("canonical httpRequest URL is invalid: {error}"))
    })?;
    if url.scheme() != "http"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.path() != ingress.path
    {
        return Err(RuntimeError::Decode(
            "httpRequest URL path does not match canonical routing ingress".to_string(),
        ));
    }
    Ok(())
}

fn validate_websocket_connect_header(
    header: &BytecodeWebSocketConnectRequestStartFrameHeader,
    body: &[u8],
) -> Result<()> {
    if header.request_id.is_empty() || header.caller.kind != "gateway" || header.mode != "unary" {
        return Err(RuntimeError::Decode(
            "canonical WebSocket connect requires a non-empty requestId, gateway caller and unary mode"
                .to_string(),
        ));
    }
    if !body.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical WebSocket connect request payload must be empty".to_string(),
        ));
    }
    let ingress = &header.routing.ingress;
    let request = &header.websocket_connect;
    if request.gateway_entry_identity != header.routing.gateway_entry_identity {
        return Err(RuntimeError::Decode(
            "websocketConnect gateway identity does not match routing".to_string(),
        ));
    }
    let url = Url::parse(&request.url).map_err(|error| {
        RuntimeError::Decode(format!(
            "canonical websocketConnect URL is invalid: {error}"
        ))
    })?;
    if !matches!(url.scheme(), "ws" | "wss")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str().is_none()
        || url.path() != ingress.path
    {
        return Err(RuntimeError::Decode(
            "websocketConnect URL path does not match canonical routing ingress".to_string(),
        ));
    }
    Ok(())
}

fn validate_websocket_connection_closed_header(
    header: &BytecodeWebSocketConnectionClosedRequestStartFrameHeader,
    body: &[u8],
) -> Result<()> {
    if header.request_id.is_empty() || header.caller.kind != "gateway" || header.mode != "unary" {
        return Err(RuntimeError::Decode(
            "canonical WebSocket connection close requires a non-empty requestId, gateway caller and unary mode"
                .to_string(),
        ));
    }
    if !body.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical WebSocket connection close request payload must be empty".to_string(),
        ));
    }
    let request = &header.websocket_connection_closed;
    if request.gateway_entry_identity != header.routing.gateway_entry_identity {
        return Err(RuntimeError::Decode(
            "websocketConnectionClosed gateway identity does not match routing".to_string(),
        ));
    }
    Ok(())
}

fn validate_websocket_jsonrpc_header(
    header: &BytecodeWebSocketJsonRpcRequestStartFrameHeader,
    params: &[u8],
) -> Result<()> {
    if header.request_id.is_empty() || header.caller.kind != "gateway" || header.mode != "unary" {
        return Err(RuntimeError::Decode(
            "canonical WebSocket JSON-RPC requires a non-empty requestId, gateway caller and unary mode"
                .to_string(),
        ));
    }
    if params.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical WebSocket JSON-RPC params payload must be present".to_string(),
        ));
    }
    if header.websocket_json_rpc.gateway_entry_identity != header.routing.gateway_entry_identity {
        return Err(RuntimeError::Decode(
            "websocketJsonRpc gateway identity does not match routing".to_string(),
        ));
    }
    Ok(())
}

fn validate_task_header(
    header: &BytecodeTaskRequestStartFrameHeader,
    payload: &[u8],
) -> Result<()> {
    if header.request_id.is_empty()
        || header.mode != "unary"
        || header.caller.kind != "service"
        || header.invocation.kind != "task"
        || header.invocation.target_kind != "function"
        || header.invocation.target.is_empty()
    {
        return Err(RuntimeError::Decode(
            "canonical task requires a non-empty requestId, unary mode, service caller and function target"
                .to_string(),
        ));
    }
    if payload.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical task recoverable args payload must be present".to_string(),
        ));
    }
    if header.test_effects_enabled != header.test_case_capability.is_some() {
        return Err(RuntimeError::Decode(
            "canonical task testEffectsEnabled must be true exactly when testCaseCapability is present"
                .to_string(),
        ));
    }
    Ok(())
}

fn effective_deadline(
    header: &BytecodeRequestStartFrameHeader,
) -> Result<Option<BytecodeRequestDeadlineFrameHeader>> {
    effective_request_deadline(header.deadline.as_ref(), "HTTP gateway")
}

fn wire_deadline_extra(
    header: &BytecodeRequestStartFrameWireHeader,
) -> serde_json::Map<String, serde_json::Value> {
    let deadline = match header {
        BytecodeRequestStartFrameWireHeader::Http(header) => header.deadline.as_ref(),
        BytecodeRequestStartFrameWireHeader::WebSocketConnect(header) => header.deadline.as_ref(),
        BytecodeRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
            header.deadline.as_ref()
        }
        BytecodeRequestStartFrameWireHeader::WebSocketJsonRpc(header) => header.deadline.as_ref(),
        BytecodeRequestStartFrameWireHeader::Task(header) => header.deadline.as_ref(),
    };
    let mut extra = serde_json::Map::new();
    if let Some(deadline) = deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline)
                .expect("typed bytecode request deadline remains serializable"),
        );
    }
    extra
}

fn effective_request_deadline(
    deadline: Option<&BytecodeRequestDeadlineFrameHeader>,
    request_kind: &str,
) -> Result<Option<BytecodeRequestDeadlineFrameHeader>> {
    let wall_now = OffsetDateTime::now_utc();
    let mut candidates = Vec::new();
    if let Some(deadline) = deadline {
        candidates.push(deadline.timeout_ms);
        let expires_at = OffsetDateTime::parse(&deadline.expires_at, &Rfc3339).map_err(|_| {
            RuntimeError::Decode(format!(
                "canonical {request_kind} deadline expiresAt must be valid RFC3339"
            ))
        })?;
        let remaining_ms = if expires_at <= wall_now {
            0
        } else {
            u64::try_from((expires_at - wall_now).whole_milliseconds()).unwrap_or(u64::MAX)
        };
        candidates.push(remaining_ms);
    }
    let Some(timeout_ms) = candidates.into_iter().min() else {
        return Ok(None);
    };
    let timeout_i64 = i64::try_from(timeout_ms).map_err(|_| {
        RuntimeError::Decode(format!(
            "{request_kind} deadline is not representable by the Host"
        ))
    })?;
    let expires_at = wall_now
        .checked_add(time::Duration::milliseconds(timeout_i64))
        .ok_or_else(|| {
            RuntimeError::Decode(format!(
                "{request_kind} deadline is not representable by the Host"
            ))
        })?
        .format(&Rfc3339)
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    Ok(Some(BytecodeRequestDeadlineFrameHeader {
        timeout_ms,
        expires_at,
    }))
}

fn deadline_exceeded() -> RuntimeError {
    request_error_into_runtime_error(RequestError::ExecutionBudgetExceeded {
        reason: ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 0,
        limit: None,
        elapsed_ms: 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_activation_identity_requires_exact_routing_facts() {
        let assembly_identity = AssemblyIdentity::new(
            "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let deployment_revision = DeploymentRevision::new("revision:request");
        let identity = request_activation_identity(
            Some(&assembly_identity),
            Some(7),
            &deployment_revision,
            "runtime-1",
        )
        .expect("exact routing facts must project an activation identity");

        assert_eq!(identity.assembly_identity, assembly_identity);
        assert_eq!(identity.generation, 7);
        assert_eq!(identity.runtime_replica_id, "runtime-1");
        assert_eq!(identity.deployment_revision, deployment_revision);

        assert!(
            request_activation_identity(None, Some(7), &deployment_revision, "runtime-1").is_none(),
            "missing routing assembly identity must fail closed"
        );
        assert!(
            request_activation_identity(
                Some(&assembly_identity),
                None,
                &deployment_revision,
                "runtime-1"
            )
            .is_none(),
            "missing routing assembly generation must fail closed"
        );
    }
}

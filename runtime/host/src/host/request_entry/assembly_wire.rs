#[cfg(test)]
use std::sync::Arc;

use skiff_artifact_model::ServiceDeploymentRef;
#[cfg(test)]
use skiff_artifact_model::{
    GatewayAdapterKind, GatewayDispatchMode, GatewayProtocolSurface, GatewayWebSocketRpcProfile,
    IngressProtocol, IngressSelector, ServiceIngressKey,
};
#[cfg(test)]
use skiff_runtime_activation::ActivationContext;
#[cfg(test)]
use skiff_runtime_capability_context::DbCapabilitySource;
use skiff_runtime_capability_context::ExecutionBudgetReason;
#[cfg(test)]
use skiff_runtime_eval::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};
#[cfg(test)]
use skiff_runtime_linked_program::AssemblyExecutionImage;
use skiff_runtime_request::{
    BinaryHttpRequestMetadata, BytecodeRequestTarget, HttpNameValue, RequestError,
    RouterWriterMessage,
};
#[cfg(test)]
use skiff_runtime_request::{
    RuntimeAssemblyTaskTarget, RuntimeAssemblyWebSocketJsonRpcTarget, RuntimeGatewayIngressPin,
    RuntimeHttpGatewayRequest, RuntimeTaskRequest, RuntimeWebSocketConnectIngress,
    RuntimeWebSocketConnectionClosedIngress,
};
use skiff_runtime_transport::response_mapper::OrdinaryResponseEvent;
#[cfg(test)]
use skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyWebSocketJsonRpcProfile;
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressProtocol,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestStartFrameWireHeader,
    RuntimeAssemblyTaskRequestStartFrameHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;
use tracing::error;
use url::Url;

use super::{request_error_into_runtime_error, response_event_into_transport_message};
#[cfg(test)]
use crate::loader::assembly_admission::ActiveAssemblyRoute;
use crate::{
    error::{Result, RuntimeError},
    host::{router_session::ConnectionBootstrap, RuntimeHost},
    loader::bytecode_admission::BytecodeRoute,
};

#[cfg(test)]
pub(super) struct AdmittedHttpGatewayRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyRequestStartFrameHeader,
    pub(super) request: RuntimeHttpGatewayRequest,
}

pub(super) struct AdmittedBytecodeHttpRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: RuntimeAssemblyRequestStartFrameHeader,
    pub(super) body: Vec<u8>,
    pub(super) target: BytecodeRequestTarget,
}

#[cfg(test)]
pub(super) struct AdmittedWebSocketConnectRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    pub(super) request: RuntimeWebSocketConnectIngress,
}

pub(super) struct AdmittedBytecodeWebSocketConnectRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    pub(super) target: BytecodeRequestTarget,
}

#[cfg(test)]
pub(super) struct AdmittedWebSocketConnectionClosedRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
    pub(super) request: RuntimeWebSocketConnectionClosedIngress,
}

pub(super) struct AdmittedBytecodeWebSocketConnectionClosedRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
    pub(super) target: BytecodeRequestTarget,
}

/// Per-request WebSocket JSON-RPC resolution: the physical WebSocket route is
/// resolved from the current assembly state (lazy-loading its deployment on
/// demand), and the method capability route plus execution target derive from
/// that exact physical route. No connection-scoped pin is retained.
#[derive(Debug)]
#[cfg(test)]
pub(super) struct ResolvedWebSocketJsonRpcExecution {
    pub(super) target: RuntimeAssemblyWebSocketJsonRpcTarget,
    pub(super) method_route: ActiveAssemblyRoute,
}

#[cfg(test)]
pub(super) struct AdmittedWebSocketJsonRpcRequest {
    pub(super) resolved: ResolvedWebSocketJsonRpcExecution,
    pub(super) header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    pub(super) params: Vec<u8>,
}

pub(super) struct AdmittedBytecodeWebSocketJsonRpcRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    pub(super) target: BytecodeRequestTarget,
    pub(super) params: Vec<u8>,
}

#[cfg(test)]
pub(super) struct AdmittedTaskRequest {
    pub(super) header: RuntimeAssemblyTaskRequestStartFrameHeader,
    pub(super) request: RuntimeTaskRequest,
    pub(super) target: RuntimeAssemblyTaskTarget,
    pub(super) activation: Arc<ActivationContext>,
    pub(super) execution_image: Arc<AssemblyExecutionImage>,
    pub(super) contexts: Arc<crate::loader::active_assembly_context::ActiveAssemblyContextSet>,
    pub(super) config_views: Arc<crate::loader::config_snapshot::ActivationConfigViews>,
    pub(super) db_source: DbCapabilitySource,
    pub(super) service_protocol_identity: String,
}

pub(super) struct AdmittedBytecodeTaskRequest {
    pub(super) route: BytecodeRoute,
    pub(super) header: RuntimeAssemblyTaskRequestStartFrameHeader,
    pub(super) target: BytecodeRequestTarget,
    pub(super) payload: Vec<u8>,
}

impl RuntimeHost {
    pub(crate) async fn spawn_runtime_assembly_request(
        &self,
        router_session_id: &str,
        header: RuntimeAssemblyRequestStartFrameWireHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let request_id = match &header {
            RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => header.request_id.clone(),
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => {
                header.request_id.clone()
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
                header.request_id.clone()
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
                header.request_id.clone()
            }
            RuntimeAssemblyRequestStartFrameWireHeader::Task(header) => header.request_id.clone(),
        };
        // A request whose build id was not loaded yet may trigger the lazy-load
        // path; refresh the router's capability view when that happens.
        let build_id = wire_routing_build_id(&header);
        #[cfg(test)]
        let was_loaded = if let Some(build_id) = build_id.as_deref() {
            self.assembly_admission.is_loaded(build_id)
                || self.bytecode_deployments.is_loaded_build_id(build_id).await
        } else {
            false
        };
        #[cfg(not(test))]
        let was_loaded = if let Some(build_id) = build_id.as_deref() {
            self.bytecode_deployments.is_loaded_build_id(build_id).await
        } else {
            false
        };
        let result = match header {
            RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => {
                self.http_gateway_request_from_wire(header, body, bootstrap)
                    .await
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => {
                self.websocket_connect_request_from_wire(header, body, bootstrap)
                    .await
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
                self.websocket_connection_closed_request_from_wire(header, body, bootstrap)
                    .await
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
                self.websocket_jsonrpc_request_from_wire(header, body, bootstrap)
                    .await
            }
            RuntimeAssemblyRequestStartFrameWireHeader::Task(header) => {
                self.task_request_from_wire(header, body, bootstrap).await
            }
        };
        if !was_loaded {
            let _ = self.queue_runtime_capabilities(sender.clone());
        }
        match result {
            #[cfg(test)]
            Ok(AdmittedRuntimeAssemblyRequest::Http(request)) => {
                self.task_request_on_active_assembly_route(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::BytecodeHttp(request)) => {
                self.task_bytecode_http_request(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            #[cfg(test)]
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnect(request)) => {
                self.task_websocket_connect_on_active_assembly_route(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::BytecodeWebSocketConnect(request)) => {
                self.task_bytecode_websocket_connect_request(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            #[cfg(test)]
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnectionClosed(request)) => {
                self.task_websocket_connection_closed_on_active_assembly_route(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::BytecodeWebSocketConnectionClosed(request)) => {
                self.task_bytecode_websocket_connection_closed_request(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            #[cfg(test)]
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketJsonRpc(request)) => {
                self.task_websocket_jsonrpc_on_resolved_route(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::BytecodeWebSocketJsonRpc(request)) => {
                self.task_bytecode_websocket_jsonrpc_request(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            #[cfg(test)]
            Ok(AdmittedRuntimeAssemblyRequest::Task(request)) => {
                self.task_direct_request_on_active_assembly(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::BytecodeTask(request)) => {
                self.task_bytecode_task_request(
                    router_session_id.to_string(),
                    request,
                    bootstrap.max_response_bytes,
                    sender,
                )
                .await
            }
            Err(runtime_error) => {
                error!(
                    event = "runtime.assembly_wire_rejected",
                    request_id,
                    error = %runtime_error
                );
                let response_event = OrdinaryResponseEvent::try_error(&runtime_error)
                    .expect("wire admission rejection is ordinary");
                match response_event_into_transport_message(request_id, response_event) {
                    Ok(message) => {
                        let _ = sender.send(message);
                    }
                    Err(encode_error) => {
                        error!(event = "runtime.response_encode_error", error = %encode_error);
                    }
                }
            }
        }
    }

    /// Resolves the canonical ingress route through the loaded deployment
    /// registry, lazy-loading the deployment under its per-buildId critical
    /// section when it is not loaded yet.
    #[cfg(test)]
    pub(crate) async fn resolve_active_assembly_request_route(
        &self,
        key: &ServiceIngressKey,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<ActiveAssemblyRoute> {
        if self.bytecode_only {
            return Err(bytecode_required_error(&key.deployment));
        }
        self.assembly_admission
            .route_or_lazy_load(
                key,
                &bootstrap.resolver,
                Some(&bootstrap.service_db),
                bootstrap.activation.profile.as_str(),
                Some(bootstrap.resolver.store().root()),
            )
            .await
            .map_err(|error| RuntimeError::Decode(error.to_string()))
    }

    async fn resolve_bytecode_request_route(
        &self,
        deployment: &ServiceDeploymentRef,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<Option<BytecodeRoute>> {
        let route = self
            .bytecode_deployments
            .route(deployment, bootstrap.resolver.store().root())
            .await
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
        if route.is_none() && self.bytecode_only {
            return Err(bytecode_required_error(deployment));
        }
        Ok(route)
    }

    async fn websocket_connect_request_from_wire(
        &self,
        mut header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<AdmittedRuntimeAssemblyRequest> {
        validate_websocket_connect_header(&header, &body)?;
        if let Some(bytecode_route) = self
            .resolve_bytecode_request_route(&header.routing.deployment, bootstrap)
            .await?
        {
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
            let target = bytecode_route
                .request_target()
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            return Ok(AdmittedRuntimeAssemblyRequest::BytecodeWebSocketConnect(
                AdmittedBytecodeWebSocketConnectRequest {
                    route: bytecode_route,
                    header,
                    target,
                },
            ));
        }
        #[cfg(any(test))]
        {
            let selector = websocket_connect_ingress_selector(&header);
            let key = ServiceIngressKey {
                deployment: header.routing.deployment.clone(),
                selector: selector.clone(),
            };
            let route = self
                .resolve_active_assembly_request_route(&key, bootstrap)
                .await?;
            validate_websocket_connect_route(&header, &selector, &route)?;
            if route.entry().optional_handler().is_none()
                && !route
                    .has_websocket_jsonrpc_methods()
                    .map_err(|error| RuntimeError::Decode(error.to_string()))?
            {
                return Err(RuntimeError::Protocol {
                    target: route.gateway_entry_key().as_str().to_string(),
                    message: "Runtime refuses WebSocket connect dispatch for a path-only entry"
                        .to_string(),
                });
            }
            let request = websocket_connect_ingress_from_wire(&route, &header);
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnect(
                AdmittedWebSocketConnectRequest {
                    route,
                    header,
                    request,
                },
            ))
        }
        #[cfg(not(test))]
        {
            Err(bytecode_required_error(&header.routing.deployment))
        }
    }

    async fn websocket_connection_closed_request_from_wire(
        &self,
        mut header: RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<AdmittedRuntimeAssemblyRequest> {
        validate_websocket_connection_closed_header(&header, &body)?;
        if let Some(bytecode_route) = self
            .resolve_bytecode_request_route(&header.routing.deployment, bootstrap)
            .await?
        {
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
            let target = bytecode_route
                .request_target()
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            return Ok(
                AdmittedRuntimeAssemblyRequest::BytecodeWebSocketConnectionClosed(
                    AdmittedBytecodeWebSocketConnectionClosedRequest {
                        route: bytecode_route,
                        header,
                        target,
                    },
                ),
            );
        }
        #[cfg(any(test))]
        {
            let selector = websocket_connection_closed_ingress_selector(&header);
            let key = ServiceIngressKey {
                deployment: header.routing.deployment.clone(),
                selector: selector.clone(),
            };
            let route = self
                .resolve_active_assembly_request_route(&key, bootstrap)
                .await?;
            validate_websocket_connection_closed_route(&header, &selector, &route)?;
            if route.entry().close_handler().is_none() {
                return Err(RuntimeError::Decode(
                    "connection close handler is not declared".to_string(),
                ));
            }
            let request = websocket_connection_closed_ingress_from_wire(&route, &header);
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnectionClosed(
                AdmittedWebSocketConnectionClosedRequest {
                    route,
                    header,
                    request,
                },
            ))
        }
        #[cfg(not(test))]
        {
            Err(bytecode_required_error(&header.routing.deployment))
        }
    }

    async fn http_gateway_request_from_wire(
        &self,
        mut header: RuntimeAssemblyRequestStartFrameHeader,
        body: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<AdmittedRuntimeAssemblyRequest> {
        validate_http_header(&header)?;
        if let Some(bytecode_route) = self
            .resolve_bytecode_request_route(&header.routing.deployment, bootstrap)
            .await?
        {
            validate_bytecode_build_id(
                &header.routing.deployment,
                header.routing.build_id.as_deref(),
                &header.request_id,
            )?;
            let target = bytecode_route
                .request_target()
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            header.deadline = effective_deadline(&header)?;
            if header
                .deadline
                .as_ref()
                .is_some_and(|deadline| deadline.timeout_ms == 0)
            {
                return Err(deadline_exceeded());
            }
            return Ok(AdmittedRuntimeAssemblyRequest::BytecodeHttp(
                AdmittedBytecodeHttpRequest {
                    route: bytecode_route,
                    header,
                    body,
                    target,
                },
            ));
        }
        #[cfg(any(test))]
        {
            let selector = ingress_selector(&header);
            let key = ServiceIngressKey {
                deployment: header.routing.deployment.clone(),
                selector: selector.clone(),
            };
            let route = self
                .resolve_active_assembly_request_route(&key, bootstrap)
                .await?;
            validate_route(&header, &selector, &route)?;
            header.deadline = effective_deadline(&header)?;
            if header
                .deadline
                .as_ref()
                .is_some_and(|deadline| deadline.timeout_ms == 0)
            {
                return Err(deadline_exceeded());
            }
            let request = http_gateway_request_from_admitted_wire(&route, &header, body)?;
            Ok(AdmittedRuntimeAssemblyRequest::Http(
                AdmittedHttpGatewayRequest {
                    route,
                    header,
                    request,
                },
            ))
        }
        #[cfg(not(test))]
        {
            Err(bytecode_required_error(&header.routing.deployment))
        }
    }

    async fn task_request_from_wire(
        &self,
        mut header: RuntimeAssemblyTaskRequestStartFrameHeader,
        payload: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<AdmittedRuntimeAssemblyRequest> {
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
        if let Some(bytecode_route) = self
            .resolve_bytecode_request_route(deployment, bootstrap)
            .await?
        {
            header.deadline = effective_request_deadline(header.deadline.as_ref(), "task")?;
            if header
                .deadline
                .as_ref()
                .is_some_and(|deadline| deadline.timeout_ms == 0)
            {
                return Err(deadline_exceeded());
            }
            let target = bytecode_route
                .request_target()
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            return Ok(AdmittedRuntimeAssemblyRequest::BytecodeTask(
                AdmittedBytecodeTaskRequest {
                    route: bytecode_route,
                    header,
                    target,
                    payload,
                },
            ));
        }
        #[cfg(any(test))]
        {
            let active = self
                .assembly_admission
                .deployment_image_or_lazy_load(
                    deployment,
                    &bootstrap.resolver,
                    Some(&bootstrap.service_db),
                    bootstrap.activation.profile.as_str(),
                    Some(bootstrap.resolver.store().root()),
                )
                .await
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            let linked_activation =
                active
                    .activation(deployment)
                    .ok_or_else(|| RuntimeError::Protocol {
                        target: header.invocation.target.clone(),
                        message: "task routing deployment is not loaded".to_string(),
                    })?;
            let activation = active
                .contexts()
                .activation_for_deployment(deployment)
                .ok_or_else(|| RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: "task routing deployment has no admitted activation".to_string(),
                })?;
            if activation.identity().deployment != *deployment
                || linked_activation.deployment_ref() != deployment
                || activation.implementation_package_build_id()
                    != linked_activation.implementation_package_build_id()
            {
                return Err(RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: "task routing does not match the admitted activation owner"
                        .to_string(),
                });
            }
            let execution_image = Arc::clone(active.candidate().execution_image());
            let request_activation =
                skiff_runtime_activation::RequestActivationContext::begin(Arc::clone(&activation))
                    .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::clone(active.contexts()) as _;
            let eval = RuntimeAssemblyEvalTarget::new(
                Arc::clone(&execution_image),
                request_activation,
                resolver,
            )
            .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            let target = RuntimeAssemblyTaskTarget::new(eval, header.invocation.target.clone())
                .map_err(|error| RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: error.to_string(),
                })?;
            let db_source = active
                .contexts()
                .db_source(activation.activation_id())
                .ok_or_else(|| RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: "task activation has no DB capability source".to_string(),
                })?;
            let config_views = active.contexts().config_views(deployment).ok_or_else(|| {
                RuntimeError::Protocol {
                    target: header.invocation.target.clone(),
                    message: "task activation has no scoped config views".to_string(),
                }
            })?;
            header.deadline = effective_request_deadline(header.deadline.as_ref(), "task")?;
            if header
                .deadline
                .as_ref()
                .is_some_and(|deadline| deadline.timeout_ms == 0)
            {
                return Err(deadline_exceeded());
            }
            let request = RuntimeTaskRequest {
                request_id: header.request_id.clone(),
                target: header.invocation.target.clone(),
                payload,
                test_effects_enabled: header.test_effects_enabled,
                test_case_capability: header.test_case_capability.clone(),
            };
            Ok(AdmittedRuntimeAssemblyRequest::Task(AdmittedTaskRequest {
                header,
                request,
                target,
                activation,
                execution_image,
                contexts: Arc::clone(active.contexts()),
                config_views,
                db_source,
                service_protocol_identity: linked_activation
                    .deployment()
                    .contract
                    .service_protocol_identity
                    .as_str()
                    .to_string(),
            }))
        }
        #[cfg(not(test))]
        {
            Err(bytecode_required_error(deployment))
        }
    }

    async fn websocket_jsonrpc_request_from_wire(
        &self,
        mut header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        params: Vec<u8>,
        bootstrap: &ConnectionBootstrap,
    ) -> Result<AdmittedRuntimeAssemblyRequest> {
        validate_websocket_jsonrpc_header(&header, &params)?;
        if let Some(bytecode_route) = self
            .resolve_bytecode_request_route(&header.routing.deployment, bootstrap)
            .await?
        {
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
            let target = bytecode_route
                .request_target()
                .map_err(|error| RuntimeError::Decode(error.to_string()))?;
            return Ok(AdmittedRuntimeAssemblyRequest::BytecodeWebSocketJsonRpc(
                AdmittedBytecodeWebSocketJsonRpcRequest {
                    route: bytecode_route,
                    header,
                    target,
                    params,
                },
            ));
        }
        #[cfg(any(test))]
        {
            let routing = &header.routing;
            let ingress = &routing.ingress;
            let request = &header.websocket_json_rpc;
            let profile = match request.profile {
                RuntimeAssemblyWebSocketJsonRpcProfile::JsonRpc2_0Text => {
                    GatewayWebSocketRpcProfile::JsonRpc2_0Text
                }
            };
            // The physical WebSocket route is the same admission unit as the
            // connect path: protocol WebSocket, path-only selector. The JSON-RPC
            // method capability route then joins on the physical entry, so every
            // request resolves against the routing.buildId deployment exactly like
            // HTTP admission (bind version, not build: a build switch mid-connection
            // applies from the next request).
            let selector = IngressSelector {
                protocol: IngressProtocol::WebSocket,
                method: None,
                path: ingress.path.clone(),
            };
            let key = ServiceIngressKey {
                deployment: routing.deployment.clone(),
                selector: selector.clone(),
            };
            let physical_route = self
                .resolve_active_assembly_request_route(&key, bootstrap)
                .await?;
            let method_route = physical_route
                .websocket_jsonrpc_method_route(
                    &ingress.path,
                    &ingress.method,
                    &routing.gateway_entry_identity,
                    profile,
                    &request.websocket_entry_id,
                )
                .map_err(|error| RuntimeError::Protocol {
                    target: request.connection_id.clone(),
                    message: error.to_string(),
                })?;
            let target = method_route
                .websocket_jsonrpc_target(&physical_route)
                .map_err(|error| RuntimeError::Protocol {
                    target: request.connection_id.clone(),
                    message: error.to_string(),
                })?;
            let resolved = ResolvedWebSocketJsonRpcExecution {
                target,
                method_route,
            };
            validate_websocket_jsonrpc_execution_route(&header, &resolved)?;
            header.deadline =
                effective_request_deadline(header.deadline.as_ref(), "WebSocket JSON-RPC")?;
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketJsonRpc(
                AdmittedWebSocketJsonRpcRequest {
                    resolved,
                    header,
                    params,
                },
            ))
        }
        #[cfg(not(test))]
        {
            Err(bytecode_required_error(&header.routing.deployment))
        }
    }
    #[cfg(test)]
    pub(crate) fn runtime_assembly_request_deadline_from_wire_for_test(
        &self,
        header: &RuntimeAssemblyRequestStartFrameHeader,
    ) -> Result<Option<RuntimeAssemblyRequestDeadlineFrameHeader>> {
        validate_http_header(header)?;
        let selector = ingress_selector(header);
        let key = ServiceIngressKey {
            deployment: header.routing.deployment.clone(),
            selector: selector.clone(),
        };
        let route = self.lookup_active_assembly_request_route(&key)?;
        validate_route(header, &selector, &route)?;
        effective_deadline(header)
    }
}

enum AdmittedRuntimeAssemblyRequest {
    #[cfg(test)]
    Http(AdmittedHttpGatewayRequest),
    BytecodeHttp(AdmittedBytecodeHttpRequest),
    #[cfg(test)]
    WebSocketConnect(AdmittedWebSocketConnectRequest),
    BytecodeWebSocketConnect(AdmittedBytecodeWebSocketConnectRequest),
    #[cfg(test)]
    WebSocketConnectionClosed(AdmittedWebSocketConnectionClosedRequest),
    BytecodeWebSocketConnectionClosed(AdmittedBytecodeWebSocketConnectionClosedRequest),
    #[cfg(test)]
    WebSocketJsonRpc(AdmittedWebSocketJsonRpcRequest),
    BytecodeWebSocketJsonRpc(AdmittedBytecodeWebSocketJsonRpcRequest),
    #[cfg(test)]
    Task(AdmittedTaskRequest),
    BytecodeTask(AdmittedBytecodeTaskRequest),
}

fn bytecode_required_error(deployment: &ServiceDeploymentRef) -> RuntimeError {
    RuntimeError::Protocol {
        target: deployment.deployment_artifact_identity.as_str().to_string(),
        message: "bytecode is required for this deployment; legacy assembly routes are disabled"
            .to_string(),
    }
}

#[cfg(test)]
fn gateway_ingress_pin(
    route: &ActiveAssemblyRoute,
    gateway_entry_identity: &skiff_artifact_model::GatewayEntryIdentity,
) -> RuntimeGatewayIngressPin {
    // The pin derives from the loaded buildId-keyed route, not from the
    // request frame's assembly tuple: the buildId is the only routing
    // dimension and the frame tuple is tolerated/defaulted.
    RuntimeGatewayIngressPin {
        assembly_identity: route.assembly_identity().clone(),
        assembly_generation: route.generation(),
        deployment: route.deployment().clone(),
        gateway_entry_identity: gateway_entry_identity.clone(),
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

/// Exact buildId a request resolves against: the deployment artifact identity
/// is the loading unit; a frame-provided buildId must equal it.
fn wire_routing_build_id(header: &RuntimeAssemblyRequestStartFrameWireHeader) -> Option<String> {
    let deployment = match header {
        RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => &header.routing.deployment,
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => {
            &header.routing.deployment
        }
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnectionClosed(header) => {
            &header.routing.deployment
        }
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
            &header.routing.deployment
        }
        RuntimeAssemblyRequestStartFrameWireHeader::Task(header) => &header.routing.deployment,
    };
    Some(deployment.deployment_artifact_identity.as_str().to_string())
}

#[cfg(test)]
fn http_gateway_request_from_admitted_wire(
    route: &ActiveAssemblyRoute,
    header: &RuntimeAssemblyRequestStartFrameHeader,
    body: Vec<u8>,
) -> Result<RuntimeHttpGatewayRequest> {
    let dispatch_mode = match header.mode.as_str() {
        "unary" => GatewayDispatchMode::Unary,
        "serverStream" => GatewayDispatchMode::ServerStream,
        other => {
            return Err(RuntimeError::Decode(format!(
                "canonical HTTP gateway dispatch mode is invalid: {other}"
            )))
        }
    };
    Ok(RuntimeHttpGatewayRequest {
        request_id: header.request_id.clone(),
        dispatch_mode,
        pin: gateway_ingress_pin(route, &header.routing.gateway_entry_identity),
        ingress_method: header.routing.ingress.method.clone(),
        ingress_path: header.routing.ingress.path.clone(),
        http_request: BinaryHttpRequestMetadata {
            method: header.http_request.method.clone(),
            url: header.http_request.url.clone(),
            path: header.http_request.path.clone(),
            query: request_name_values(&header.http_request.query),
            headers: request_name_values(&header.http_request.headers),
        },
        body,
        test_effects_enabled: header.test_effects_enabled,
    })
}

#[cfg(test)]
fn websocket_connect_ingress_from_wire(
    route: &ActiveAssemblyRoute,
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> RuntimeWebSocketConnectIngress {
    let request = &header.websocket_connect;
    RuntimeWebSocketConnectIngress {
        request_id: header.request_id.clone(),
        pin: gateway_ingress_pin(route, &header.routing.gateway_entry_identity),
        ingress_path: header.routing.ingress.path.clone(),
        connection_id: request.connection_id.clone(),
        url: request.url.clone(),
        query: request_name_values(&request.query),
        headers: request_name_values(&request.headers),
        cookies: request_name_values(&request.cookies),
        version: request.version.clone(),
        websocket_entry_id: request.websocket_entry_id.clone(),
        connect_gateway_entry_identity: request.gateway_entry_identity.clone(),
        test_effects_enabled: header.test_effects_enabled,
    }
}

#[cfg(test)]
fn websocket_connection_closed_ingress_from_wire(
    route: &ActiveAssemblyRoute,
    header: &RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
) -> RuntimeWebSocketConnectionClosedIngress {
    let request = &header.websocket_connection_closed;
    RuntimeWebSocketConnectionClosedIngress {
        request_id: header.request_id.clone(),
        pin: gateway_ingress_pin(route, &header.routing.gateway_entry_identity),
        ingress_path: header.routing.ingress.path.clone(),
        connection_id: request.connection_id.clone(),
        websocket_entry_id: request.websocket_entry_id.clone(),
        close_gateway_entry_identity: request.gateway_entry_identity.clone(),
        business_identity: request.business_identity.clone(),
        close_code: request.close_code,
        close_reason: request.close_reason.clone(),
        test_effects_enabled: header.test_effects_enabled,
    }
}

#[cfg(test)]
fn request_name_values(
    values: &[skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestNameValueFrameHeader],
) -> Vec<HttpNameValue> {
    values
        .iter()
        .map(|value| HttpNameValue {
            name: value.name.clone(),
            value: value.value.clone(),
        })
        .collect()
}

fn validate_http_header(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<()> {
    if header.request_id.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical request.start requestId must be non-empty".to_string(),
        ));
    }
    if header.caller.kind != "gateway" {
        return Err(RuntimeError::Unsupported(
            "canonical HTTP gateway request requires caller.kind gateway".to_string(),
        ));
    }
    if header.routing.ingress.protocol != RuntimeAssemblyRequestIngressProtocol::Http {
        return Err(RuntimeError::Unsupported(
            "RuntimeAssembly request bridge accepts only canonical HTTP gateway requests"
                .to_string(),
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

#[cfg(test)]
fn ingress_selector(header: &RuntimeAssemblyRequestStartFrameHeader) -> IngressSelector {
    let ingress = &header.routing.ingress;
    IngressSelector {
        protocol: IngressProtocol::Http,
        method: Some(ingress.method.clone()),
        path: ingress.path.clone(),
    }
}

fn validate_websocket_connect_header(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
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
    header: &RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
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
    header: &RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
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
    header: &RuntimeAssemblyTaskRequestStartFrameHeader,
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

#[cfg(test)]
fn validate_websocket_jsonrpc_execution_route(
    header: &RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    resolved: &ResolvedWebSocketJsonRpcExecution,
) -> Result<()> {
    let route = &resolved.method_route;
    let target = &resolved.target;
    let routing = &header.routing;
    let ingress = &routing.ingress;
    if routing_build_id_mismatch(routing.build_id.as_deref(), route)
        || route.deployment() != &routing.deployment
        || route.selector().path != ingress.path
        || route.selector().method.as_deref() != Some(ingress.method.as_str())
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || target.assembly_identity() != route.assembly_identity()
        || target.assembly_generation() != route.generation()
        || target.selector() != route.selector()
        || target.gateway_entry_identity() != route.gateway_entry_identity()
        || target.owner() != route.entry().owner()
        || target.implementation_package_build_id()
            != route.activation().implementation_package_build_id()
        || !std::sync::Arc::ptr_eq(target.eval().activation_context(), route.activation())
        || !std::sync::Arc::ptr_eq(target.eval().execution_image(), route.execution_image())
    {
        return Err(RuntimeError::Protocol {
            target: header.websocket_json_rpc.connection_id.clone(),
            message:
                "resolved WebSocket JSON-RPC target and method capability route have different generation owners"
                    .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
fn websocket_connect_ingress_selector(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> IngressSelector {
    let ingress = &header.routing.ingress;
    IngressSelector {
        protocol: IngressProtocol::WebSocket,
        method: None,
        path: ingress.path.clone(),
    }
}

#[cfg(test)]
fn websocket_connection_closed_ingress_selector(
    header: &RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
) -> IngressSelector {
    let ingress = &header.routing.ingress;
    IngressSelector {
        protocol: IngressProtocol::WebSocket,
        method: None,
        path: ingress.path.clone(),
    }
}

#[cfg(test)]
fn validate_websocket_connection_closed_route(
    header: &RuntimeAssemblyWebSocketConnectionClosedRequestStartFrameHeader,
    selector: &IngressSelector,
    route: &ActiveAssemblyRoute,
) -> Result<()> {
    let routing = &header.routing;
    let activation_identity = route.activation().identity();
    if !matches!(
        route.protocol_surface().protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ) || routing_build_id_mismatch(routing.build_id.as_deref(), route)
        || route.deployment() != &routing.deployment
        || route.selector() != selector
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || &activation_identity.deployment != route.entry().owner()
        || route.gateway_entry_identity()
            != &header.websocket_connection_closed.gateway_entry_identity
        || !route.activation().websocket_entry_matches(
            selector,
            route.gateway_entry_key(),
            route.gateway_entry_identity(),
            &header.websocket_connection_closed.websocket_entry_id,
        )
    {
        return Err(RuntimeError::Protocol {
            target: route.gateway_entry_key().as_str().to_string(),
            message:
                "canonical request routing does not match the admitted WebSocket connection close route"
                    .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
fn validate_websocket_connect_route(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    selector: &IngressSelector,
    route: &ActiveAssemblyRoute,
) -> Result<()> {
    let routing = &header.routing;
    let activation_identity = route.activation().identity();
    if !matches!(
        route.protocol_surface().protocol,
        GatewayProtocolSurface::WebSocketConnect(_)
    ) || routing_build_id_mismatch(routing.build_id.as_deref(), route)
        || route.deployment() != &routing.deployment
        || route.selector() != selector
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || &activation_identity.deployment != route.entry().owner()
        || route.gateway_entry_identity() != &header.websocket_connect.gateway_entry_identity
        || !route.activation().websocket_entry_matches(
            selector,
            route.gateway_entry_key(),
            route.gateway_entry_identity(),
            &header.websocket_connect.websocket_entry_id,
        )
    {
        return Err(RuntimeError::Protocol {
            target: route.gateway_entry_key().as_str().to_string(),
            message:
                "canonical request routing does not match the admitted WebSocket connect route"
                    .to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
fn validate_route(
    header: &RuntimeAssemblyRequestStartFrameHeader,
    selector: &IngressSelector,
    route: &ActiveAssemblyRoute,
) -> Result<()> {
    let routing = &header.routing;
    let activation_identity = route.activation().identity();
    let GatewayProtocolSurface::Http(http) = &route.protocol_surface().protocol else {
        return Err(RuntimeError::Protocol {
            target: route.gateway_entry_key().as_str().to_string(),
            message: "HTTP request bridge cannot admit a non-HTTP gateway route".to_string(),
        });
    };
    let expected_mode = match http.dispatch_mode {
        GatewayDispatchMode::Unary => "unary",
        GatewayDispatchMode::ServerStream => "serverStream",
    };
    let adapter_mode_is_valid = matches!(
        (http.adapter_kind, http.dispatch_mode),
        (GatewayAdapterKind::TypedJson, GatewayDispatchMode::Unary)
            | (GatewayAdapterKind::RawHttp, GatewayDispatchMode::Unary)
            | (
                GatewayAdapterKind::RawHttp,
                GatewayDispatchMode::ServerStream
            )
    );
    if routing_build_id_mismatch(routing.build_id.as_deref(), route)
        || route.deployment() != &routing.deployment
        || route.selector() != selector
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || &activation_identity.deployment != route.entry().owner()
        || header.mode != expected_mode
        || !adapter_mode_is_valid
    {
        return Err(RuntimeError::Protocol {
            target: route.gateway_entry_key().as_str().to_string(),
            message: "canonical request routing does not match the admitted HTTP gateway route"
                .to_string(),
        });
    }
    Ok(())
}

/// M2 routing authority: when the request carries an exact buildId it must
/// match the route's deployment artifact identity. Routers without buildId
/// support fall back to the exact deployment ref match performed by the
/// ingress key; assembly identity/generation are no longer consumed.
#[cfg(test)]
fn routing_build_id_mismatch(frame_build_id: Option<&str>, route: &ActiveAssemblyRoute) -> bool {
    frame_build_id.is_some_and(|build_id| {
        build_id != route.deployment().deployment_artifact_identity.as_str()
    })
}

fn effective_deadline(
    header: &RuntimeAssemblyRequestStartFrameHeader,
) -> Result<Option<RuntimeAssemblyRequestDeadlineFrameHeader>> {
    effective_request_deadline(header.deadline.as_ref(), "HTTP gateway")
}

fn effective_request_deadline(
    deadline: Option<&RuntimeAssemblyRequestDeadlineFrameHeader>,
    request_kind: &str,
) -> Result<Option<RuntimeAssemblyRequestDeadlineFrameHeader>> {
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
    Ok(Some(RuntimeAssemblyRequestDeadlineFrameHeader {
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

use skiff_artifact_model::{
    GatewayAdapterKind, GatewayDispatchMode, GatewayProtocolSurface, GatewayWebSocketRpcProfile,
    IngressProtocol, IngressSelector, ServiceIngressKey,
};
use skiff_runtime_capability_context::ExecutionBudgetReason;
use skiff_runtime_request::{
    BinaryHttpRequestMetadata, HttpNameValue, RequestError, RouterWriterMessage,
    RuntimeGatewayIngressPin, RuntimeHttpGatewayRequest, RuntimeWebSocketConnectIngress,
};
use skiff_runtime_transport::response_mapper::OrdinaryResponseEvent;
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressProtocol,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestStartFrameWireHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader, RuntimeAssemblyWebSocketJsonRpcProfile,
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;
use tracing::error;
use url::Url;

use super::{request_error_into_runtime_error, response_event_into_transport_message};
use crate::{
    error::{Result, RuntimeError},
    host::RuntimeHost,
    loader::assembly_admission::ActiveAssemblyRoute,
};

pub(super) struct AdmittedHttpGatewayRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyRequestStartFrameHeader,
    pub(super) request: RuntimeHttpGatewayRequest,
}

pub(super) struct AdmittedWebSocketConnectRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
    pub(super) request: RuntimeWebSocketConnectIngress,
}

pub(super) struct AdmittedWebSocketJsonRpcRequest {
    pub(super) resolved: crate::host::websocket_generation::ResolvedWebSocketJsonRpcExecution,
    pub(super) header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    pub(super) params: Vec<u8>,
}

impl RuntimeHost {
    pub(crate) async fn spawn_runtime_assembly_request(
        &self,
        router_session_id: &str,
        header: RuntimeAssemblyRequestStartFrameWireHeader,
        body: Vec<u8>,
        http_response_max_bytes: usize,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let request_id = match &header {
            RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => header.request_id.clone(),
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => {
                header.request_id.clone()
            }
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(header) => {
                header.request_id.clone()
            }
        };
        let result = match header {
            RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => self
                .http_gateway_request_from_wire(header, body)
                .map(AdmittedRuntimeAssemblyRequest::Http),
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => self
                .websocket_connect_request_from_wire(header, body)
                .map(AdmittedRuntimeAssemblyRequest::WebSocketConnect),
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(header) => self
                .websocket_jsonrpc_request_from_wire(router_session_id, header, body)
                .map(AdmittedRuntimeAssemblyRequest::WebSocketJsonRpc),
        };
        match result {
            Ok(AdmittedRuntimeAssemblyRequest::Http(request)) => {
                self.spawn_request_on_active_assembly_route(
                    router_session_id.to_string(),
                    request,
                    http_response_max_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnect(request)) => {
                self.spawn_websocket_connect_on_active_assembly_route(
                    router_session_id.to_string(),
                    request,
                    http_response_max_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketJsonRpc(request)) => {
                self.spawn_websocket_jsonrpc_on_pinned_route(
                    router_session_id.to_string(),
                    request,
                    http_response_max_bytes,
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

    fn websocket_connect_request_from_wire(
        &self,
        header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        body: Vec<u8>,
    ) -> Result<AdmittedWebSocketConnectRequest> {
        validate_websocket_connect_header(&header, &body)?;
        let selector = websocket_connect_ingress_selector(&header);
        let key = ServiceIngressKey {
            deployment: header.routing.deployment.clone(),
            selector: selector.clone(),
        };
        let route = self.lookup_active_assembly_request_route(&key)?;
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
        let request = websocket_connect_ingress_from_wire(&header);
        Ok(AdmittedWebSocketConnectRequest {
            route,
            header,
            request,
        })
    }

    fn http_gateway_request_from_wire(
        &self,
        mut header: RuntimeAssemblyRequestStartFrameHeader,
        body: Vec<u8>,
    ) -> Result<AdmittedHttpGatewayRequest> {
        validate_http_header(&header)?;
        let selector = ingress_selector(&header);
        let key = ServiceIngressKey {
            deployment: header.routing.deployment.clone(),
            selector: selector.clone(),
        };
        let route = self.lookup_active_assembly_request_route(&key)?;
        validate_route(&header, &selector, &route)?;
        header.deadline = effective_deadline(&header, &route)?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        let request = http_gateway_request_from_admitted_wire(&header, body)?;
        Ok(AdmittedHttpGatewayRequest {
            route,
            header,
            request,
        })
    }

    fn websocket_jsonrpc_request_from_wire(
        &self,
        router_session_id: &str,
        mut header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        params: Vec<u8>,
    ) -> Result<AdmittedWebSocketJsonRpcRequest> {
        validate_websocket_jsonrpc_header(&header, &params)?;
        let routing = &header.routing;
        let ingress = &routing.ingress;
        let request = &header.websocket_json_rpc;
        let profile = match request.profile {
            RuntimeAssemblyWebSocketJsonRpcProfile::JsonRpc2_0Text => {
                GatewayWebSocketRpcProfile::JsonRpc2_0Text
            }
        };
        let resolved = self
            .websocket_generations
            .websocket_jsonrpc_execution_route(
                router_session_id,
                &request.connection_id,
                &routing.assembly_identity,
                routing.assembly_generation,
                &request.websocket_entry_id,
                &ingress.path,
                &ingress.method,
                &routing.gateway_entry_identity,
                profile,
            )?;
        validate_websocket_jsonrpc_execution_route(&header, &resolved)?;
        header.deadline = effective_request_deadline(
            header.deadline.as_ref(),
            &resolved.method_route,
            "WebSocket JSON-RPC",
        )?;
        Ok(AdmittedWebSocketJsonRpcRequest {
            resolved,
            header,
            params,
        })
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
        effective_deadline(header, &route)
    }
}

enum AdmittedRuntimeAssemblyRequest {
    Http(AdmittedHttpGatewayRequest),
    WebSocketConnect(AdmittedWebSocketConnectRequest),
    WebSocketJsonRpc(AdmittedWebSocketJsonRpcRequest),
}

fn gateway_ingress_pin(
    assembly_identity: &skiff_artifact_model::AssemblyIdentity,
    assembly_generation: u64,
    deployment: &skiff_artifact_model::ServiceDeploymentRef,
    gateway_entry_identity: &skiff_artifact_model::GatewayEntryIdentity,
) -> RuntimeGatewayIngressPin {
    RuntimeGatewayIngressPin {
        assembly_identity: assembly_identity.clone(),
        assembly_generation,
        deployment: deployment.clone(),
        gateway_entry_identity: gateway_entry_identity.clone(),
    }
}

fn http_gateway_request_from_admitted_wire(
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
        pin: gateway_ingress_pin(
            &header.routing.assembly_identity,
            header.routing.assembly_generation,
            &header.routing.deployment,
            &header.routing.gateway_entry_identity,
        ),
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

fn websocket_connect_ingress_from_wire(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> RuntimeWebSocketConnectIngress {
    let request = &header.websocket_connect;
    RuntimeWebSocketConnectIngress {
        request_id: header.request_id.clone(),
        pin: gateway_ingress_pin(
            &header.routing.assembly_identity,
            header.routing.assembly_generation,
            &header.routing.deployment,
            &header.routing.gateway_entry_identity,
        ),
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

fn validate_websocket_jsonrpc_execution_route(
    header: &RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    resolved: &crate::host::websocket_generation::ResolvedWebSocketJsonRpcExecution,
) -> Result<()> {
    let route = &resolved.method_route;
    let target = &resolved.target;
    let routing = &header.routing;
    let ingress = &routing.ingress;
    if route.assembly_identity() != &routing.assembly_identity
        || route.generation() != routing.assembly_generation
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
    ) || route.assembly_identity() != &routing.assembly_identity
        || route.generation() != routing.assembly_generation
        || route.deployment() != &routing.deployment
        || route.selector() != selector
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || activation_identity.assembly_identity != routing.assembly_identity
        || activation_identity.assembly_generation != routing.assembly_generation
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
    if route.assembly_identity() != &routing.assembly_identity
        || route.generation() != routing.assembly_generation
        || route.deployment() != &routing.deployment
        || route.selector() != selector
        || route.gateway_entry_identity() != &routing.gateway_entry_identity
        || activation_identity.assembly_identity != routing.assembly_identity
        || activation_identity.assembly_generation != routing.assembly_generation
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

fn effective_deadline(
    header: &RuntimeAssemblyRequestStartFrameHeader,
    route: &ActiveAssemblyRoute,
) -> Result<Option<RuntimeAssemblyRequestDeadlineFrameHeader>> {
    effective_request_deadline(header.deadline.as_ref(), route, "HTTP gateway")
}

fn effective_request_deadline(
    deadline: Option<&RuntimeAssemblyRequestDeadlineFrameHeader>,
    route: &ActiveAssemblyRoute,
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
    if let Some(timeout_ms) = route.deployment_policy().timeout_ms {
        candidates.push(timeout_ms);
    }
    let Some(timeout_ms) = candidates.into_iter().min() else {
        return Ok(None);
    };
    let timeout_i64 = i64::try_from(timeout_ms).map_err(|_| {
        RuntimeError::Decode(format!(
            "{request_kind} deployment deadline is not representable by the Host"
        ))
    })?;
    let expires_at = wall_now
        .checked_add(time::Duration::milliseconds(timeout_i64))
        .ok_or_else(|| {
            RuntimeError::Decode(format!(
                "{request_kind} deployment deadline is not representable by the Host"
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

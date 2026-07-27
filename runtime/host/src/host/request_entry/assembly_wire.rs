use skiff_artifact_model::{
    GatewayAdapterKind, GatewayDispatchMode, GatewayProtocolSurface, IngressProtocol,
    IngressSelector,
};
use skiff_runtime_capability_context::ExecutionBudgetReason;
use skiff_runtime_request::{RequestError, ResponseEvent, RouterWriterMessage};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressProtocol,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestStartFrameWireHeader,
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;
use tracing::error;
use url::{Position, Url};

use super::{request_error_into_runtime_error, response_event_into_transport_message};
use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    host::RuntimeHost,
    loader::assembly_admission::ActiveAssemblyRoute,
};

pub(super) struct AdmittedHttpGatewayRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyRequestStartFrameHeader,
    pub(super) body: Vec<u8>,
}

pub(super) struct AdmittedWebSocketConnectRequest {
    pub(super) route: ActiveAssemblyRoute,
    pub(super) header: RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
}

impl RuntimeHost {
    pub(crate) async fn spawn_runtime_assembly_request(
        &self,
        _router_session_id: &str,
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
        };
        let result = match header {
            RuntimeAssemblyRequestStartFrameWireHeader::Http(header) => self
                .http_gateway_request_from_wire(header, body)
                .map(AdmittedRuntimeAssemblyRequest::Http),
            RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(header) => self
                .websocket_connect_request_from_wire(header, body)
                .map(AdmittedRuntimeAssemblyRequest::WebSocketConnect),
        };
        match result {
            Ok(AdmittedRuntimeAssemblyRequest::Http(request)) => {
                self.spawn_request_on_active_assembly_route(
                    request,
                    http_response_max_bytes,
                    sender,
                )
                .await
            }
            Ok(AdmittedRuntimeAssemblyRequest::WebSocketConnect(request)) => {
                self.spawn_websocket_connect_on_active_assembly_route(
                    _router_session_id.to_string(),
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
                match response_event_into_transport_message(
                    request_id,
                    ResponseEvent::Error(response_error_from_runtime_error(&runtime_error)),
                ) {
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
        let route = self.lookup_active_assembly_request_route(&selector)?;
        validate_websocket_connect_route(&header, &selector, &route)?;
        if route.entry().optional_handler().is_none() {
            return Err(RuntimeError::Protocol {
                target: route.gateway_entry_key().as_str().to_string(),
                message:
                    "Runtime refuses WebSocket connect dispatch for an entry without a handler"
                        .to_string(),
            });
        }
        Ok(AdmittedWebSocketConnectRequest { route, header })
    }

    fn http_gateway_request_from_wire(
        &self,
        mut header: RuntimeAssemblyRequestStartFrameHeader,
        body: Vec<u8>,
    ) -> Result<AdmittedHttpGatewayRequest> {
        validate_http_header(&header)?;
        let selector = ingress_selector(&header);
        let route = self.lookup_active_assembly_request_route(&selector)?;
        validate_route(&header, &selector, &route)?;
        header.deadline = effective_deadline(&header, &route)?;
        if header
            .deadline
            .as_ref()
            .is_some_and(|deadline| deadline.timeout_ms == 0)
        {
            return Err(deadline_exceeded());
        }
        Ok(AdmittedHttpGatewayRequest {
            route,
            header,
            body,
        })
    }

    #[cfg(test)]
    pub(crate) fn runtime_assembly_request_deadline_from_wire_for_test(
        &self,
        header: &RuntimeAssemblyRequestStartFrameHeader,
    ) -> Result<Option<RuntimeAssemblyRequestDeadlineFrameHeader>> {
        validate_http_header(header)?;
        let selector = ingress_selector(header);
        let route = self.lookup_active_assembly_request_route(&selector)?;
        validate_route(header, &selector, &route)?;
        effective_deadline(header, &route)
    }
}

enum AdmittedRuntimeAssemblyRequest {
    Http(AdmittedHttpGatewayRequest),
    WebSocketConnect(AdmittedWebSocketConnectRequest),
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
        || &url[Position::BeforeHost..Position::AfterPort] != ingress.host.as_str()
    {
        return Err(RuntimeError::Decode(
            "httpRequest URL host/path does not match canonical routing ingress".to_string(),
        ));
    }
    Ok(())
}

fn ingress_selector(header: &RuntimeAssemblyRequestStartFrameHeader) -> IngressSelector {
    let ingress = &header.routing.ingress;
    IngressSelector {
        protocol: IngressProtocol::Http,
        host: ingress.host.clone(),
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
        || &url[Position::BeforeHost..Position::AfterPort] != ingress.host.as_str()
    {
        return Err(RuntimeError::Decode(
            "websocketConnect URL host/path does not match canonical routing ingress".to_string(),
        ));
    }
    Ok(())
}

fn websocket_connect_ingress_selector(
    header: &RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
) -> IngressSelector {
    let ingress = &header.routing.ingress;
    IngressSelector {
        protocol: IngressProtocol::WebSocket,
        host: ingress.host.clone(),
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
    let wall_now = OffsetDateTime::now_utc();
    let mut candidates = Vec::new();
    if let Some(deadline) = &header.deadline {
        candidates.push(deadline.timeout_ms);
        let expires_at = OffsetDateTime::parse(&deadline.expires_at, &Rfc3339).map_err(|_| {
            RuntimeError::Decode(
                "canonical HTTP gateway deadline expiresAt must be valid RFC3339".to_string(),
            )
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
        RuntimeError::Decode(
            "HTTP gateway deployment deadline is not representable by the Host".to_string(),
        )
    })?;
    let expires_at = wall_now
        .checked_add(time::Duration::milliseconds(timeout_i64))
        .ok_or_else(|| {
            RuntimeError::Decode(
                "HTTP gateway deployment deadline is not representable by the Host".to_string(),
            )
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

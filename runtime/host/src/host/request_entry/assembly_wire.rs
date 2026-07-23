use serde_json::{Map, Value};
use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{
    GatewayAdapterArg, GatewayAdapterSource, HttpNameValue, RequestEnvelope, ResponseEvent,
    RouterWriterMessage, WebSocketAdapter, WebSocketAdapterKind, WebSocketConnectRequest,
    WebSocketContextCodec, WebSocketContextExpectation, WebSocketMessage, WebSocketMessageEncoding,
    WebSocketMessageTag, WebSocketPayloadSegment, WebSocketPayloadSegmentKind,
    WebSocketReceiveRequest,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyWebSocketAdapterFrameHeader, RuntimeAssemblyWebSocketAdapterKindFrameHeader,
    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader,
    RuntimeAssemblyWebSocketContextExpectationFrameHeader,
    RuntimeAssemblyWebSocketMessageEncodingFrameHeader,
    RuntimeAssemblyWebSocketMessageTagFrameHeader,
    RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader,
};
use tokio::sync::mpsc;
use tracing::error;
use url::{Position, Url};

use super::{
    response_event_into_transport_message,
    websocket_generation::{websocket_connect_generation_pin, WebSocketConnectGenerationPin},
};
use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    host::RuntimeHost,
    loader::assembly_admission::ActiveAssemblyRoute,
};

impl RuntimeHost {
    pub(crate) async fn spawn_runtime_assembly_request(
        &self,
        router_session_id: &str,
        header: RuntimeAssemblyRequestStartFrameHeader,
        payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let request_id = header.request_id.clone();
        match self.runtime_assembly_request_from_wire(router_session_id, header, payload) {
            Ok((route, request, connect_pin)) => {
                self.spawn_request_on_active_assembly_route(route, request, connect_pin, sender)
                    .await;
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

    fn runtime_assembly_request_from_wire(
        &self,
        router_session_id: &str,
        header: RuntimeAssemblyRequestStartFrameHeader,
        payload: Vec<u8>,
    ) -> Result<(
        ActiveAssemblyRoute,
        RequestEnvelope,
        Option<WebSocketConnectGenerationPin>,
    )> {
        validate_narrow_unary_header(&header)?;
        let selector = ingress_selector(&header)?;
        let route = self.runtime_assembly_route_for_wire(router_session_id, &header, &selector)?;
        validate_route(&header, &selector, &route)?;
        let request = request_envelope_from_route(header, payload, &route)?;
        let connect_pin = websocket_connect_generation_pin(router_session_id, &request)?;
        Ok((route, request, connect_pin))
    }

    #[cfg(test)]
    pub(crate) fn runtime_assembly_request_route_from_wire_for_test(
        &self,
        router_session_id: &str,
        header: &RuntimeAssemblyRequestStartFrameHeader,
    ) -> Result<ActiveAssemblyRoute> {
        validate_narrow_unary_header(header)?;
        let selector = ingress_selector(header)?;
        let route = self.runtime_assembly_route_for_wire(router_session_id, header, &selector)?;
        validate_route(header, &selector, &route)?;
        Ok(route)
    }
}

fn validate_narrow_unary_header(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<()> {
    if header.request_id.is_empty() {
        return Err(RuntimeError::Decode(
            "canonical request.start requestId must be non-empty".to_string(),
        ));
    }
    if header.mode != "unary" {
        return Err(RuntimeError::Unsupported(format!(
            "canonical assembly ingress only supports unary request.start, got {}",
            header.mode
        )));
    }
    if header.caller.kind != "gateway" {
        return Err(RuntimeError::Unsupported(
            "canonical assembly ingress requires caller.kind gateway".to_string(),
        ));
    }
    if header.test_effects_enabled || !header.test_effect_doubles.is_empty() {
        return Err(RuntimeError::Unsupported(
            "canonical unary bridge does not accept test effects".to_string(),
        ));
    }
    match header.routing.ingress.protocol {
        RuntimeAssemblyRequestIngressProtocol::Http => validate_http_ingress_header(header),
        RuntimeAssemblyRequestIngressProtocol::WebSocket => {
            validate_websocket_ingress_header(header)
        }
    }
}

fn validate_http_ingress_header(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<()> {
    if header.http_adapter.is_some() || header.websocket_adapter.is_some() {
        return Err(RuntimeError::Unsupported(
            "canonical HTTP ingress does not accept gateway adapter metadata".to_string(),
        ));
    }
    let http_request = header.http_request.as_ref().ok_or_else(|| {
        RuntimeError::Decode(
            "canonical HTTP assembly ingress requires httpRequest metadata".to_string(),
        )
    })?;
    let ingress = &header.routing.ingress;
    if http_request.method != ingress.method.as_deref().unwrap_or_default()
        || http_request.path != ingress.path
    {
        return Err(RuntimeError::Decode(
            "httpRequest method/path does not match canonical routing ingress".to_string(),
        ));
    }
    let url = Url::parse(&http_request.url).map_err(|error| {
        RuntimeError::Decode(format!("canonical httpRequest URL is invalid: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
        || url.fragment().is_some()
    {
        return Err(RuntimeError::Decode(
            "canonical httpRequest URL must be an HTTP(S) URL without credentials or fragment"
                .to_string(),
        ));
    }
    if &url[Position::BeforeHost..Position::AfterPort] != ingress.host || url.path() != ingress.path
    {
        return Err(RuntimeError::Decode(
            "httpRequest URL host/path does not match canonical routing ingress".to_string(),
        ));
    }
    Ok(())
}

fn validate_websocket_ingress_header(
    header: &RuntimeAssemblyRequestStartFrameHeader,
) -> Result<()> {
    if header.http_request.is_some() || header.http_adapter.is_some() {
        return Err(RuntimeError::Unsupported(
            "canonical WebSocket ingress does not accept HTTP metadata".to_string(),
        ));
    }
    if header.websocket_adapter.is_none() {
        return Err(RuntimeError::Decode(
            "canonical WebSocket ingress requires websocketAdapter metadata".to_string(),
        ));
    }
    Ok(())
}

fn ingress_selector(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<IngressSelector> {
    let ingress = &header.routing.ingress;
    Ok(IngressSelector {
        protocol: match ingress.protocol {
            RuntimeAssemblyRequestIngressProtocol::Http => IngressProtocol::Http,
            RuntimeAssemblyRequestIngressProtocol::WebSocket => IngressProtocol::WebSocket,
        },
        host: ingress.host.clone(),
        method: ingress.method.clone(),
        path: ingress.path.clone(),
    })
}

fn validate_route(
    header: &RuntimeAssemblyRequestStartFrameHeader,
    selector: &IngressSelector,
    route: &ActiveAssemblyRoute,
) -> Result<()> {
    let routing = &header.routing;
    let activation_identity = route.activation().identity();
    if route.assembly_identity() != &routing.assembly_identity
        || route.generation() != routing.assembly_generation
        || route.binding().selector != *selector
        || route.binding().contract_operation_id != routing.contract_operation_id
        || route.operation_descriptor().operation_id != routing.contract_operation_id
        || activation_identity.assembly_identity != routing.assembly_identity
        || activation_identity.assembly_generation != routing.assembly_generation
        || activation_identity.deployment != route.binding().deployment
        || route.binding().contract.service_id != route.binding().deployment.service_id
    {
        return Err(RuntimeError::Protocol {
            target: routing.contract_operation_id.as_str().to_string(),
            message: "canonical request routing does not match the current admitted assembly route"
                .to_string(),
        });
    }
    Ok(())
}

fn request_envelope_from_route(
    header: RuntimeAssemblyRequestStartFrameHeader,
    payload: Vec<u8>,
    route: &ActiveAssemblyRoute,
) -> Result<RequestEnvelope> {
    let binding = route.binding();
    let operation_id = route
        .operation_descriptor()
        .operation_id
        .as_str()
        .to_string();
    let activation = route.activation();
    let extra = request_extra(&header)?;
    let websocket_adapter = header
        .websocket_adapter
        .as_ref()
        .map(websocket_adapter_from_header);
    Ok(RequestEnvelope {
        request_id: header.request_id,
        mode: header.mode,
        target: operation_id,
        operation_abi_id: None,
        selector: None,
        service_id: Some(binding.contract.service_id.clone()),
        build_id: activation
            .implementation_package_build_id()
            .as_str()
            .to_string(),
        service_protocol_identity: binding
            .contract
            .service_protocol_identity
            .as_str()
            .to_string(),
        contract_identity: None,
        activation_identity: Some(activation.activation_id().as_str().to_string()),
        ingress_selector: Some(binding.selector.clone()),
        binary_http: None,
        http_adapter: None,
        websocket_adapter,
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: payload,
        extra,
    })
}

fn request_extra(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<Map<String, Value>> {
    let mut extra = Map::new();
    extra.insert("caller".to_string(), serde_json::to_value(&header.caller)?);
    insert_optional_string(
        &mut extra,
        "gatewayEntryIdentity",
        header.gateway_entry_identity.as_ref(),
    );
    insert_optional_string(
        &mut extra,
        "businessIdentity",
        header.business_identity.as_ref(),
    );
    insert_optional_string(
        &mut extra,
        "websocketEntryId",
        header.websocket_entry_id.as_ref(),
    );
    if let Some(client_session) = &header.client_session {
        extra.insert(
            "clientSession".to_string(),
            serde_json::to_value(client_session)?,
        );
    }
    if let Some(deadline) = &header.deadline {
        extra.insert("deadline".to_string(), serde_json::to_value(deadline)?);
    }
    extra.insert("trace".to_string(), serde_json::to_value(&header.trace)?);
    Ok(extra)
}

fn insert_optional_string(map: &mut Map<String, Value>, key: &str, value: Option<&String>) {
    if let Some(value) = value {
        map.insert(key.to_string(), Value::String(value.clone()));
    }
}

fn websocket_adapter_from_header(
    adapter: &RuntimeAssemblyWebSocketAdapterFrameHeader,
) -> WebSocketAdapter {
    WebSocketAdapter {
        kind: match adapter.kind {
            RuntimeAssemblyWebSocketAdapterKindFrameHeader::Connect => {
                WebSocketAdapterKind::Connect
            }
            RuntimeAssemblyWebSocketAdapterKindFrameHeader::Receive => {
                WebSocketAdapterKind::Receive
            }
        },
        adapter_args: adapter
            .adapter_args
            .iter()
            .map(|arg| GatewayAdapterArg {
                param: arg.param.clone(),
                source: match arg.source.kind {
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::IngressEvent => {
                        GatewayAdapterSource::WebSocketIngressEvent
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::ConnectRequest => {
                        GatewayAdapterSource::WebSocketConnectRequest
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::ReceiveEvent => {
                        GatewayAdapterSource::WebSocketReceiveEvent
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::Connection => {
                        GatewayAdapterSource::WebSocketConnection
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::ConnectionContext => {
                        GatewayAdapterSource::WebSocketConnectionContext
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::Message => {
                        GatewayAdapterSource::WebSocketMessage
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::MessageBody => {
                        GatewayAdapterSource::WebSocketMessageBody
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::ConnectionId => {
                        GatewayAdapterSource::WebSocketConnectionId
                    }
                    RuntimeAssemblyWebSocketAdapterSourceKindFrameHeader::BusinessIdentity => {
                        GatewayAdapterSource::WebSocketBusinessIdentity
                    }
                },
            })
            .collect(),
        context_expectation: adapter.context_expectation.as_ref().map(|expectation| {
            match expectation {
                RuntimeAssemblyWebSocketContextExpectationFrameHeader::Null => {
                    WebSocketContextExpectation::Null
                }
                RuntimeAssemblyWebSocketContextExpectationFrameHeader::Typed {
                    connect_operation_abi_id,
                    context_type_identity,
                } => WebSocketContextExpectation::Typed {
                    connect_operation_abi_id: connect_operation_abi_id.clone(),
                    context_type_identity: context_type_identity.clone(),
                },
            }
        }),
        connect_request: adapter
            .connect_request
            .as_ref()
            .map(|request| WebSocketConnectRequest {
                connection_id: request.connection_id.clone(),
                url: request.url.clone(),
                query: name_values(&request.query),
                headers: name_values(&request.headers),
                cookies: name_values(&request.cookies),
                version: request.version.clone(),
            }),
        receive_request: adapter
            .receive_event
            .as_ref()
            .map(|receive| WebSocketReceiveRequest {
                connection_id: receive.connection_id.clone(),
                business_identity: receive.business_identity.clone(),
                message: WebSocketMessage {
                    tag: match receive.message.tag {
                        RuntimeAssemblyWebSocketMessageTagFrameHeader::Text => {
                            WebSocketMessageTag::Text
                        }
                        RuntimeAssemblyWebSocketMessageTagFrameHeader::Binary => {
                            WebSocketMessageTag::Binary
                        }
                    },
                    encoding: match receive.message.encoding {
                        RuntimeAssemblyWebSocketMessageEncodingFrameHeader::Utf8 => {
                            WebSocketMessageEncoding::Utf8
                        }
                        RuntimeAssemblyWebSocketMessageEncodingFrameHeader::Binary => {
                            WebSocketMessageEncoding::Raw
                        }
                    },
                },
                context_codec: receive
                    .context_codec
                    .as_ref()
                    .map(|codec| WebSocketContextCodec {
                        operation_abi_id: codec.operation_abi_id.clone(),
                        context_type_identity: codec.context_type_identity.clone(),
                    }),
                payload_segments: receive
                    .payload_segments
                    .iter()
                    .map(|segment| WebSocketPayloadSegment {
                        kind: match segment.kind {
                            RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader::Context => {
                                WebSocketPayloadSegmentKind::Context
                            }
                            RuntimeAssemblyWebSocketPayloadSegmentKindFrameHeader::Message => {
                                WebSocketPayloadSegmentKind::Message
                            }
                        },
                        offset: usize::try_from(segment.offset)
                            .expect("wire safe integer fits the runtime host target"),
                        length: usize::try_from(segment.length)
                            .expect("wire safe integer fits the runtime host target"),
                    })
                    .collect(),
            }),
    }
}

fn name_values(
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

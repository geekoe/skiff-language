use serde_json::{Map, Value};
use skiff_artifact_model::{IngressProtocol, IngressSelector};
use skiff_runtime_request::{RequestEnvelope, ResponseEvent, RouterWriterMessage};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestStartFrameHeader,
};
use tokio::sync::mpsc;
use tracing::error;
use url::{Position, Url};

use super::response_event_into_transport_message;
use crate::{
    capability_context::response_error_from_runtime_error,
    error::{Result, RuntimeError},
    host::RuntimeHost,
    loader::assembly_admission::ActiveAssemblyRoute,
};

impl RuntimeHost {
    pub(crate) async fn spawn_runtime_assembly_request(
        &self,
        header: RuntimeAssemblyRequestStartFrameHeader,
        payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let request_id = header.request_id.clone();
        match self.runtime_assembly_request_from_wire(header, payload) {
            Ok((route, request)) => {
                self.spawn_request_on_active_assembly_route(route, request, sender)
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
        header: RuntimeAssemblyRequestStartFrameHeader,
        payload: Vec<u8>,
    ) -> Result<(ActiveAssemblyRoute, RequestEnvelope)> {
        validate_narrow_unary_header(&header)?;
        let selector = ingress_selector(&header)?;
        let route = self.lookup_active_assembly_request_route(&selector)?;
        validate_route(&header, &selector, &route)?;
        let request = request_envelope_from_route(header, payload, &route)?;
        Ok((route, request))
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
    if !matches!(
        header.routing.ingress.protocol,
        RuntimeAssemblyRequestIngressProtocol::Http
    ) {
        return Err(RuntimeError::Unsupported(
            "canonical unary bridge only accepts HTTP ingress".to_string(),
        ));
    }
    if header.http_adapter.is_some() || header.websocket_adapter.is_some() {
        return Err(RuntimeError::Unsupported(
            "canonical unary bridge does not accept gateway adapter metadata".to_string(),
        ));
    }
    if header.test_effects_enabled || !header.test_effect_doubles.is_empty() {
        return Err(RuntimeError::Unsupported(
            "canonical unary bridge does not accept test effects".to_string(),
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

fn ingress_selector(header: &RuntimeAssemblyRequestStartFrameHeader) -> Result<IngressSelector> {
    let ingress = &header.routing.ingress;
    let method = ingress.method.clone().ok_or_else(|| {
        RuntimeError::Decode("canonical HTTP routing ingress requires method".to_string())
    })?;
    Ok(IngressSelector {
        protocol: IngressProtocol::Http,
        host: ingress.host.clone(),
        method: Some(method),
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
        websocket_adapter: None,
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

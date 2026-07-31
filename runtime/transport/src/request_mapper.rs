use std::collections::HashMap;

use serde_json::Value;
use skiff_runtime_request_contract::{
    BinaryHttpRequest, BinaryHttpRequestMetadata, GatewayAdapterArg, GatewayAdapterSource,
    HttpAdapter, HttpAdapterCallable, HttpAdapterKind, HttpNameValue, RequestCancel,
    RequestEffectDouble, RequestEnvelope,
};

use crate::ingress_selector::ingress_selector_from_start_frame;
use crate::protocol::{
    RequestCancelFrameHeader, RequestStartFrameHeader, RequestTestEffectDouble,
    RuntimeGatewayAdapterArgFrameHeader, RuntimeGatewayAdapterSourceFrameHeader,
    RuntimeHttpAdapterCallableFrameHeader, RuntimeHttpAdapterFrameHeader,
    RuntimeHttpAdapterKindFrameHeader, RuntimeHttpNameValueFrameHeader,
    RuntimeHttpRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};

pub fn request_envelope_from_start_frame(
    header: RequestStartFrameHeader,
    payload_bytes: Vec<u8>,
) -> Result<RequestEnvelope, String> {
    if header.schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(format!(
            "request.start schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}, got {}",
            header.schema_version
        ));
    }
    if header.envelope_type != "request.start" {
        return Err(format!(
            "binary frame type must be request.start, got {}",
            header.envelope_type
        ));
    }
    if header.build_id.is_empty() {
        return Err("request.start buildId must be a non-empty string".to_string());
    }
    let ingress_selector = ingress_selector_from_start_frame(&header)?;
    Ok(RequestEnvelope {
        request_id: header.request_id.clone(),
        mode: header.mode.clone(),
        target: header.target.clone(),
        operation_abi_id: header.operation_abi_id.clone(),
        selector: header.selector.clone(),
        service_id: header.service_id.clone(),
        build_id: header.build_id.clone(),
        service_protocol_identity: header.service_protocol_identity.clone(),
        contract_identity: None,
        activation_identity: header.activation_identity.clone(),
        ingress_selector: Some(ingress_selector),
        binary_http: binary_http_request_from_frame(header.http_request.clone(), &payload_bytes),
        http_adapter: http_adapter_from_frame(header.http_adapter.clone()),
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: request_effect_doubles_from_frame(&header.test_effect_doubles),
        payload_bytes,
        extra: request_start_extra_from_frame(&header),
    })
}

pub fn request_cancel_from_frame_header(header: RequestCancelFrameHeader) -> RequestCancel {
    RequestCancel {
        request_id: header.request_id,
        reason: Some(header.reason),
    }
}

fn binary_http_request_from_frame(
    metadata: Option<RuntimeHttpRequestFrameHeader>,
    payload_bytes: &[u8],
) -> Option<BinaryHttpRequest> {
    metadata.map(|metadata| BinaryHttpRequest {
        metadata: BinaryHttpRequestMetadata {
            method: metadata.method,
            url: metadata.url,
            path: metadata.path,
            query: http_name_values_from_frame(metadata.query),
            headers: http_name_values_from_frame(metadata.headers),
        },
        body: payload_bytes.to_vec(),
    })
}

fn http_name_values_from_frame(items: Vec<RuntimeHttpNameValueFrameHeader>) -> Vec<HttpNameValue> {
    items
        .into_iter()
        .map(|item| HttpNameValue {
            name: item.name,
            value: item.value,
        })
        .collect()
}

fn http_adapter_from_frame(metadata: Option<RuntimeHttpAdapterFrameHeader>) -> Option<HttpAdapter> {
    metadata.map(|metadata| HttpAdapter {
        kind: match metadata.kind {
            RuntimeHttpAdapterKindFrameHeader::TypedJson => HttpAdapterKind::TypedJson,
            RuntimeHttpAdapterKindFrameHeader::RawHttp => HttpAdapterKind::RawHttp,
        },
        handler: http_adapter_callable_from_frame(metadata.handler),
        guard: metadata.guard.map(http_adapter_callable_from_frame),
        pre: metadata.pre.map(http_adapter_callable_from_frame),
        adapter_args: gateway_adapter_args_from_frame(metadata.adapter_args),
    })
}

fn http_adapter_callable_from_frame(
    callable: RuntimeHttpAdapterCallableFrameHeader,
) -> HttpAdapterCallable {
    match callable {
        RuntimeHttpAdapterCallableFrameHeader::ServiceFunction {
            module_path,
            symbol,
        } => HttpAdapterCallable::ServiceFunction {
            module_path,
            symbol,
        },
        RuntimeHttpAdapterCallableFrameHeader::PackageFunction {
            package_id,
            symbol_path,
        } => HttpAdapterCallable::PackageFunction {
            package_id,
            symbol_path,
        },
    }
}

fn gateway_adapter_args_from_frame(
    args: Vec<RuntimeGatewayAdapterArgFrameHeader>,
) -> Vec<GatewayAdapterArg> {
    args.into_iter()
        .map(|arg| GatewayAdapterArg {
            param: arg.param,
            source: gateway_adapter_source_from_frame(arg.source),
        })
        .collect()
}

fn gateway_adapter_source_from_frame(
    source: RuntimeGatewayAdapterSourceFrameHeader,
) -> GatewayAdapterSource {
    match source {
        RuntimeGatewayAdapterSourceFrameHeader::HttpRequest => GatewayAdapterSource::HttpRequest,
        RuntimeGatewayAdapterSourceFrameHeader::HttpBody => GatewayAdapterSource::HttpBody,
        RuntimeGatewayAdapterSourceFrameHeader::HttpContext => GatewayAdapterSource::HttpContext,
    }
}

fn request_effect_doubles_from_frame(
    doubles: &HashMap<String, Vec<RequestTestEffectDouble>>,
) -> HashMap<String, Vec<RequestEffectDouble>> {
    doubles
        .iter()
        .map(|(target, sequence)| {
            (
                target.clone(),
                sequence
                    .iter()
                    .map(|double| RequestEffectDouble {
                        expect_request: double.expect_request.clone(),
                        response: double.response.clone(),
                    })
                    .collect(),
            )
        })
        .collect()
}

fn request_start_extra_from_frame(
    header: &RequestStartFrameHeader,
) -> serde_json::Map<String, Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "caller".to_string(),
        serde_json::to_value(&header.caller).unwrap_or(Value::Null),
    );
    if let Some(gateway_entry_identity) = &header.gateway_entry_identity {
        extra.insert(
            "gatewayEntryIdentity".to_string(),
            Value::String(gateway_entry_identity.clone()),
        );
    }
    if let Some(service_id) = &header.service_id {
        extra.insert("serviceId".to_string(), Value::String(service_id.clone()));
    }
    if let Some(operation_abi_id) = &header.operation_abi_id {
        extra.insert(
            "operationAbiId".to_string(),
            Value::String(operation_abi_id.clone()),
        );
    }
    if let Some(selector) = &header.selector {
        extra.insert("selector".to_string(), Value::String(selector.clone()));
    }
    if let Some(client_session) = &header.client_session {
        extra.insert(
            "clientSession".to_string(),
            serde_json::to_value(client_session).unwrap_or(Value::Null),
        );
    }
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline).unwrap_or(Value::Null),
        );
    }
    extra.insert(
        "trace".to_string(),
        serde_json::to_value(&header.trace).unwrap_or(Value::Null),
    );
    extra
}

#[cfg(test)]
mod tests;

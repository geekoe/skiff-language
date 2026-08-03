use serde_json::{Map, Value};
pub use skiff_runtime_capability_context::{
    BinaryHttpRequestContext, HttpNameValueContext, InvocationContext, RequestPayloadContext,
    RequestPayloadEncoding,
};

use crate::{RequestEnvelope, RuntimeOperation};

pub fn request_payload_context_from_request<'a>(
    request: &'a RequestEnvelope,
) -> RequestPayloadContext<'a> {
    RequestPayloadContext::new(
        request.target.as_str(),
        request.payload_bytes.as_slice(),
        request.binary_http.as_ref().map(|binary_http| {
            BinaryHttpRequestContext::new(
                binary_http.metadata.method.as_str(),
                binary_http.metadata.url.as_str(),
                binary_http.metadata.path.as_str(),
                http_name_value_contexts(&binary_http.metadata.query),
                http_name_value_contexts(&binary_http.metadata.headers),
                binary_http.body.as_slice(),
            )
        }),
    )
    .with_payload_encoding(request_payload_encoding(request))
}

pub fn invocation_context_from_request<'a>(
    runtime_id: &'a str,
    service_id: &'a str,
    service_version: &'a str,
    request: &'a RequestEnvelope,
    operation: &'a RuntimeOperation,
) -> InvocationContext<'a> {
    InvocationContext::new(
        runtime_id,
        service_id,
        service_version,
        request.request_id.as_str(),
        request.target.as_str(),
        request.build_id.as_str(),
        request.service_protocol_identity.as_str(),
        operation.service_protocol_identity.as_deref(),
        request.activation_identity.as_deref(),
        request_trace_id(&request.extra),
    )
}

fn http_name_value_contexts(items: &[crate::HttpNameValue]) -> Vec<HttpNameValueContext<'_>> {
    items
        .iter()
        .map(|item| HttpNameValueContext::new(item.name.as_str(), item.value.as_str()))
        .collect()
}

fn request_trace_id(extra: &Map<String, Value>) -> Option<&str> {
    extra
        .get("trace")
        .and_then(Value::as_object)
        .and_then(|trace| trace.get("traceId"))
        .and_then(Value::as_str)
}

fn request_payload_encoding(request: &RequestEnvelope) -> RequestPayloadEncoding {
    if request
        .extra
        .get("caller")
        .and_then(Value::as_object)
        .and_then(|caller| caller.get("kind"))
        .and_then(Value::as_str)
        == Some("task")
    {
        RequestPayloadEncoding::RecoverableTaskDispatchPayload
    } else {
        RequestPayloadEncoding::RuntimeBinary
    }
}

#[cfg(test)]
mod tests;

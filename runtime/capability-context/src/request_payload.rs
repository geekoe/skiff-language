use std::{error::Error, fmt};

use skiff_runtime_boundary::http::{HttpBoundaryNameValue, HttpBoundaryRequestParts};
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

const MISSING_BINARY_HTTP_MESSAGE: &str = "binary HTTP request metadata is missing";

#[derive(Clone)]
pub struct RequestPayloadContext<'a> {
    target: &'a str,
    payload_bytes: &'a [u8],
    binary_http: Option<BinaryHttpRequestContext<'a>>,
    payload_encoding: RequestPayloadEncoding,
}

impl<'a> RequestPayloadContext<'a> {
    pub fn new(
        target: &'a str,
        payload_bytes: &'a [u8],
        binary_http: Option<BinaryHttpRequestContext<'a>>,
    ) -> Self {
        Self {
            target,
            payload_bytes,
            binary_http,
            payload_encoding: RequestPayloadEncoding::RuntimeBinary,
        }
    }

    pub fn with_payload_encoding(mut self, payload_encoding: RequestPayloadEncoding) -> Self {
        self.payload_encoding = payload_encoding;
        self
    }

    pub fn target(&self) -> &'a str {
        self.target
    }

    pub fn payload_bytes(&self) -> &'a [u8] {
        self.payload_bytes
    }

    pub fn payload_encoding(&self) -> RequestPayloadEncoding {
        self.payload_encoding
    }

    pub fn binary_http(&self) -> Option<&BinaryHttpRequestContext<'a>> {
        self.binary_http.as_ref()
    }

    pub fn has_binary_http(&self) -> bool {
        self.binary_http.is_some()
    }

    pub fn require_binary_http(
        &self,
    ) -> Result<&BinaryHttpRequestContext<'a>, RequestPayloadContextError> {
        self.binary_http
            .as_ref()
            .ok_or_else(|| RequestPayloadContextError::MissingBinaryHttp {
                target: self.target.to_string(),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPayloadEncoding {
    RuntimeBinary,
    RecoverableSpawnPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestPayloadContextError {
    MissingBinaryHttp { target: String },
}

impl fmt::Display for RequestPayloadContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingBinaryHttp { .. } => formatter.write_str(MISSING_BINARY_HTTP_MESSAGE),
        }
    }
}

impl Error for RequestPayloadContextError {}

impl WirePayload for RequestPayloadContextError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::MissingBinaryHttp { target } => RuntimeErrorPayload {
                code: "std.service.ProtocolError".to_string(),
                message: MISSING_BINARY_HTTP_MESSAGE.to_string(),
                status: None,
                details: Some(serde_json::json!({
                    "target": target,
                    "message": MISSING_BINARY_HTTP_MESSAGE,
                })),
            },
        }
    }

    fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
        match self {
            Self::MissingBinaryHttp { target } => Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                serde_json::json!({
                    "target": target,
                    "message": MISSING_BINARY_HTTP_MESSAGE,
                }),
            )),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Clone)]
pub struct BinaryHttpRequestContext<'a> {
    method: &'a str,
    url: &'a str,
    path: &'a str,
    query: Vec<HttpNameValueContext<'a>>,
    headers: Vec<HttpNameValueContext<'a>>,
    body: &'a [u8],
}

impl<'a> BinaryHttpRequestContext<'a> {
    pub fn new(
        method: &'a str,
        url: &'a str,
        path: &'a str,
        query: Vec<HttpNameValueContext<'a>>,
        headers: Vec<HttpNameValueContext<'a>>,
        body: &'a [u8],
    ) -> Self {
        Self {
            method,
            url,
            path,
            query,
            headers,
            body,
        }
    }

    pub fn method(&self) -> &'a str {
        self.method
    }

    pub fn url(&self) -> &'a str {
        self.url
    }

    pub fn path(&self) -> &'a str {
        self.path
    }

    pub fn query(&self) -> &[HttpNameValueContext<'a>] {
        self.query.as_slice()
    }

    pub fn headers(&self) -> &[HttpNameValueContext<'a>] {
        self.headers.as_slice()
    }

    pub fn body(&self) -> &'a [u8] {
        self.body
    }
}

pub fn binary_http_request_parts(
    binary_http: &BinaryHttpRequestContext<'_>,
) -> HttpBoundaryRequestParts {
    HttpBoundaryRequestParts {
        method: binary_http.method().to_string(),
        url: binary_http.url().to_string(),
        path: binary_http.path().to_string(),
        query: http_name_value_contexts(binary_http.query()),
        headers: http_name_value_contexts(binary_http.headers()),
        body: binary_http.body().to_vec(),
    }
}

#[derive(Clone, Copy)]
pub struct HttpNameValueContext<'a> {
    name: &'a str,
    value: &'a str,
}

impl<'a> HttpNameValueContext<'a> {
    pub fn new(name: &'a str, value: &'a str) -> Self {
        Self { name, value }
    }

    pub fn name(&self) -> &'a str {
        self.name
    }

    pub fn value(&self) -> &'a str {
        self.value
    }
}

pub fn http_name_value_contexts(items: &[HttpNameValueContext<'_>]) -> Vec<HttpBoundaryNameValue> {
    items.iter().map(http_name_value_context).collect()
}

pub fn http_name_value_context(item: &HttpNameValueContext<'_>) -> HttpBoundaryNameValue {
    HttpBoundaryNameValue {
        name: item.name().to_string(),
        value: item.value().to_string(),
    }
}

#[derive(Clone, Copy)]
pub struct InvocationContext<'a> {
    runtime_id: &'a str,
    service_id: &'a str,
    service_version: &'a str,
    request_id: &'a str,
    request_target: &'a str,
    request_build_id: &'a str,
    request_service_protocol_identity: &'a str,
    operation_service_protocol_identity: Option<&'a str>,
    activation_identity: Option<&'a str>,
    trace_id: Option<&'a str>,
}

impl<'a> InvocationContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime_id: &'a str,
        service_id: &'a str,
        service_version: &'a str,
        request_id: &'a str,
        request_target: &'a str,
        request_build_id: &'a str,
        request_service_protocol_identity: &'a str,
        operation_service_protocol_identity: Option<&'a str>,
        activation_identity: Option<&'a str>,
        trace_id: Option<&'a str>,
    ) -> Self {
        Self {
            runtime_id,
            service_id,
            service_version,
            request_id,
            request_target,
            request_build_id,
            request_service_protocol_identity,
            operation_service_protocol_identity,
            activation_identity,
            trace_id,
        }
    }

    pub fn runtime_id(&self) -> &'a str {
        self.runtime_id
    }

    pub fn service_id(&self) -> &'a str {
        self.service_id
    }

    pub fn service_version(&self) -> &'a str {
        self.service_version
    }

    pub fn request_id(&self) -> &'a str {
        self.request_id
    }

    pub fn request_target(&self) -> &'a str {
        self.request_target
    }

    pub fn request_build_id(&self) -> &'a str {
        self.request_build_id
    }

    pub fn actor_service_protocol_identity(&self) -> &'a str {
        self.request_service_protocol_identity
    }

    pub fn spawn_service_protocol_identity(&self) -> &'a str {
        self.operation_service_protocol_identity
            .unwrap_or(self.request_service_protocol_identity)
    }

    pub fn activation_identity(&self) -> Option<&'a str> {
        self.activation_identity
    }

    pub fn trace_id(&self) -> Option<&'a str> {
        self.trace_id
    }
}

use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

pub use skiff_runtime_capability_context::HttpNameValue;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RequestEnvelope {
    pub request_id: String,
    pub mode: String,
    pub target: String,
    pub operation_abi_id: Option<String>,
    pub selector: Option<String>,
    pub service_id: Option<String>,
    pub build_id: String,
    pub service_protocol_identity: String,
    pub contract_identity: Option<String>,
    pub activation_identity: Option<String>,
    pub binary_http: Option<BinaryHttpRequest>,
    pub http_adapter: Option<HttpAdapter>,
    pub websocket_adapter: Option<WebSocketAdapter>,
    pub test_effects_enabled: bool,
    pub test_effect_doubles: HashMap<String, Vec<RequestEffectDouble>>,
    pub payload_bytes: Vec<u8>,
    pub extra: serde_json::Map<String, Value>,
}

impl RequestEnvelope {
    pub fn build_id(&self) -> &str {
        &self.build_id
    }
}

impl<'de> Deserialize<'de> for RequestEnvelope {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let _ = Value::deserialize(deserializer)?;
        Err(serde::de::Error::custom(
            "text protocol messages are not supported on runtime WebSocket; use binary runtime frames",
        ))
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BinaryHttpRequest {
    pub metadata: BinaryHttpRequestMetadata,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinaryHttpRequestMetadata {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<HttpNameValue>,
    pub headers: Vec<HttpNameValue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpAdapter {
    pub kind: HttpAdapterKind,
    pub handler: HttpAdapterCallable,
    pub guard: Option<HttpAdapterCallable>,
    pub pre: Option<HttpAdapterCallable>,
    pub adapter_args: Vec<GatewayAdapterArg>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpAdapterKind {
    TypedJson,
    RawHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HttpAdapterCallable {
    ServiceFunction {
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        package_id: String,
        symbol_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayAdapterArg {
    pub param: String,
    pub source: GatewayAdapterSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayAdapterSource {
    HttpRequest,
    HttpBody,
    HttpContext,
    WebSocketConnectRequest,
    WebSocketReceiveEvent,
    WebSocketConnection,
    WebSocketConnectionContext,
    WebSocketMessage,
    WebSocketMessageBody,
    WebSocketConnectionId,
    WebSocketBusinessIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketAdapter {
    pub kind: WebSocketAdapterKind,
    pub adapter_args: Vec<GatewayAdapterArg>,
    pub context_expectation: Option<WebSocketContextExpectation>,
    pub connect_request: Option<WebSocketConnectRequest>,
    pub receive_request: Option<WebSocketReceiveRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketAdapterKind {
    Connect,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketContextExpectation {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketContextCodec {
    pub operation_abi_id: String,
    pub context_type_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketConnectRequest {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<HttpNameValue>,
    pub headers: Vec<HttpNameValue>,
    pub cookies: Vec<HttpNameValue>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketReceiveRequest {
    pub connection_id: String,
    pub business_identity: Option<String>,
    pub message: WebSocketMessage,
    pub context_codec: Option<WebSocketContextCodec>,
    pub payload_segments: Vec<WebSocketPayloadSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketMessage {
    pub tag: WebSocketMessageTag,
    pub encoding: WebSocketMessageEncoding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketMessageTag {
    Text,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketMessageEncoding {
    Utf8,
    Raw,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSocketPayloadSegment {
    pub kind: WebSocketPayloadSegmentKind,
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketPayloadSegmentKind {
    Context,
    Message,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestCancel {
    pub request_id: String,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestEffectDouble {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_request: Option<Value>,
    pub response: Value,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::RequestEnvelope;

    #[test]
    fn request_start_text_json_deserialize_fails_closed() {
        let error = serde_json::from_value::<RequestEnvelope>(json!({
            "requestId": "request-1",
            "mode": "unary",
            "target": "service.example~com~~service-a.Api.hello",
            "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "args": {}
        }))
        .expect_err("text protocol request.start should fail closed");

        assert!(error
            .to_string()
            .contains("text protocol messages are not supported on runtime WebSocket"));
    }
}

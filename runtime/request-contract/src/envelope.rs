use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_artifact_model::IngressSelector;

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
    /// Canonical external route identity projected strictly from gateway wire metadata.
    /// Legacy package-test/Phase 05 callers may omit it, but production ingress fails closed.
    pub ingress_selector: Option<IngressSelector>,
    pub binary_http: Option<BinaryHttpRequest>,
    pub http_adapter: Option<HttpAdapter>,
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
mod tests;

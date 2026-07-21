use std::collections::HashMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{AssemblyIdentity, ContractOperationId};
use skiff_runtime_request_contract::RuntimeClientSessionControl;

use crate::{
    protocol::{
        decode_typed_binary_frame, RequestTestEffectDouble, RuntimeDeadlineFrameHeader,
        RuntimeHttpAdapterFrameHeader, RuntimeHttpRequestFrameHeader,
        RuntimeTraceContextFrameHeader, RuntimeWebSocketAdapterFrameHeader,
    },
    BinaryFrameError,
};

mod lexical;

use lexical::{
    deserialize_assembly_identity, deserialize_contract_operation_id, deserialize_dispatch_mode,
    deserialize_gateway_caller_kind, deserialize_request_start_type,
    deserialize_required_nullable_string, deserialize_runtime_assembly_routing_kind,
    deserialize_runtime_frame_schema_version, deserialize_safe_activation_generation,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyRequestRoutingFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_entry_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_session: Option<RuntimeClientSessionControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<RuntimeDeadlineFrameHeader>,
    pub trace: RuntimeTraceContextFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_request: Option<RuntimeHttpRequestFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_adapter: Option<RuntimeHttpAdapterFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_adapter: Option<RuntimeWebSocketAdapterFrameHeader>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub test_effects_enabled: bool,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub test_effect_doubles: HashMap<String, Vec<RequestTestEffectDouble>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestCallerFrameHeader {
    #[serde(deserialize_with = "deserialize_gateway_caller_kind")]
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_contract_operation_id")]
    pub contract_operation_id: ContractOperationId,
    pub ingress: RuntimeAssemblyRequestIngressFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestIngressFrameHeader {
    pub protocol: RuntimeAssemblyRequestIngressProtocol,
    pub host: String,
    pub method: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyRequestIngressProtocol {
    #[serde(rename = "http")]
    Http,
    #[serde(rename = "webSocket")]
    WebSocket,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyRequestIngressFrameHeader {
    protocol: RuntimeAssemblyRequestIngressProtocol,
    host: String,
    #[serde(deserialize_with = "deserialize_required_nullable_string")]
    method: Option<String>,
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRequestIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyRequestIngressFrameHeader::deserialize(deserializer)?;
        if raw.host.is_empty() || !raw.path.starts_with('/') {
            return Err(de::Error::custom(
                "routing.ingress must carry a non-empty host and absolute path",
            ));
        }
        match (raw.protocol, raw.method.as_deref()) {
            (RuntimeAssemblyRequestIngressProtocol::Http, Some(method)) if !method.is_empty() => {}
            (RuntimeAssemblyRequestIngressProtocol::WebSocket, None) => {}
            _ => {
                return Err(de::Error::custom(
                    "routing.ingress.method does not match protocol",
                ))
            }
        }
        Ok(Self {
            protocol: raw.protocol,
            host: raw.host,
            method: raw.method,
            path: raw.path,
        })
    }
}

pub fn decode_runtime_assembly_request_start_frame(
    frame: &[u8],
) -> Result<(RuntimeAssemblyRequestStartFrameHeader, Vec<u8>), BinaryFrameError> {
    decode_typed_binary_frame(frame)
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
#[path = "runtime_assembly_request/tests.rs"]
mod tests;

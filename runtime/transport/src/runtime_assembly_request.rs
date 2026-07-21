use std::collections::HashMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{AssemblyIdentity, ContractOperationId};

use crate::{BinaryFrameError, TransportError};

mod lexical;
mod metadata;
mod strict_json;

use lexical::{
    deserialize_assembly_identity, deserialize_contract_operation_id, deserialize_dispatch_mode,
    deserialize_gateway_caller_kind, deserialize_request_start_type,
    deserialize_required_nullable_string, deserialize_runtime_assembly_routing_kind,
    deserialize_runtime_frame_schema_version, deserialize_safe_activation_generation,
};
pub use metadata::*;
use metadata::{
    deserialize_optional_activation_identity, deserialize_optional_gateway_entry_identity,
    deserialize_optional_http_adapter, deserialize_optional_websocket_adapter,
    deserialize_present_option, deserialize_test_effect_doubles,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "RawRuntimeAssemblyRequestStartFrameHeader"
)]
pub struct RuntimeAssemblyRequestStartFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub frame_type: String,
    pub request_id: String,
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyRequestRoutingFrameHeader,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gateway_entry_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_request: Option<RuntimeAssemblyHttpRequestFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_adapter: Option<RuntimeAssemblyHttpAdapterFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_adapter: Option<RuntimeAssemblyWebSocketAdapterFrameHeader>,
    #[serde(skip_serializing_if = "is_false")]
    pub test_effects_enabled: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub test_effect_doubles:
        HashMap<String, Vec<RuntimeAssemblyRequestTestEffectDoubleFrameHeader>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    frame_type: String,
    request_id: String,
    #[serde(deserialize_with = "deserialize_dispatch_mode")]
    mode: String,
    caller: RuntimeAssemblyRequestCallerFrameHeader,
    routing: RuntimeAssemblyRequestRoutingFrameHeader,
    #[serde(default, deserialize_with = "deserialize_optional_activation_identity")]
    activation_identity: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_gateway_entry_identity"
    )]
    gateway_entry_identity: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    business_identity: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    websocket_entry_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    trace: RuntimeAssemblyRequestTraceFrameHeader,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    http_request: Option<RuntimeAssemblyHttpRequestFrameHeader>,
    #[serde(default, deserialize_with = "deserialize_optional_http_adapter")]
    http_adapter: Option<RuntimeAssemblyHttpAdapterFrameHeader>,
    #[serde(default, deserialize_with = "deserialize_optional_websocket_adapter")]
    websocket_adapter: Option<RuntimeAssemblyWebSocketAdapterFrameHeader>,
    #[serde(default)]
    test_effects_enabled: bool,
    #[serde(default, deserialize_with = "deserialize_test_effect_doubles")]
    test_effect_doubles: HashMap<String, Vec<RuntimeAssemblyRequestTestEffectDoubleFrameHeader>>,
}

impl TryFrom<RawRuntimeAssemblyRequestStartFrameHeader> for RuntimeAssemblyRequestStartFrameHeader {
    type Error = String;

    fn try_from(raw: RawRuntimeAssemblyRequestStartFrameHeader) -> Result<Self, Self::Error> {
        if raw.websocket_adapter.is_some()
            && (raw.websocket_entry_id.is_none() || raw.gateway_entry_identity.is_none())
        {
            return Err(
                "websocketAdapter requires websocketEntryId and gatewayEntryIdentity".to_string(),
            );
        }
        Ok(Self {
            schema_version: raw.schema_version,
            frame_type: raw.frame_type,
            request_id: raw.request_id,
            mode: raw.mode,
            caller: raw.caller,
            routing: raw.routing,
            activation_identity: raw.activation_identity,
            gateway_entry_identity: raw.gateway_entry_identity,
            business_identity: raw.business_identity,
            websocket_entry_id: raw.websocket_entry_id,
            client_session: raw.client_session,
            deadline: raw.deadline,
            trace: raw.trace,
            http_request: raw.http_request,
            http_adapter: raw.http_adapter,
            websocket_adapter: raw.websocket_adapter,
            test_effects_enabled: raw.test_effects_enabled,
            test_effect_doubles: raw.test_effect_doubles,
        })
    }
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
    let (header, payload) = strict_json::decode_runtime_assembly_request_json_frame(frame)?;
    let header = serde_json::from_value(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header failed typed decode: {error}"
        ))
    })?;
    Ok((header, payload))
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
#[path = "runtime_assembly_request/tests.rs"]
mod tests;

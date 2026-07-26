use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{AssemblyIdentity, GatewayEntryIdentity};

use crate::{BinaryFrameError, TransportError};

mod lexical;
mod metadata;
mod strict_json;

use lexical::{
    deserialize_assembly_identity, deserialize_dispatch_mode, deserialize_gateway_caller_kind,
    deserialize_gateway_entry_identity, deserialize_request_start_type,
    deserialize_runtime_assembly_routing_kind, deserialize_runtime_frame_schema_version,
    deserialize_safe_activation_generation,
};
use metadata::deserialize_present_option;
pub use metadata::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub client_session: Option<RuntimeAssemblyRequestClientSessionFrameHeader>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    pub trace: RuntimeAssemblyRequestTraceFrameHeader,
    pub http_request: RuntimeAssemblyHttpRequestFrameHeader,
    #[serde(default, skip_serializing_if = "is_false")]
    pub test_effects_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestCallerFrameHeader {
    #[serde(deserialize_with = "deserialize_gateway_caller_kind")]
    pub kind: String,
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
    #[serde(deserialize_with = "deserialize_gateway_entry_identity")]
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub ingress: RuntimeAssemblyRequestIngressFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRequestIngressFrameHeader {
    pub protocol: RuntimeAssemblyRequestIngressProtocol,
    pub host: String,
    pub method: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyRequestIngressProtocol {
    #[serde(rename = "http")]
    Http,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyRequestIngressFrameHeader {
    protocol: RuntimeAssemblyRequestIngressProtocol,
    host: String,
    method: String,
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRequestIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyRequestIngressFrameHeader::deserialize(deserializer)?;
        if raw.host.is_empty() {
            return Err(de::Error::custom(
                "routing.ingress.host must be a non-empty string",
            ));
        }
        if raw.method.is_empty() {
            return Err(de::Error::custom(
                "routing.ingress.method must be a non-empty string",
            ));
        }
        if !raw.path.starts_with('/') {
            return Err(de::Error::custom(
                "routing.ingress.path must be an absolute path",
            ));
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
    let header: RuntimeAssemblyRequestStartFrameHeader =
        serde_json::from_value(header).map_err(|error| {
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

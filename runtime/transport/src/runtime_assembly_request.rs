use std::num::NonZeroU32;

use serde::{de, Deserialize, Deserializer, Serialize};
use skiff_artifact_model::{AssemblyIdentity, GatewayEntryIdentity, WebSocketEntryId};

use crate::{BinaryFrameError, TransportError};

mod lexical;
mod metadata;
mod strict_json;

use lexical::{
    deserialize_assembly_identity, deserialize_dispatch_mode, deserialize_gateway_caller_kind,
    deserialize_gateway_entry_identity, deserialize_request_start_type,
    deserialize_response_end_type, deserialize_runtime_assembly_routing_kind,
    deserialize_runtime_frame_schema_version, deserialize_safe_activation_generation,
    deserialize_unary_dispatch_mode,
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
    #[serde(default)]
    pub test_effects_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum RuntimeAssemblyRequestStartFrameWireHeader {
    Http(RuntimeAssemblyRequestStartFrameHeader),
    WebSocketConnect(RuntimeAssemblyWebSocketConnectRequestStartFrameHeader),
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRequestStartFrameWireHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let protocol = value
            .get("routing")
            .and_then(|routing| routing.get("ingress"))
            .and_then(|ingress| ingress.get("protocol"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                de::Error::custom("request.start routing.ingress.protocol is required")
            })?;
        match protocol {
            "http" => serde_json::from_value(value)
                .map(Self::Http)
                .map_err(de::Error::custom),
            "webSocket" => serde_json::from_value(value)
                .map(Self::WebSocketConnect)
                .map_err(de::Error::custom),
            _ => Err(de::Error::custom(
                "request.start routing.ingress.protocol must be http or webSocket",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    pub schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_request_start_type")]
    pub frame_type: String,
    pub request_id: String,
    #[serde(deserialize_with = "deserialize_unary_dispatch_mode")]
    pub mode: String,
    pub caller: RuntimeAssemblyRequestCallerFrameHeader,
    pub routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
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
    pub websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader,
    #[serde(default)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_assembly_routing_kind")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_assembly_identity")]
    pub assembly_identity: AssemblyIdentity,
    #[serde(deserialize_with = "deserialize_safe_activation_generation")]
    pub assembly_generation: u64,
    #[serde(deserialize_with = "deserialize_gateway_entry_identity")]
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeAssemblyWebSocketConnectIngressProtocol {
    #[serde(rename = "webSocket")]
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectIngressFrameHeader {
    pub protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    pub host: String,
    pub method: (),
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectIngressFrameHeader {
    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol,
    host: String,
    method: (),
    path: String,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketConnectIngressFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyWebSocketConnectIngressFrameHeader::deserialize(deserializer)?;
        if raw.host.is_empty() {
            return Err(de::Error::custom(
                "routing.ingress.host must be a non-empty string",
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectRequestFrameHeader {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    pub cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub websocket_entry_id: WebSocketEntryId,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectRequestFrameHeader {
    connection_id: String,
    url: String,
    query: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    headers: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    cookies: Vec<RuntimeAssemblyRequestNameValueFrameHeader>,
    #[serde(default, deserialize_with = "deserialize_present_option")]
    version: Option<String>,
    websocket_entry_id: WebSocketEntryId,
    gateway_entry_identity: GatewayEntryIdentity,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyWebSocketConnectRequestFrameHeader {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyWebSocketConnectRequestFrameHeader::deserialize(deserializer)?;
        if raw.connection_id.is_empty()
            || raw.connection_id.len() > 255
            || !raw.connection_id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'~' | b'-')
            })
        {
            return Err(de::Error::custom(
                "websocketConnect.connectionId is not canonical",
            ));
        }
        Ok(Self {
            connection_id: raw.connection_id,
            url: raw.url,
            query: raw.query,
            headers: raw.headers,
            cookies: raw.cookies,
            version: raw.version,
            websocket_entry_id: raw.websocket_entry_id,
            gateway_entry_identity: raw.gateway_entry_identity,
        })
    }
}

pub fn decode_runtime_assembly_request_start_frame(
    frame: &[u8],
) -> Result<(RuntimeAssemblyRequestStartFrameWireHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload) = strict_json::decode_runtime_assembly_request_json_frame(frame)?;
    let header: RuntimeAssemblyRequestStartFrameWireHeader = serde_json::from_value(header)
        .map_err(|error| {
            TransportError::decode(format!(
                "invalid skiff binary frame: header failed typed decode: {error}"
            ))
        })?;
    if let RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(websocket) = &header {
        if websocket.websocket_connect.gateway_entry_identity
            != websocket.routing.gateway_entry_identity
        {
            return Err(TransportError::decode(
                "invalid runtimeAssembly websocketConnect request.start frame: websocketConnect.gatewayEntryIdentity must match routing.gatewayEntryIdentity",
            ));
        }
        if !payload.is_empty() {
            return Err(TransportError::decode(
                "invalid runtimeAssembly websocketConnect request.start frame: payload must be empty",
            ));
        }
    }
    Ok((header, payload))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum RuntimeAssemblyWebSocketConnectResponseFrameHeader {
    #[serde(rename = "accept")]
    Accept {
        #[serde(
            default,
            deserialize_with = "deserialize_present_option",
            skip_serializing_if = "Option::is_none"
        )]
        business_identity: Option<String>,
        #[serde(
            default,
            deserialize_with = "deserialize_present_option",
            skip_serializing_if = "Option::is_none"
        )]
        connection_policy: Option<RuntimeAssemblyWebSocketConnectionPolicyFrameHeader>,
    },
    #[serde(rename = "reject")]
    Reject { code: u16, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyWebSocketConnectionPolicyFrameHeader {
    pub max_connections: NonZeroU32,
    pub overflow: RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub close_code: Option<u16>,
    #[serde(
        default,
        deserialize_with = "deserialize_present_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub close_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader {
    CloseOldest,
    RejectNew,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    try_from = "RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader",
    into = "RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader"
)]
pub struct RuntimeAssemblyWebSocketConnectResponseEndFrameHeader {
    pub schema_version: String,
    pub frame_type: String,
    pub request_id: String,
    pub websocket_connect: RuntimeAssemblyWebSocketConnectResponseFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader {
    #[serde(deserialize_with = "deserialize_runtime_frame_schema_version")]
    schema_version: String,
    #[serde(rename = "type", deserialize_with = "deserialize_response_end_type")]
    frame_type: String,
    request_id: String,
    payload_present: bool,
    websocket_connect: RuntimeAssemblyWebSocketConnectResponseFrameHeader,
}

impl TryFrom<RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader>
    for RuntimeAssemblyWebSocketConnectResponseEndFrameHeader
{
    type Error = String;

    fn try_from(
        raw: RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader,
    ) -> Result<Self, Self::Error> {
        if raw.payload_present {
            return Err(
                "runtimeAssembly websocketConnect response.end payloadPresent must be false"
                    .to_string(),
            );
        }
        Ok(Self {
            schema_version: raw.schema_version,
            frame_type: raw.frame_type,
            request_id: raw.request_id,
            websocket_connect: raw.websocket_connect,
        })
    }
}

impl From<RuntimeAssemblyWebSocketConnectResponseEndFrameHeader>
    for RawRuntimeAssemblyWebSocketConnectResponseEndFrameHeader
{
    fn from(header: RuntimeAssemblyWebSocketConnectResponseEndFrameHeader) -> Self {
        Self {
            schema_version: header.schema_version,
            frame_type: header.frame_type,
            request_id: header.request_id,
            payload_present: false,
            websocket_connect: header.websocket_connect,
        }
    }
}

pub fn decode_runtime_assembly_websocket_connect_response_end_frame(
    frame: &[u8],
) -> Result<RuntimeAssemblyWebSocketConnectResponseEndFrameHeader, BinaryFrameError> {
    let (header, payload) = strict_json::decode_runtime_assembly_json_frame(
        frame,
        "runtimeAssembly websocketConnect response.end",
    )?;
    if !payload.is_empty() {
        return Err(TransportError::decode(
            "invalid runtimeAssembly websocketConnect response.end frame: payload must be empty",
        ));
    }
    serde_json::from_value(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid runtimeAssembly websocketConnect response.end frame: header failed typed decode: {error}"
        ))
    })
}

#[cfg(test)]
#[path = "runtime_assembly_request/tests.rs"]
mod tests;

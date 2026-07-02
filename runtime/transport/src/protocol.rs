use std::{collections::HashMap, path::PathBuf};

use crate::TransportError;
use serde::{de, de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_artifact_model::ConfigShape;
use skiff_runtime_request_contract::{
    RuntimeClientSessionControl, WebSocketConnectionPolicyControl,
};

pub const BINARY_FRAME_MAGIC: [u8; 4] = *b"SKBF";
pub const BINARY_FRAME_VERSION: u8 = 1;
pub const BINARY_FRAME_HEADER_ENCODING_JSON: u8 = 1;
pub const RUNTIME_FRAME_SCHEMA_VERSION: &str = "skiff-runtime-frame-v1";

const BINARY_FRAME_FIXED_HEADER_BYTES: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryFrame {
    pub header: Value,
    pub payload_bytes: Vec<u8>,
}

pub use crate::BinaryFrameError;

pub fn encode_binary_frame<THeader: Serialize>(
    header: &THeader,
    payload_bytes: &[u8],
) -> std::result::Result<Vec<u8>, BinaryFrameError> {
    let header_bytes = serde_json::to_vec(header).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header serialization failed: {error}"
        ))
    })?;
    if header_bytes.is_empty() {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must not be empty",
        ));
    }
    if header_bytes.len() > u32::MAX as usize {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header length exceeds u32",
        ));
    }
    if payload_bytes.len() > u32::MAX as usize {
        return Err(TransportError::decode(
            "invalid skiff binary frame: payload length exceeds u32",
        ));
    }

    let mut frame = Vec::with_capacity(
        BINARY_FRAME_FIXED_HEADER_BYTES + header_bytes.len() + payload_bytes.len(),
    );
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header_bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload_bytes.len() as u32).to_be_bytes());
    frame.extend_from_slice(&header_bytes);
    frame.extend_from_slice(payload_bytes);
    Ok(frame)
}

pub fn decode_binary_frame(frame: &[u8]) -> std::result::Result<BinaryFrame, BinaryFrameError> {
    if frame.len() < BINARY_FRAME_FIXED_HEADER_BYTES {
        return Err(TransportError::decode(
            "invalid skiff binary frame: frame is too short",
        ));
    }
    if frame[0..4] != BINARY_FRAME_MAGIC {
        return Err(TransportError::decode(
            "invalid skiff binary frame: expected skiff binary frame magic",
        ));
    }
    let version = frame[4];
    if version != BINARY_FRAME_VERSION {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported frame version {version}"
        )));
    }
    let header_encoding = frame[5];
    if header_encoding != BINARY_FRAME_HEADER_ENCODING_JSON {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: unsupported header encoding {header_encoding}"
        )));
    }

    let header_length = u32::from_be_bytes([frame[6], frame[7], frame[8], frame[9]]) as usize;
    let payload_length = u32::from_be_bytes([frame[10], frame[11], frame[12], frame[13]]) as usize;
    if header_length == 0 {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must not be empty",
        ));
    }
    let expected_length = BINARY_FRAME_FIXED_HEADER_BYTES
        .checked_add(header_length)
        .and_then(|length| length.checked_add(payload_length))
        .ok_or_else(|| {
            TransportError::decode("invalid skiff binary frame: frame length overflow")
        })?;
    if frame.len() != expected_length {
        return Err(TransportError::decode(format!(
            "invalid skiff binary frame: frame length {} does not match header length {} plus payload length {}",
            frame.len(),
            header_length,
            payload_length
        )));
    }

    let header_start = BINARY_FRAME_FIXED_HEADER_BYTES;
    let payload_start = header_start + header_length;
    let header: Value =
        serde_json::from_slice(&frame[header_start..payload_start]).map_err(|error| {
            TransportError::decode(format!(
                "invalid skiff binary frame: header is not valid JSON: {error}"
            ))
        })?;
    if !header.is_object() {
        return Err(TransportError::decode(
            "invalid skiff binary frame: header must be an object",
        ));
    }

    Ok(BinaryFrame {
        header,
        payload_bytes: frame[payload_start..].to_vec(),
    })
}

pub fn decode_typed_binary_frame<THeader: DeserializeOwned>(
    frame: &[u8],
) -> std::result::Result<(THeader, Vec<u8>), BinaryFrameError> {
    let frame = decode_binary_frame(frame)?;
    let header = serde_json::from_value(frame.header).map_err(|error| {
        TransportError::decode(format!(
            "invalid skiff binary frame: header failed typed decode: {error}"
        ))
    })?;
    Ok((header, frame.payload_bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeTraceContextFrameHeader {
    pub trace_id: String,
    pub span_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeDeadlineFrameHeader {
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCallerFrameHeader {
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRegisterFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub build_id: String,
    pub revision_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    pub service_protocol_identity: String,
    pub targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_revision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gateway_entry_identities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<RuntimeCapabilitiesFrameHeaderMetadata>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilitiesFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
    pub capabilities: RuntimeCapabilitiesFrameHeaderMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilitiesFrameHeaderMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispatch_modes: Vec<RuntimeDispatchModeCapability>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub package_test_dispatch: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub request_cancel: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub runtime_program: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeDispatchModeCapability {
    #[serde(rename = "unary")]
    Unary,
    #[serde(rename = "serverStream")]
    ServerStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRegisteredFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub runtime_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterControlFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub artifact_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_reload: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_config: Vec<RouterControlServiceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryControlConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_backend: Option<FileBackendControlConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHttpNameValueFrameHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHttpRequestFrameHeader {
    pub method: String,
    pub url: String,
    pub path: String,
    pub query: Vec<RuntimeHttpNameValueFrameHeader>,
    pub headers: Vec<RuntimeHttpNameValueFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHttpResponseFrameHeader {
    pub status: u16,
    pub headers: Vec<RuntimeHttpNameValueFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeHttpAdapterFrameHeader {
    pub kind: RuntimeHttpAdapterKindFrameHeader,
    pub handler: RuntimeHttpAdapterCallableFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<RuntimeHttpAdapterCallableFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<RuntimeHttpAdapterCallableFrameHeader>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeGatewayAdapterArgFrameHeader>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeHttpAdapterKindFrameHeader {
    TypedJson,
    RawHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeHttpAdapterCallableFrameHeader {
    ServiceFunction {
        module_path: String,
        symbol: String,
    },
    PackageFunction {
        package_id: String,
        symbol_path: String,
    },
}

pub type RuntimeHttpAdapterArgFrameHeader = RuntimeGatewayAdapterArgFrameHeader;
pub type RuntimeHttpAdapterSourceFrameHeader = RuntimeGatewayAdapterSourceFrameHeader;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeGatewayAdapterArgFrameHeader {
    pub param: String,
    pub source: RuntimeGatewayAdapterSourceFrameHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RuntimeGatewayAdapterSourceFrameHeader {
    #[serde(rename = "http.request")]
    HttpRequest,
    #[serde(rename = "http.body")]
    HttpBody,
    #[serde(rename = "http.context")]
    HttpContext,
    #[serde(rename = "websocket.connectRequest")]
    WebSocketConnectRequest,
    #[serde(rename = "websocket.receiveEvent")]
    WebSocketReceiveEvent,
    #[serde(rename = "websocket.connection")]
    WebSocketConnection,
    #[serde(rename = "websocket.connectionContext")]
    WebSocketConnectionContext,
    #[serde(rename = "websocket.message")]
    WebSocketMessage,
    #[serde(rename = "websocket.messageBody")]
    WebSocketMessageBody,
    #[serde(rename = "websocket.connectionId")]
    WebSocketConnectionId,
    #[serde(rename = "websocket.businessIdentity")]
    WebSocketBusinessIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeWebSocketAdapterKindFrameHeader {
    Connect,
    Receive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketAdapterFrameHeader {
    pub kind: RuntimeWebSocketAdapterKindFrameHeader,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<RuntimeGatewayAdapterArgFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_expectation: Option<RuntimeWebSocketContextExpectationFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_request: Option<RuntimeWebSocketConnectRequestFrameHeader>,
    #[serde(rename = "receiveEvent")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_request: Option<RuntimeWebSocketReceiveRequestFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RuntimeWebSocketContextExpectationFrameHeader {
    Null,
    Typed {
        connect_operation_abi_id: String,
        context_type_identity: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketContextCodecFrameHeader {
    pub operation_abi_id: String,
    pub context_type_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketConnectRequestFrameHeader {
    pub connection_id: String,
    pub url: String,
    pub query: Vec<RuntimeHttpNameValueFrameHeader>,
    pub headers: Vec<RuntimeHttpNameValueFrameHeader>,
    pub cookies: Vec<RuntimeHttpNameValueFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketReceiveRequestFrameHeader {
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    pub message: RuntimeWebSocketMessageFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_codec: Option<RuntimeWebSocketContextCodecFrameHeader>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub payload_segments: Vec<RuntimeWebSocketPayloadSegmentFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketMessageFrameHeader {
    pub tag: RuntimeWebSocketMessageTagFrameHeader,
    pub encoding: RuntimeWebSocketMessageEncodingFrameHeader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeWebSocketMessageTagFrameHeader {
    Text,
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeWebSocketMessageEncodingFrameHeader {
    Utf8,
    #[serde(rename = "binary")]
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeWebSocketPayloadSegmentKindFrameHeader {
    #[serde(rename = "websocket.context")]
    Context,
    #[serde(rename = "websocket.message")]
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketPayloadSegmentFrameHeader {
    pub kind: RuntimeWebSocketPayloadSegmentKindFrameHeader,
    pub offset: usize,
    pub length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeWebSocketResponseFrameHeader {
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_policy: Option<WebSocketConnectionPolicyControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_codec: Option<RuntimeWebSocketContextCodecFrameHeader>,
    // The router envelope schema marks `contextPayloadPresent` as required, so it
    // must always be serialized (including `false` for the reject path).
    pub context_payload_present: bool,
    #[serde(rename = "code")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<u16>,
    #[serde(rename = "reason")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestTestEffectDouble {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_request: Option<Value>,
    pub response: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestStartFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub mode: String,
    pub caller: RuntimeCallerFrameHeader,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_abi_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    // Stable published addressing coordinate. The router resolves the current
    // build for (service_id, version) at request time. Optional for gateway- and
    // HTTP-entry-originated requests that route by build/entry identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub build_id: String,
    pub service_protocol_identity: String,
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
    #[serde(
        default,
        deserialize_with = "deserialize_test_effect_doubles",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub test_effect_doubles: HashMap<String, Vec<RequestTestEffectDouble>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageTestStartFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub caller: RuntimeCallerFrameHeader,
    pub package_id: String,
    pub package_version: String,
    pub test_build_identity: String,
    pub entrypoint_id: String,
    pub activation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<RuntimeDeadlineFrameHeader>,
    pub trace: RuntimeTraceContextFrameHeader,
    #[serde(default, skip_serializing_if = "is_false")]
    pub test_effects_enabled: bool,
    #[serde(
        default,
        deserialize_with = "deserialize_test_effect_doubles",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub test_effect_doubles: HashMap<String, Vec<RequestTestEffectDouble>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseChunkFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseStartFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub http_response: RuntimeHttpResponseFrameHeader,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseEndFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub payload_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_response: Option<RuntimeHttpResponseFrameHeader>,
    #[serde(rename = "websocketConnect")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_connect: Option<RuntimeWebSocketResponseFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeErrorFramePayload {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResponseErrorFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub error: RuntimeErrorFramePayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestCancelFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionSendFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub service_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket_entry_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorKeyFrameMetadata {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRefFrameMetadata {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorPutRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyFrameMetadata,
    pub object_schema_identity: String,
    pub object_encoding_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorPutResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub actor_ref: ActorRefFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFindRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorFindResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub found: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_ref: Option<ActorRefFrameMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRemoveRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub actor_key: ActorKeyFrameMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorRemoveResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub removed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnSubmitRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub target_kind: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller_target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queue_wait_ms: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnSubmitResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub spawn_id: String,
    pub item_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnClaimRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub worker_id: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    pub supported_targets: Vec<String>,
    pub supported_spawn_compatibility_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_execution_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnClaimDescriptorFrameMetadata {
    pub item_id: String,
    pub lease_id: String,
    pub spawn_execution_id: String,
    pub runtime_request_id: String,
    pub spawn_id: String,
    pub target_kind: String,
    pub target: String,
    pub service_id: String,
    pub service_version: String,
    pub service_protocol_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_schema_identity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnClaimResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub claimed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<SpawnClaimDescriptorFrameMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRenewRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub item_id: String,
    pub lease_id: String,
    pub worker_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnRenewResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub item_id: String,
    pub renewed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnCompleteRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub item_id: String,
    pub lease_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnCompleteResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub item_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnFailRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub runtime_id: String,
    pub item_id: String,
    pub lease_id: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SpawnFailResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub item_id: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorSpawnRuntimeErrorFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub rpc_id: String,
    pub error: RuntimeErrorFramePayload,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn deserialize_test_effect_doubles<'de, D>(
    deserializer: D,
) -> std::result::Result<HashMap<String, Vec<RequestTestEffectDouble>>, D::Error>
where
    D: Deserializer<'de>,
{
    let doubles = HashMap::<String, Vec<RequestTestEffectDouble>>::deserialize(deserializer)?;
    for (target, sequence) in &doubles {
        if sequence.is_empty() {
            return Err(de::Error::custom(format!(
                "testEffectDoubles.{target} must be a non-empty array"
            )));
        }
    }
    Ok(doubles)
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum TelemetryProtocol {
    #[serde(rename = "skiff-telemetry-v1")]
    SkiffTelemetryV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryControlConfig {
    pub endpoint: String,
    pub protocol: TelemetryProtocol,
    pub topics: Vec<TelemetryTopic>,
    pub queue_max_events: u32,
    pub batch_max_events: u32,
    pub batch_max_bytes: u32,
    pub flush_interval_ms: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryTopic {
    #[serde(rename = "log")]
    Log,
    #[serde(rename = "trace")]
    Trace,
    #[serde(rename = "metric")]
    Metric,
    #[serde(rename = "health")]
    Health,
    #[serde(rename = "debug")]
    Debug,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetrySource {
    #[serde(rename = "gateway")]
    Gateway,
    #[serde(rename = "router")]
    Router,
    #[serde(rename = "runtime")]
    Runtime,
    #[serde(rename = "provider")]
    Provider,
    #[serde(rename = "test")]
    Test,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryLevel {
    #[serde(rename = "debug")]
    Debug,
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendControlConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<FileBackendLocalConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oss: Option<FileBackendOssConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendLocalConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendOssConfig {
    pub endpoint: String,
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_secret_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRegisterEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub protocol: TelemetryProtocol,
    pub producer_id: String,
    pub source: TelemetrySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    pub topics: Vec<TelemetryTopic>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryEvent {
    pub topic: TelemetryTopic,
    pub ts: String,
    pub source: TelemetrySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_capability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<TelemetryLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryBatchEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub producer_id: String,
    pub seq: u64,
    pub events: Vec<TelemetryEvent>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeRegisterEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: &'static str,
    pub runtime_id: String,
    pub service_id: String,
    // Stable published version this runtime serves. The router indexes
    // (service_id, version) -> current build_id from on-disk pointer records and
    // uses version as the cross-service addressing key; this field lets the live
    // registry confirm which version a registered build belongs to.
    pub version: String,
    pub build_id: String,
    // Compatibility fields required by the current TypeScript router.
    pub revision_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    pub service_protocol_identity: String,
    pub contract_identity: String,
    pub targets: Vec<String>,
    pub protocol_version: String,
    pub runtime_version: String,
    pub code_revision_id: String,
    pub implementation_identity: String,
    pub artifact_identity: String,
    pub capabilities: RuntimeCapabilitiesFrameHeaderMetadata,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gateway_entry_identities: Vec<String>,
}

impl From<RuntimeRegisterEnvelope> for RuntimeRegisterFrameHeader {
    fn from(envelope: RuntimeRegisterEnvelope) -> Self {
        Self {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: envelope.envelope_type.to_string(),
            runtime_id: envelope.runtime_id,
            service_id: envelope.service_id,
            version: Some(envelope.version),
            build_id: envelope.build_id,
            revision_id: envelope.revision_id,
            activation_identity: envelope.activation_identity,
            service_protocol_identity: envelope.service_protocol_identity,
            targets: envelope.targets,
            protocol_version: Some(envelope.protocol_version),
            runtime_version: Some(envelope.runtime_version),
            code_revision_id: Some(envelope.code_revision_id),
            artifact_identity: Some(envelope.artifact_identity),
            gateway_entry_identities: envelope.gateway_entry_identities,
            capabilities: Some(envelope.capabilities),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlEnvelope {
    pub artifact_roots: Vec<PathBuf>,
    #[serde(default)]
    pub dev_reload: Option<bool>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub service_config: Vec<RouterControlServiceConfig>,
    #[serde(default)]
    pub telemetry: Option<TelemetryControlConfig>,
    #[serde(default)]
    pub file_backend: Option<FileBackendControlConfig>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl RouterControlEnvelope {
    pub fn ordered_artifact_roots(&self) -> std::result::Result<Vec<PathBuf>, String> {
        ordered_artifact_roots(&self.artifact_roots)
    }

    pub fn reject_legacy_config_fields(&self) -> std::result::Result<(), String> {
        self.ordered_artifact_roots()?;
        if self.extra.contains_key("serviceValues") {
            return Err(
                "router.control serviceValues is no longer supported; use serviceConfig"
                    .to_string(),
            );
        }
        if self.extra.contains_key("serviceEnv") {
            return Err(
                "router.control serviceEnv is no longer supported; use serviceConfig".to_string(),
            );
        }
        if let Some(file_backend) = &self.file_backend {
            file_backend.validate()?;
        }
        for (index, service_config) in self.service_config.iter().enumerate() {
            service_config.reject_legacy_config_fields(index)?;
        }
        Ok(())
    }
}

impl FileBackendControlConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.local.is_none() && self.oss.is_none() {
            return Err("router.control fileBackend must configure local or oss".to_string());
        }
        if let Some(local) = &self.local {
            local.validate()?;
        }
        if let Some(oss) = &self.oss {
            oss.validate()?;
        }
        Ok(())
    }
}

impl FileBackendLocalConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if self.root.as_os_str().is_empty() {
            return Err("router.control fileBackend.local.root must be non-empty".to_string());
        }
        Ok(())
    }
}

impl FileBackendOssConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("router.control fileBackend.oss.endpoint must be non-empty".to_string());
        }
        if self.bucket.trim().is_empty() {
            return Err("router.control fileBackend.oss.bucket must be non-empty".to_string());
        }
        if missing_optional_string(self.access_key_id.as_deref())
            && missing_optional_string(self.access_key_id_env.as_deref())
        {
            return Err(
                "router.control fileBackend.oss requires accessKeyIdEnv or accessKeyId".to_string(),
            );
        }
        if missing_optional_string(self.access_key_secret.as_deref())
            && missing_optional_string(self.access_key_secret_env.as_deref())
        {
            return Err(
                "router.control fileBackend.oss requires accessKeySecretEnv or accessKeySecret"
                    .to_string(),
            );
        }
        Ok(())
    }
}

fn missing_optional_string(value: Option<&str>) -> bool {
    match value {
        Some(value) => value.is_empty(),
        None => true,
    }
}

fn deserialize_optional_config_shape<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<ConfigShape>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalConfigShapeVisitor;

    impl<'de> de::Visitor<'de> for OptionalConfigShapeVisitor {
        type Value = Option<ConfigShape>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a config shape object")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Err(E::custom("configShape must be an object when present"))
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            let shape = ConfigShape::deserialize(deserializer)?;
            shape.validate_schema_version().map_err(de::Error::custom)?;
            Ok(Some(shape))
        }
    }

    deserializer.deserialize_option(OptionalConfigShapeVisitor)
}

fn ordered_artifact_roots(artifact_roots: &[PathBuf]) -> std::result::Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    for root in artifact_roots {
        push_artifact_root(&mut roots, root)?;
    }
    if roots.is_empty() {
        return Err("router.control artifactRoots must include at least one root".to_string());
    }
    Ok(roots)
}

fn push_artifact_root(roots: &mut Vec<PathBuf>, root: &PathBuf) -> std::result::Result<(), String> {
    if root.as_os_str().is_empty() {
        return Err("router.control artifact roots must not be empty".to_string());
    }
    if roots.iter().any(|existing| existing == root) {
        return Err(format!(
            "router.control artifactRoots contains duplicate root {}",
            root.display()
        ));
    }
    roots.push(root.clone());
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlServiceConfig {
    pub service_id: String,
    pub build_id: String,
    pub activation_identity: String,
    pub resolved_config_identity: String,
    #[serde(default)]
    pub resolved_config: Value,
    #[serde(default)]
    pub redacted_resolved_config: Value,
    #[serde(default)]
    pub redaction_projection_identity: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_config_shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub config_shape: Option<ConfigShape>,
    #[serde(default)]
    pub service_db: Option<RouterControlServiceDb>,
    #[serde(default)]
    pub package_configs: Vec<RouterControlPackageConfig>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlServiceDb {
    pub mongo_url: String,
    pub storage_service_id: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlPackageConfig {
    pub package_id: String,
    #[serde(default)]
    pub package_slot: Option<usize>,
    pub alias: String,
    pub resolved_config_identity: String,
    #[serde(default)]
    pub resolved_config: Value,
    #[serde(default)]
    pub redacted_resolved_config: Value,
    #[serde(default)]
    pub redaction_projection_identity: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_config_shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub config_shape: Option<ConfigShape>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl RouterControlServiceConfig {
    fn reject_legacy_config_fields(&self, index: usize) -> std::result::Result<(), String> {
        for field in [
            "valuesSnapshotIdentity",
            "valuesSnapshot",
            "redactedValuesSnapshot",
            "valuesPolicy",
            "resolvedEnvIdentity",
            "resolvedEnv",
            "redactedResolvedEnv",
            "envShape",
            "dependencyRef",
        ] {
            if self.extra.contains_key(field) {
                return Err(format!(
                    "router.control serviceConfig[{index}].{field} is no longer supported"
                ));
            }
        }
        for (package_index, package_config) in self.package_configs.iter().enumerate() {
            package_config.reject_legacy_config_fields(index, package_index)?;
        }
        if let Some(service_db) = &self.service_db {
            service_db.reject_legacy_config_fields(index)?;
        }
        Ok(())
    }
}

impl RouterControlServiceDb {
    fn reject_legacy_config_fields(&self, index: usize) -> std::result::Result<(), String> {
        if self.extra.contains_key("storageNamespace") {
            return Err(format!(
                "router.control serviceConfig[{index}].serviceDb.storageNamespace is no longer supported"
            ));
        }
        Ok(())
    }
}

impl RouterControlPackageConfig {
    fn reject_legacy_config_fields(
        &self,
        service_index: usize,
        package_index: usize,
    ) -> std::result::Result<(), String> {
        for field in [
            "valuesSnapshotIdentity",
            "valuesSnapshot",
            "redactedValuesSnapshot",
            "valuesPolicy",
            "resolvedEnvIdentity",
            "resolvedEnv",
            "redactedResolvedEnv",
            "envShape",
        ] {
            if self.extra.contains_key(field) {
                return Err(format!(
                    "router.control serviceConfig[{service_index}].packageConfigs[{package_index}].{field} is no longer supported"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSendEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: &'static str,
    pub service_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    pub payload_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct TypedEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

#[cfg(test)]
mod tests;

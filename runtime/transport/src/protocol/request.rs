use std::collections::HashMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_runtime_request_contract::{OpaqueServiceError, RuntimeClientSessionControl};

use crate::cancel_reason::RequestCancelReason;
use crate::{
    protocol::frame::{
        decode_binary_frame, decode_typed_binary_frame, encode_binary_frame,
        RESPONSE_ERROR_FRAME_SCHEMA_VERSION,
    },
    protocol::{FrameDirection, RUNTIME_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

pub const REQUEST_START_FRAME_TYPE: &str = "request.start";
pub const REQUEST_CANCEL_FRAME_TYPE: &str = "request.cancel";
pub const RESPONSE_START_FRAME_TYPE: &str = "response.start";
pub const RESPONSE_CHUNK_FRAME_TYPE: &str = "response.chunk";
pub const RESPONSE_END_FRAME_TYPE: &str = "response.end";
pub const RESPONSE_ERROR_FRAME_TYPE: &str = "response.error";

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
    pub client_session: Option<RuntimeClientSessionControl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<RuntimeDeadlineFrameHeader>,
    pub trace: RuntimeTraceContextFrameHeader,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_request: Option<RuntimeHttpRequestFrameHeader>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_adapter: Option<RuntimeHttpAdapterFrameHeader>,
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
#[serde(
    try_from = "RawResponseEndFrameHeader",
    into = "RawResponseEndFrameHeader"
)]
pub struct ResponseEndFrameHeader {
    pub schema_version: String,
    pub envelope_type: String,
    pub request_id: String,
    pub payload_present: bool,
    pub metadata: ResponseEndFrameMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ResponseEndFrameMetadata {
    None,
    Http(RuntimeHttpResponseFrameHeader),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawResponseEndFrameHeader {
    schema_version: String,
    #[serde(rename = "type")]
    envelope_type: String,
    request_id: String,
    payload_present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    http_response: Option<RuntimeHttpResponseFrameHeader>,
}

impl TryFrom<RawResponseEndFrameHeader> for ResponseEndFrameHeader {
    type Error = String;

    fn try_from(raw: RawResponseEndFrameHeader) -> Result<Self, Self::Error> {
        let metadata = match raw.http_response {
            None => ResponseEndFrameMetadata::None,
            Some(http) => ResponseEndFrameMetadata::Http(http),
        };
        Ok(Self {
            schema_version: raw.schema_version,
            envelope_type: raw.envelope_type,
            request_id: raw.request_id,
            payload_present: raw.payload_present,
            metadata,
        })
    }
}

impl From<ResponseEndFrameHeader> for RawResponseEndFrameHeader {
    fn from(header: ResponseEndFrameHeader) -> Self {
        let http_response = match header.metadata {
            ResponseEndFrameMetadata::None => None,
            ResponseEndFrameMetadata::Http(http) => Some(http),
        };
        Self {
            schema_version: header.schema_version,
            envelope_type: header.envelope_type,
            request_id: header.request_id,
            payload_present: header.payload_present,
            http_response,
        }
    }
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
#[serde(
    tag = "errorKind",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ResponseErrorFrameHeader {
    #[serde(rename = "fixedService")]
    FixedService {
        schema_version: String,
        #[serde(rename = "type")]
        envelope_type: String,
        request_id: String,
    },
    #[serde(rename = "control")]
    Control {
        schema_version: String,
        #[serde(rename = "type")]
        envelope_type: String,
        request_id: String,
        error: RuntimeErrorFramePayload,
    },
}

impl ResponseErrorFrameHeader {
    pub fn fixed_service(request_id: String) -> Self {
        Self::FixedService {
            schema_version: RESPONSE_ERROR_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.error".to_string(),
            request_id,
        }
    }

    pub fn control(request_id: String, error: RuntimeErrorFramePayload) -> Self {
        Self::Control {
            schema_version: RESPONSE_ERROR_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.error".to_string(),
            request_id,
            error,
        }
    }

    pub fn request_id(&self) -> &str {
        match self {
            Self::FixedService { request_id, .. } | Self::Control { request_id, .. } => request_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ValidatedResponseErrorFrame {
    FixedService(OpaqueServiceError),
    Control(RuntimeErrorFramePayload),
}

pub fn validate_response_error_frame(
    header: &ResponseErrorFrameHeader,
    payload_bytes: Vec<u8>,
) -> std::result::Result<ValidatedResponseErrorFrame, BinaryFrameError> {
    let (schema_version, envelope_type, request_id) = match header {
        ResponseErrorFrameHeader::FixedService {
            schema_version,
            envelope_type,
            request_id,
        }
        | ResponseErrorFrameHeader::Control {
            schema_version,
            envelope_type,
            request_id,
            ..
        } => (schema_version, envelope_type, request_id),
    };
    if schema_version != RESPONSE_ERROR_FRAME_SCHEMA_VERSION {
        return Err(TransportError::decode(format!(
            "invalid response.error frame: schemaVersion must be {RESPONSE_ERROR_FRAME_SCHEMA_VERSION}"
        )));
    }
    if envelope_type != "response.error" {
        return Err(TransportError::decode(
            "invalid response.error frame: type must be response.error",
        ));
    }
    if request_id.trim().is_empty() {
        return Err(TransportError::decode(
            "invalid response.error frame: requestId must be non-empty",
        ));
    }

    match header {
        ResponseErrorFrameHeader::FixedService { .. } => {
            if payload_bytes.is_empty() {
                return Err(TransportError::decode(
                    "invalid response.error fixedService frame: payload must be non-empty",
                ));
            }
            let error = OpaqueServiceError::decode(payload_bytes).map_err(|error| {
                TransportError::decode(format!(
                    "invalid response.error fixedService frame: payload failed strict service error decode: {error}"
                ))
            })?;
            Ok(ValidatedResponseErrorFrame::FixedService(error))
        }
        ResponseErrorFrameHeader::Control { error, .. } => {
            if !payload_bytes.is_empty() {
                return Err(TransportError::decode(
                    "invalid response.error control frame: payload must be empty",
                ));
            }
            if error.code.trim().is_empty() {
                return Err(TransportError::decode(
                    "invalid response.error control frame: error.code must be non-empty",
                ));
            }
            if error.message.trim().is_empty() {
                return Err(TransportError::decode(
                    "invalid response.error control frame: error.message must be non-empty",
                ));
            }
            if error
                .status
                .is_some_and(|status| !(400..=599).contains(&status))
            {
                return Err(TransportError::decode(
                    "invalid response.error control frame: error.status must be between 400 and 599",
                ));
            }
            Ok(ValidatedResponseErrorFrame::Control(error.clone()))
        }
    }
}

pub fn decode_response_error_frame(
    frame: &[u8],
) -> std::result::Result<(ResponseErrorFrameHeader, ValidatedResponseErrorFrame), BinaryFrameError>
{
    let decoded = decode_binary_frame(frame)?;
    let header =
        serde_json::from_value::<ResponseErrorFrameHeader>(decoded.header).map_err(|error| {
            TransportError::decode(format!(
                "invalid response.error frame: header failed strict decode: {error}"
            ))
        })?;
    let body = validate_response_error_frame(&header, decoded.payload_bytes)?;
    Ok((header, body))
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

/// Frame-level direction/payload-presence surface for the Request family
/// (C-model-request §2). W-dispatch uses this when wiring the closed demux
/// and sink bundle (plan §5.5); the codec-level payload rules are enforced by
/// the frame-specific `decode_*` / `encode_*` functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestFrameKind {
    Start,
    Cancel,
    ResponseStart,
    ResponseChunk,
    ResponseEnd,
    ResponseError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestFramePayloadPresence {
    /// Optional opaque bytes; `response.end` additionally requires
    /// `payloadPresent == !payload.is_empty()` (codec enforced).
    Optional,
    /// Payload must be empty (codec enforced).
    Empty,
    /// Variant-decided: `response.error` fixedService = required, control =
    /// empty (enforced by `validate_response_error_frame`).
    Variant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestFrameRule {
    pub kind: RequestFrameKind,
    pub direction: FrameDirection,
    pub payload_presence: RequestFramePayloadPresence,
}

/// Frozen frame-level rules for the Request family (C-model-request §2).
pub fn request_frame_rule(frame_type: &str) -> Option<RequestFrameRule> {
    match frame_type {
        REQUEST_START_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::Start,
            direction: FrameDirection::RouterToRuntime,
            payload_presence: RequestFramePayloadPresence::Optional,
        }),
        REQUEST_CANCEL_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::Cancel,
            direction: FrameDirection::Either,
            payload_presence: RequestFramePayloadPresence::Empty,
        }),
        RESPONSE_START_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::ResponseStart,
            direction: FrameDirection::RuntimeToRouter,
            payload_presence: RequestFramePayloadPresence::Empty,
        }),
        RESPONSE_CHUNK_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::ResponseChunk,
            direction: FrameDirection::RuntimeToRouter,
            payload_presence: RequestFramePayloadPresence::Optional,
        }),
        RESPONSE_END_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::ResponseEnd,
            direction: FrameDirection::RuntimeToRouter,
            payload_presence: RequestFramePayloadPresence::Optional,
        }),
        RESPONSE_ERROR_FRAME_TYPE => Some(RequestFrameRule {
            kind: RequestFrameKind::ResponseError,
            direction: FrameDirection::RuntimeToRouter,
            payload_presence: RequestFramePayloadPresence::Variant,
        }),
        _ => None,
    }
}

pub fn decode_request_cancel_frame(
    frame: &[u8],
) -> Result<RequestCancelFrameHeader, BinaryFrameError> {
    let (header, payload): (RequestCancelFrameHeader, Vec<u8>) = decode_typed_binary_frame(frame)?;
    validate_request_cancel_frame(&header, &payload)?;
    Ok(header)
}

pub fn encode_request_cancel_frame(
    header: &RequestCancelFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_request_cancel_frame(header, &[])?;
    encode_binary_frame(header, &[])
}

pub fn validate_request_cancel_frame(
    header: &RequestCancelFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_request_frame_identity(
        REQUEST_CANCEL_FRAME_TYPE,
        &header.schema_version,
        &header.envelope_type,
        &header.request_id,
    )?;
    if !payload.is_empty() {
        return Err(TransportError::decode(
            "request.cancel frame payload must be empty",
        ));
    }
    if RequestCancelReason::from_contract_h_wire(&header.reason).is_none() {
        return Err(TransportError::decode(
            "request.cancel reason must be one of the CONTRACT_H wire reasons",
        ));
    }
    Ok(())
}

pub fn decode_response_start_frame(
    frame: &[u8],
) -> Result<ResponseStartFrameHeader, BinaryFrameError> {
    let (header, payload): (ResponseStartFrameHeader, Vec<u8>) = decode_typed_binary_frame(frame)?;
    validate_response_start_frame(&header, &payload)?;
    Ok(header)
}

pub fn encode_response_start_frame(
    header: &ResponseStartFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_response_start_frame(header, &[])?;
    encode_binary_frame(header, &[])
}

pub fn validate_response_start_frame(
    header: &ResponseStartFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_request_frame_identity(
        RESPONSE_START_FRAME_TYPE,
        &header.schema_version,
        &header.envelope_type,
        &header.request_id,
    )?;
    if !payload.is_empty() {
        return Err(TransportError::decode(
            "response.start frame payload must be empty",
        ));
    }
    Ok(())
}

pub fn decode_response_chunk_frame(
    frame: &[u8],
) -> Result<(ResponseChunkFrameHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload): (ResponseChunkFrameHeader, Vec<u8>) = decode_typed_binary_frame(frame)?;
    validate_response_chunk_frame(&header)?;
    Ok((header, payload))
}

pub fn encode_response_chunk_frame(
    header: &ResponseChunkFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_response_chunk_frame(header)?;
    encode_binary_frame(header, payload)
}

pub fn validate_response_chunk_frame(
    header: &ResponseChunkFrameHeader,
) -> Result<(), BinaryFrameError> {
    validate_request_frame_identity(
        RESPONSE_CHUNK_FRAME_TYPE,
        &header.schema_version,
        &header.envelope_type,
        &header.request_id,
    )
}

pub fn decode_response_end_frame(
    frame: &[u8],
) -> Result<(ResponseEndFrameHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) = decode_typed_binary_frame(frame)?;
    validate_response_end_frame_wire(&header, &payload)?;
    Ok((header, payload))
}

pub fn encode_response_end_frame(
    header: &ResponseEndFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_response_end_frame_wire(header, payload)?;
    encode_binary_frame(header, payload)
}

/// Wire-level `response.end` phase consistency (C-model-request §5.2):
/// `payloadPresent` must equal `!payload.is_empty()`. This covers the unary
/// Payload/Http phases and the stream empty terminal; mode-specific stream
/// ordering stays with the dispatcher state machine (C-model-request §5.4).
pub fn validate_response_end_frame_wire(
    header: &ResponseEndFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_request_frame_identity(
        RESPONSE_END_FRAME_TYPE,
        &header.schema_version,
        &header.envelope_type,
        &header.request_id,
    )?;
    if header.payload_present == payload.is_empty() {
        return Err(TransportError::decode(
            "response.end payloadPresent must match payload presence",
        ));
    }
    Ok(())
}

fn validate_request_frame_identity(
    frame_type: &str,
    schema_version: &str,
    envelope_type: &str,
    request_id: &str,
) -> Result<(), BinaryFrameError> {
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(TransportError::decode(format!(
            "{frame_type} schemaVersion must be {RUNTIME_FRAME_SCHEMA_VERSION}"
        )));
    }
    if envelope_type != frame_type {
        return Err(TransportError::decode(format!(
            "{frame_type} type must be {frame_type}"
        )));
    }
    if request_id.trim().is_empty() {
        return Err(TransportError::decode(format!(
            "{frame_type} requestId must be non-empty"
        )));
    }
    Ok(())
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
pub(crate) fn is_false(value: &bool) -> bool {
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

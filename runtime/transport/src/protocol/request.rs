use std::collections::HashMap;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_runtime_request_contract::{OpaqueServiceError, RuntimeClientSessionControl};

use crate::{
    protocol::frame::{decode_binary_frame, RESPONSE_ERROR_FRAME_SCHEMA_VERSION},
    BinaryFrameError, TransportError,
};

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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use skiff_artifact_model::{WebSocketEntryId, WEBSOCKET_JSON_RPC_TEXT_PROFILE};

use crate::{
    cancel_reason::RequestCancelReason,
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, RuntimeDeadlineFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
    BinaryFrameError, TransportError,
};

pub const CONNECTION_REQUEST_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;
pub const CONNECTION_REQUEST_MAX_METHOD_BYTES: usize = 256;
pub const CONNECTION_RESPONSE_MAX_MESSAGE_BYTES: usize = 4096;
const MAX_IDENTIFIER_BYTES: usize = 1024;
const JAVASCRIPT_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketRpcProfile {
    #[serde(rename = "jsonrpc-2.0-text")]
    JsonRpc2_0Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionRequestFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub service_id: String,
    pub websocket_entry_id: WebSocketEntryId,
    pub connection_id: String,
    pub profile: WebSocketRpcProfile,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<RuntimeDeadlineFrameHeader>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionRequestCancelFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub reason: RequestCancelReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionResponseOutcome {
    Success,
    DeadlineExceeded,
    ConnectionUnavailable,
    TransportUnavailable,
    ProtocolError,
    ResourceLimit,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionRemoteErrorFrameHeader {
    pub code: i64,
    pub message: String,
    pub data_present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConnectionResponseFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub request_id: String,
    pub outcome: ConnectionResponseOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<ConnectionRemoteErrorFrameHeader>,
}

pub fn encode_connection_request_frame(
    header: &ConnectionRequestFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_request(header, payload)?;
    encode_binary_frame(header, payload)
}

pub fn decode_connection_request_frame(
    frame: &[u8],
) -> Result<(ConnectionRequestFrameHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload) = decode_typed_binary_frame(frame)?;
    validate_request(&header, &payload)?;
    Ok((header, payload))
}

pub fn encode_connection_request_cancel_frame(
    header: &ConnectionRequestCancelFrameHeader,
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_cancel(header, &[])?;
    encode_binary_frame(header, &[])
}

pub fn decode_connection_request_cancel_frame(
    frame: &[u8],
) -> Result<ConnectionRequestCancelFrameHeader, BinaryFrameError> {
    let (header, payload) = decode_typed_binary_frame(frame)?;
    validate_cancel(&header, &payload)?;
    Ok(header)
}

pub fn encode_connection_response_frame(
    header: &ConnectionResponseFrameHeader,
    payload: &[u8],
) -> Result<Vec<u8>, BinaryFrameError> {
    validate_response(header, payload)?;
    encode_binary_frame(header, payload)
}

pub fn decode_connection_response_frame(
    frame: &[u8],
) -> Result<(ConnectionResponseFrameHeader, Vec<u8>), BinaryFrameError> {
    let (header, payload) = decode_typed_binary_frame(frame)?;
    validate_response(&header, &payload)?;
    Ok((header, payload))
}

fn validate_request(
    header: &ConnectionRequestFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_envelope(
        &header.schema_version,
        &header.envelope_type,
        "connection.request",
    )?;
    validate_token(&header.request_id, "requestId", MAX_IDENTIFIER_BYTES)?;
    validate_token(&header.service_id, "serviceId", MAX_IDENTIFIER_BYTES)?;
    validate_token(
        header.websocket_entry_id.as_str(),
        "websocketEntryId",
        MAX_IDENTIFIER_BYTES,
    )?;
    validate_token(&header.connection_id, "connectionId", MAX_IDENTIFIER_BYTES)?;
    if !matches!(header.profile, WebSocketRpcProfile::JsonRpc2_0Text)
        || WEBSOCKET_JSON_RPC_TEXT_PROFILE != "jsonrpc-2.0-text"
    {
        return Err(decode_error("connection request profile is unsupported"));
    }
    validate_token(
        &header.method,
        "method",
        CONNECTION_REQUEST_MAX_METHOD_BYTES,
    )?;
    if let Some(deadline) = &header.deadline {
        if deadline.timeout_ms == 0
            || deadline.timeout_ms > JAVASCRIPT_MAX_SAFE_INTEGER as u64
            || !is_rfc3339_utc_or_offset(&deadline.expires_at)
        {
            return Err(decode_error(
                "connection request deadline must have positive safe-integer timeoutMs and RFC3339 expiresAt",
            ));
        }
    }
    if payload.is_empty() || payload.len() > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES {
        return Err(decode_error(
            "connection request payload must be present and within the payload limit",
        ));
    }
    let value = decode_json_payload(payload, "connection request params")?;
    if !value.is_object() && !value.is_array() {
        return Err(decode_error(
            "connection request params payload must be a JSON object or array",
        ));
    }
    Ok(())
}

fn validate_cancel(
    header: &ConnectionRequestCancelFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_envelope(
        &header.schema_version,
        &header.envelope_type,
        "connection.request.cancel",
    )?;
    validate_token(&header.request_id, "requestId", MAX_IDENTIFIER_BYTES)?;
    if !payload.is_empty() {
        return Err(decode_error(
            "connection request cancel payload must be empty",
        ));
    }
    Ok(())
}

fn validate_response(
    header: &ConnectionResponseFrameHeader,
    payload: &[u8],
) -> Result<(), BinaryFrameError> {
    validate_envelope(
        &header.schema_version,
        &header.envelope_type,
        "connection.response",
    )?;
    validate_token(&header.request_id, "requestId", MAX_IDENTIFIER_BYTES)?;
    if payload.len() > CONNECTION_REQUEST_MAX_PAYLOAD_BYTES {
        return Err(decode_error(
            "connection response payload exceeds the payload limit",
        ));
    }

    match header.outcome {
        ConnectionResponseOutcome::Success => {
            if header.remote.is_some() || payload.is_empty() {
                return Err(decode_error(
                    "successful connection response requires payload and forbids remote metadata",
                ));
            }
            decode_json_payload(payload, "connection response result")?;
        }
        ConnectionResponseOutcome::Remote => {
            let remote = header.remote.as_ref().ok_or_else(|| {
                decode_error("remote connection response requires remote metadata")
            })?;
            if remote.code.unsigned_abs() > JAVASCRIPT_MAX_SAFE_INTEGER as u64 {
                return Err(decode_error(
                    "remote connection response code must be a safe integer",
                ));
            }
            if remote.message.trim().is_empty()
                || remote.message.as_bytes().len() > CONNECTION_RESPONSE_MAX_MESSAGE_BYTES
            {
                return Err(decode_error(
                    "remote connection response message must be a bounded non-empty string",
                ));
            }
            if remote.data_present != !payload.is_empty() {
                return Err(decode_error(
                    "remote dataPresent must match payload presence",
                ));
            }
            if remote.data_present {
                decode_json_payload(payload, "remote connection response data")?;
            }
        }
        ConnectionResponseOutcome::DeadlineExceeded
        | ConnectionResponseOutcome::ConnectionUnavailable
        | ConnectionResponseOutcome::TransportUnavailable
        | ConnectionResponseOutcome::ProtocolError
        | ConnectionResponseOutcome::ResourceLimit => {
            if header.remote.is_some() || !payload.is_empty() {
                return Err(decode_error(
                    "non-payload connection response forbids remote metadata and payload",
                ));
            }
        }
    }
    Ok(())
}

fn validate_envelope(
    schema_version: &str,
    envelope_type: &str,
    expected_type: &str,
) -> Result<(), BinaryFrameError> {
    if schema_version != RUNTIME_FRAME_SCHEMA_VERSION {
        return Err(decode_error(
            "connection frame schemaVersion is unsupported",
        ));
    }
    if envelope_type != expected_type {
        return Err(decode_error(format!(
            "connection frame type must be {expected_type}"
        )));
    }
    Ok(())
}

fn validate_token(value: &str, label: &str, max_bytes: usize) -> Result<(), BinaryFrameError> {
    if value.is_empty()
        || value.trim() != value
        || value.as_bytes().len() > max_bytes
        || value.chars().any(char::is_control)
    {
        return Err(decode_error(format!(
            "connection frame {label} must be a bounded non-empty canonical string"
        )));
    }
    Ok(())
}

fn decode_json_payload(payload: &[u8], label: &str) -> Result<Value, BinaryFrameError> {
    std::str::from_utf8(payload).map_err(|_| decode_error(format!("{label} must be UTF-8")))?;
    serde_json::from_slice(payload)
        .map_err(|error| decode_error(format!("{label} must be valid JSON: {error}")))
}

fn is_rfc3339_utc_or_offset(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
        || bytes.get(13) != Some(&b':')
        || bytes.get(16) != Some(&b':')
        || !bytes[0..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
        || !bytes[11..13].iter().all(u8::is_ascii_digit)
        || !bytes[14..16].iter().all(u8::is_ascii_digit)
        || !bytes[17..19].iter().all(u8::is_ascii_digit)
    {
        return false;
    }

    let year = parse_decimal(&bytes[0..4]);
    let month = parse_decimal(&bytes[5..7]);
    let day = parse_decimal(&bytes[8..10]);
    let hour = parse_decimal(&bytes[11..13]);
    let minute = parse_decimal(&bytes[14..16]);
    let second = parse_decimal(&bytes[17..19]);
    if month == 0
        || month > 12
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return false;
    }

    let mut cursor = 19;
    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let fraction_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == fraction_start {
            return false;
        }
    }

    if bytes.get(cursor) == Some(&b'Z') {
        return cursor + 1 == bytes.len();
    }
    if cursor + 6 != bytes.len()
        || !matches!(bytes[cursor], b'+' | b'-')
        || bytes[cursor + 3] != b':'
        || !bytes[cursor + 1..cursor + 3].iter().all(u8::is_ascii_digit)
        || !bytes[cursor + 4..cursor + 6].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    parse_decimal(&bytes[cursor + 1..cursor + 3]) <= 23
        && parse_decimal(&bytes[cursor + 4..cursor + 6]) <= 59
}

fn parse_decimal(bytes: &[u8]) -> u16 {
    bytes
        .iter()
        .fold(0, |value, digit| value * 10 + u16::from(digit - b'0'))
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn decode_error(message: impl Into<String>) -> BinaryFrameError {
    TransportError::decode(format!("invalid connection frame: {}", message.into()))
}

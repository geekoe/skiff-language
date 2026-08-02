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
pub const WEBSOCKET_JSONRPC_MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const WEBSOCKET_JSONRPC_MAX_JSON_DEPTH: usize = 64;
pub const WEBSOCKET_JSONRPC_MAX_JSON_NODES: usize = 100_000;
pub const WEBSOCKET_JSONRPC_MAX_STRING_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 1024;
const JAVASCRIPT_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WebSocketRpcProfile {
    #[serde(rename = "jsonrpc-2.0-text")]
    JsonRpc2_0Text,
}

/// Logical client socket generation identity (C-model-connection §2).
///
/// `connection_id` + `generation` uniquely identify one physical client
/// socket generation; business replacement creates a new generation and the
/// old generation's captured writer is no longer eligible to write the new
/// socket. This is intentionally a dedicated newtype and is never
/// interchangeable with a `RuntimeSessionEpoch`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClientSocketGeneration {
    pub connection_id: String,
    pub generation: u64,
}

impl ClientSocketGeneration {
    /// Strict constructor: `connectionId` follows the connection wire token
    /// rule (non-empty, no control characters, at most 1024 bytes).
    pub fn new(connection_id: impl Into<String>, generation: u64) -> Result<Self, String> {
        let connection_id = connection_id.into();
        validate_token(&connection_id, "connectionId", MAX_IDENTIFIER_BYTES)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection_id,
            generation,
        })
    }
}

/// Canonical peer identifier produced by the JSON-RPC 2.0 text profile
/// (C-model-connection §5.2). Numeric ids are validated lexeme-exactly as
/// JavaScript safe integers and canonicalized to canonical decimal before
/// use; `1e0` -> `1`, `-0` -> `0`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OpaquePeerId {
    String(String),
    SafeInteger(i128),
}

impl OpaquePeerId {
    /// Canonical peer key (`s:<string>` / `n:<integer>`) used for peer
    /// correlation (C-model-connection §5.2).
    pub fn canonical_key(&self) -> String {
        match self {
            Self::String(value) => format!("s:{value}"),
            Self::SafeInteger(value) => format!("n:{value}"),
        }
    }
}

/// Platform error classification for malformed peer frames
/// (C-model-connection §5.1.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonRpcPlatformErrorKind {
    Parse,
    InvalidRequest,
    InvalidParams,
}

/// Classified action for one peer text frame (C-model-connection §5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileAction {
    Request { id: OpaquePeerId, method: String },
    Notification { method: String },
    Response { id: String },
    PlatformError { kind: JsonRpcPlatformErrorKind },
    Close { code: u16 },
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

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 text profile lexical classification (C-model-connection §5)
// ---------------------------------------------------------------------------

/// Classifies one peer text frame under the frozen `jsonrpc-2.0-text` profile.
///
/// This is a total function over wire bytes:
/// - non-UTF-8 or frames above [`WEBSOCKET_JSONRPC_MAX_TEXT_BYTES`] produce
///   `Close { code: 1009 }` (§5.1.1);
/// - malformed JSON produces `PlatformError { kind: Parse }`;
/// - structurally invalid requests/notifications produce
///   `PlatformError` (`invalidRequest` / `invalidParams`);
/// - invalid responses produce `Close { code: 1002 }` (§5.1.6);
/// - valid frames produce `Request` / `Notification` / `Response` with the
///   canonical peer id (§5.2).
///
/// The classifier only performs lexical classification; it never parses
/// business params/result/error schemas.
pub fn classify_jsonrpc_20_text_frame(frame: &[u8]) -> ProfileAction {
    if frame.len() > WEBSOCKET_JSONRPC_MAX_TEXT_BYTES {
        return ProfileAction::Close { code: 1009 };
    }
    if std::str::from_utf8(frame).is_err() {
        return ProfileAction::Close { code: 1009 };
    }

    let mut cursor = 0;
    let mut budget = LexBudget::default();
    let node = match parse_lex_node(frame, &mut cursor, &mut budget) {
        Ok(node) if cursor == frame.len() => node,
        Ok(_) | Err(LexError::Parse) => {
            return ProfileAction::PlatformError {
                kind: JsonRpcPlatformErrorKind::Parse,
            }
        }
        Err(LexError::Budget) => return ProfileAction::Close { code: 1009 },
    };
    let LexNode::Object(members) = node else {
        return ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidRequest,
        };
    };

    let mut seen = std::collections::HashSet::new();
    for (key, _) in &members {
        if !seen.insert(key.clone()) {
            // Duplicate object members are an invalid request, not a parse
            // error (C-model-connection §5.1.2 corpus).
            return ProfileAction::PlatformError {
                kind: JsonRpcPlatformErrorKind::InvalidRequest,
            };
        }
    }

    let field_names = members
        .iter()
        .map(|(key, _)| key.as_str())
        .collect::<std::collections::HashSet<_>>();
    let response_candidate = field_names.contains("result")
        || field_names.contains("error")
        || (field_names.contains("id") && !field_names.contains("method"));
    if response_candidate {
        return classify_response(&members);
    }
    if field_names.contains("method")
        && !field_names.contains("result")
        && !field_names.contains("error")
    {
        return classify_request_or_notification(&members);
    }
    ProfileAction::PlatformError {
        kind: JsonRpcPlatformErrorKind::InvalidRequest,
    }
}

fn classify_request_or_notification(members: &[(String, LexNode)]) -> ProfileAction {
    let has_id = members.iter().any(|(key, _)| key == "id");
    let allowed: &[&str] = if has_id {
        &["jsonrpc", "id", "method", "params"]
    } else {
        &["jsonrpc", "method", "params"]
    };
    let member = |key: &str| {
        members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    if members
        .iter()
        .any(|(key, _)| !allowed.contains(&key.as_str()))
        || member("jsonrpc") != Some(&LexNode::String("2.0".to_string()))
    {
        return ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidRequest,
        };
    }
    let method = match member("method") {
        Some(LexNode::String(method))
            if !method.is_empty()
                && method.len() <= CONNECTION_REQUEST_MAX_METHOD_BYTES =>
        {
            method.clone()
        }
        _ => {
            return ProfileAction::PlatformError {
                kind: JsonRpcPlatformErrorKind::InvalidRequest,
            }
        }
    };
    if !has_id {
        return ProfileAction::Notification { method };
    }
    let id = match parse_peer_id(member("id")) {
        Some(id) => id,
        None => {
            return ProfileAction::PlatformError {
                kind: JsonRpcPlatformErrorKind::InvalidRequest,
            }
        }
    };
    match member("params") {
        Some(LexNode::Object(_)) | Some(LexNode::Array(_)) => ProfileAction::Request { id, method },
        Some(_) | None => ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidParams,
        },
    }
}

fn classify_response(members: &[(String, LexNode)]) -> ProfileAction {
    let member = |key: &str| {
        members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    let has_result = members.iter().any(|(key, _)| key == "result");
    let has_error = members.iter().any(|(key, _)| key == "error");
    if has_result == has_error || member("jsonrpc") != Some(&LexNode::String("2.0".to_string())) {
        return ProfileAction::Close { code: 1002 };
    }
    let id = match member("id") {
        Some(LexNode::String(value))
            if !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES =>
        {
            value.clone()
        }
        _ => return ProfileAction::Close { code: 1002 },
    };
    if has_result {
        if members
            .iter()
            .any(|(key, _)| !["jsonrpc", "id", "result"].contains(&key.as_str()))
        {
            return ProfileAction::Close { code: 1002 };
        }
        return ProfileAction::Response { id };
    }
    if members
        .iter()
        .any(|(key, _)| !["jsonrpc", "id", "error"].contains(&key.as_str()))
    {
        return ProfileAction::Close { code: 1002 };
    }
    let Some(LexNode::Object(error)) = member("error") else {
        return ProfileAction::Close { code: 1002 };
    };
    let error_member = |key: &str| {
        error
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    let fields_ok = error
        .iter()
        .all(|(key, _)| ["code", "message", "data"].contains(&key.as_str()));
    let code_ok = matches!(
        error_member("code"),
        Some(LexNode::Number(lexeme)) if parse_safe_integer(lexeme).is_some()
    );
    let message_ok = matches!(
        error_member("message"),
        Some(LexNode::String(message))
            if !message.is_empty() && message.len() <= WEBSOCKET_JSONRPC_MAX_STRING_BYTES
    );
    if fields_ok && code_ok && message_ok {
        ProfileAction::Response { id }
    } else {
        ProfileAction::Close { code: 1002 }
    }
}

fn parse_peer_id(node: Option<&LexNode>) -> Option<OpaquePeerId> {
    match node? {
        LexNode::String(value)
            if !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES =>
        {
            Some(OpaquePeerId::String(value.clone()))
        }
        LexNode::Number(lexeme) => Some(OpaquePeerId::SafeInteger(parse_safe_integer(lexeme)?)),
        _ => None,
    }
}

/// Validates a numeric lexeme as an exact JavaScript safe integer and returns
/// its canonical decimal value (`1e0` -> 1, `-0` -> 0, `1.000e2` -> 100).
/// Numbers that cannot be represented exactly (e.g. `1.5`, `9007199254740992`,
/// `1e-324`, `1.0000000000000000001`) are rejected (C-model-connection §5.2).
fn parse_safe_integer(lexeme: &str) -> Option<i128> {
    let (negative, coefficient, exponent, fraction_len) = split_number_lexeme(lexeme)?;
    if coefficient.chars().all(|ch| ch == '0') {
        return Some(0);
    }
    let exponent = exponent.parse::<i128>().ok()?;
    let scale = exponent - fraction_len as i128;
    let mut exact_digits = String::new();
    if scale >= 0 {
        let significant = coefficient.trim_start_matches('0');
        if significant.len() as i128 + scale > 16 {
            return None;
        }
        exact_digits.push_str(significant);
        for _ in 0..scale {
            exact_digits.push('0');
        }
    } else {
        let removed_count = (-scale) as usize;
        if removed_count > coefficient.len() {
            return None;
        }
        let removed = &coefficient[coefficient.len() - removed_count..];
        if removed.chars().any(|ch| ch != '0') {
            return None;
        }
        exact_digits.push_str(&coefficient[..coefficient.len() - removed_count]);
        let trimmed = exact_digits.trim_start_matches('0');
        if trimmed.is_empty() {
            return Some(0);
        }
        exact_digits = trimmed.to_string();
    }
    let mut exact = exact_digits.parse::<i128>().ok()?;
    if negative {
        exact = -exact;
    }
    if exact > JAVASCRIPT_MAX_SAFE_INTEGER as i128 || exact < -(JAVASCRIPT_MAX_SAFE_INTEGER as i128)
    {
        return None;
    }
    Some(exact)
}

fn split_number_lexeme(lexeme: &str) -> Option<(bool, String, String, usize)> {
    let bytes = lexeme.as_bytes();
    let mut index = 0;
    let negative = bytes.first() == Some(&b'-');
    if negative {
        index += 1;
    }
    let int_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == int_start {
        return None;
    }
    let integer = &lexeme[int_start..index];
    let mut fraction = "";
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == fraction_start {
            return None;
        }
        fraction = &lexeme[fraction_start..index];
    }
    let mut exponent = "0";
    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return None;
        }
        exponent = &lexeme[exponent_start..index];
    }
    if index != bytes.len() {
        return None;
    }
    Some((
        negative,
        format!("{integer}{fraction}"),
        exponent.to_string(),
        fraction.len(),
    ))
}

// ---------------------------------------------------------------------------
// Lexeme-preserving JSON subset parser (keeps number lexemes verbatim)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum LexNode {
    Object(Vec<(String, LexNode)>),
    Array(Vec<LexNode>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, Default)]
struct LexBudget {
    depth: usize,
    nodes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LexError {
    Parse,
    Budget,
}

fn parse_lex_node(
    source: &[u8],
    cursor: &mut usize,
    budget: &mut LexBudget,
) -> Result<LexNode, LexError> {
    skip_ws(source, cursor);
    match source.get(*cursor).copied() {
        Some(b'{') => {
            *cursor += 1;
            if budget.depth >= WEBSOCKET_JSONRPC_MAX_JSON_DEPTH {
                return Err(LexError::Budget);
            }
            if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
                return Err(LexError::Budget);
            }
            budget.depth += 1;
            budget.nodes += 1;
            let mut members = Vec::new();
            skip_ws(source, cursor);
            if source.get(*cursor) == Some(&b'}') {
                *cursor += 1;
                budget.depth -= 1;
                return Ok(LexNode::Object(members));
            }
            loop {
                let key = parse_lex_string(source, cursor, budget)?;
                skip_ws(source, cursor);
                if source.get(*cursor) != Some(&b':') {
                    return Err(LexError::Parse);
                }
                *cursor += 1;
                let value = parse_lex_node(source, cursor, budget)?;
                members.push((key, value));
                skip_ws(source, cursor);
                match source.get(*cursor) {
                    Some(b',') => {
                        *cursor += 1;
                    }
                    Some(b'}') => {
                        *cursor += 1;
                        budget.depth -= 1;
                        return Ok(LexNode::Object(members));
                    }
                    _ => return Err(LexError::Parse),
                }
            }
        }
        Some(b'[') => {
            *cursor += 1;
            if budget.depth >= WEBSOCKET_JSONRPC_MAX_JSON_DEPTH {
                return Err(LexError::Budget);
            }
            if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
                return Err(LexError::Budget);
            }
            budget.depth += 1;
            budget.nodes += 1;
            let mut items = Vec::new();
            skip_ws(source, cursor);
            if source.get(*cursor) == Some(&b']') {
                *cursor += 1;
                budget.depth -= 1;
                return Ok(LexNode::Array(items));
            }
            loop {
                items.push(parse_lex_node(source, cursor, budget)?);
                skip_ws(source, cursor);
                match source.get(*cursor) {
                    Some(b',') => {
                        *cursor += 1;
                    }
                    Some(b']') => {
                        *cursor += 1;
                        budget.depth -= 1;
                        return Ok(LexNode::Array(items));
                    }
                    _ => return Err(LexError::Parse),
                }
            }
        }
        Some(b'"') => parse_lex_string(source, cursor, budget).map(LexNode::String),
        Some(b't') => {
            if source.get(*cursor..*cursor + 4) == Some(b"true") {
                *cursor += 4;
                if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
                    return Err(LexError::Budget);
                }
                budget.nodes += 1;
                Ok(LexNode::Bool(true))
            } else {
                Err(LexError::Parse)
            }
        }
        Some(b'f') => {
            if source.get(*cursor..*cursor + 5) == Some(b"false") {
                *cursor += 5;
                if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
                    return Err(LexError::Budget);
                }
                budget.nodes += 1;
                Ok(LexNode::Bool(false))
            } else {
                Err(LexError::Parse)
            }
        }
        Some(b'n') => {
            if source.get(*cursor..*cursor + 4) == Some(b"null") {
                *cursor += 4;
                if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
                    return Err(LexError::Budget);
                }
                budget.nodes += 1;
                Ok(LexNode::Null)
            } else {
                Err(LexError::Parse)
            }
        }
        Some(b'-') | Some(b'0'..=b'9') => parse_lex_number(source, cursor, budget),
        _ => Err(LexError::Parse),
    }
}

fn skip_ws(source: &[u8], cursor: &mut usize) {
    while matches!(source.get(*cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *cursor += 1;
    }
}

fn parse_lex_string(
    source: &[u8],
    cursor: &mut usize,
    budget: &mut LexBudget,
) -> Result<String, LexError> {
    if source.get(*cursor) != Some(&b'"') {
        return Err(LexError::Parse);
    }
    if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
        return Err(LexError::Budget);
    }
    budget.nodes += 1;
    *cursor += 1;
    let mut value = String::new();
    loop {
        let byte = *source.get(*cursor).ok_or(LexError::Parse)?;
        *cursor += 1;
        match byte {
            b'"' => return Ok(value),
            b'\\' => {
                let escaped = *source.get(*cursor).ok_or(LexError::Parse)?;
                *cursor += 1;
                match escaped {
                    b'"' => value.push('"'),
                    b'\\' => value.push('\\'),
                    b'/' => value.push('/'),
                    b'b' => value.push('\u{0008}'),
                    b'f' => value.push('\u{000c}'),
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    b'u' => {
                        let hex = source.get(*cursor..*cursor + 4).ok_or(LexError::Parse)?;
                        *cursor += 4;
                        let first = u32::from_str_radix(
                            std::str::from_utf8(hex).map_err(|_| LexError::Parse)?,
                            16,
                        )
                        .map_err(|_| LexError::Parse)?;
                        if (0xD800..=0xDBFF).contains(&first) {
                            if source.get(*cursor..*cursor + 2) != Some(b"\\u") {
                                return Err(LexError::Parse);
                            }
                            *cursor += 2;
                            let second_hex =
                                source.get(*cursor..*cursor + 4).ok_or(LexError::Parse)?;
                            *cursor += 4;
                            let second = u32::from_str_radix(
                                std::str::from_utf8(second_hex).map_err(|_| LexError::Parse)?,
                                16,
                            )
                            .map_err(|_| LexError::Parse)?;
                            if !(0xDC00..=0xDFFF).contains(&second) {
                                return Err(LexError::Parse);
                            }
                            let codepoint = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                            value.push(char::from_u32(codepoint).ok_or(LexError::Parse)?);
                        } else if (0xDC00..=0xDFFF).contains(&first) {
                            return Err(LexError::Parse);
                        } else {
                            value.push(char::from_u32(first).ok_or(LexError::Parse)?);
                        }
                    }
                    _ => return Err(LexError::Parse),
                }
            }
            0x00..=0x1f => return Err(LexError::Parse),
            _ => {
                let len = utf8_sequence_len(byte).ok_or(LexError::Parse)?;
                let end = *cursor - 1 + len;
                let slice = source.get(*cursor - 1..end).ok_or(LexError::Parse)?;
                let text = std::str::from_utf8(slice).map_err(|_| LexError::Parse)?;
                value.push_str(text);
                *cursor = end;
            }
        }
        if value.len() > WEBSOCKET_JSONRPC_MAX_STRING_BYTES {
            return Err(LexError::Budget);
        }
    }
}

fn utf8_sequence_len(byte: u8) -> Option<usize> {
    match byte {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

fn parse_lex_number(
    source: &[u8],
    cursor: &mut usize,
    budget: &mut LexBudget,
) -> Result<LexNode, LexError> {
    let start = *cursor;
    if source.get(*cursor) == Some(&b'-') {
        *cursor += 1;
    }
    let int_start = *cursor;
    while source.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if *cursor == int_start {
        return Err(LexError::Parse);
    }
    let integer = &source[int_start..*cursor];
    if integer.len() > 1 && integer[0] == b'0' {
        // Leading zeros are a JSON parse error, not an invalidRequest.
        return Err(LexError::Parse);
    }
    if source.get(*cursor) == Some(&b'.') {
        *cursor += 1;
        let fraction_start = *cursor;
        while source.get(*cursor).is_some_and(u8::is_ascii_digit) {
            *cursor += 1;
        }
        if *cursor == fraction_start {
            return Err(LexError::Parse);
        }
    }
    if matches!(source.get(*cursor), Some(b'e') | Some(b'E')) {
        *cursor += 1;
        if matches!(source.get(*cursor), Some(b'+') | Some(b'-')) {
            *cursor += 1;
        }
        let exponent_start = *cursor;
        while source.get(*cursor).is_some_and(u8::is_ascii_digit) {
            *cursor += 1;
        }
        if *cursor == exponent_start {
            return Err(LexError::Parse);
        }
    }
    if budget.nodes >= WEBSOCKET_JSONRPC_MAX_JSON_NODES {
        return Err(LexError::Budget);
    }
    budget.nodes += 1;
    Ok(LexNode::Number(
        std::str::from_utf8(&source[start..*cursor])
            .map_err(|_| LexError::Parse)?
            .to_string(),
    ))
}

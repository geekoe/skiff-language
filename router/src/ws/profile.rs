//! JSON-RPC 2.0 text profile adapter (C-model-connection §5).
//!
//! Peer text classification is consumed from the frozen transport classifier
//! (`classify_jsonrpc_20_text_frame`); this module adds what the frozen
//! classifier intentionally does not carry: the lexical opaque slices of
//! `result` / `error.data` (response terminal) and the outbound/inbound
//! frame encoders. Business params/result/error are never schema-decoded.

use std::fmt;

use skiff_runtime_transport::connection_protocol::{
    classify_jsonrpc_20_text_frame, JsonRpcPlatformErrorKind, OpaquePeerId, ProfileAction,
    WEBSOCKET_JSONRPC_MAX_JSON_DEPTH, WEBSOCKET_JSONRPC_MAX_JSON_NODES,
    WEBSOCKET_JSONRPC_MAX_STRING_BYTES, WEBSOCKET_JSONRPC_MAX_TEXT_BYTES,
};

/// Profile lexical limits (C-model-connection §5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileLimits {
    pub max_text_bytes: usize,
    pub max_json_depth: usize,
    pub max_json_nodes: usize,
    pub max_string_bytes: usize,
}

impl Default for ProfileLimits {
    fn default() -> Self {
        Self {
            max_text_bytes: WEBSOCKET_JSONRPC_MAX_TEXT_BYTES,
            max_json_depth: WEBSOCKET_JSONRPC_MAX_JSON_DEPTH,
            max_json_nodes: WEBSOCKET_JSONRPC_MAX_JSON_NODES,
            max_string_bytes: WEBSOCKET_JSONRPC_MAX_STRING_BYTES,
        }
    }
}

/// Platform error classification for terminal frames (TS
/// `PLATFORM_ERRORS`; C-model-connection §5.1.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformErrorKind {
    Parse,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    Internal,
    ServerBusy,
    Timeout,
}

impl PlatformErrorKind {
    pub fn code(self) -> i64 {
        match self {
            Self::Parse => -32700,
            Self::InvalidRequest => -32600,
            Self::MethodNotFound => -32601,
            Self::InvalidParams => -32602,
            Self::Internal => -32603,
            Self::ServerBusy => -32000,
            Self::Timeout => -32001,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::Parse => "Parse error",
            Self::InvalidRequest => "Invalid Request",
            Self::MethodNotFound => "Method not found",
            Self::InvalidParams => "Invalid params",
            Self::Internal => "Internal error",
            Self::ServerBusy => "Server busy",
            Self::Timeout => "Request timed out",
        }
    }
}

impl From<JsonRpcPlatformErrorKind> for PlatformErrorKind {
    fn from(kind: JsonRpcPlatformErrorKind) -> Self {
        match kind {
            JsonRpcPlatformErrorKind::Parse => Self::Parse,
            JsonRpcPlatformErrorKind::InvalidRequest => Self::InvalidRequest,
            JsonRpcPlatformErrorKind::InvalidParams => Self::InvalidParams,
        }
    }
}

/// Response terminal produced from a classified peer response frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerResponseTerminal {
    Success {
        /// Opaque lexical `result` slice.
        result: Vec<u8>,
    },
    RemoteError {
        code: i64,
        message: String,
        /// Opaque lexical `error.data` slice, if present.
        data: Option<Vec<u8>>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JsonRpc20TextProfile {
    pub limits: ProfileLimits,
}

impl JsonRpc20TextProfile {
    /// Frozen lexical classification (C-model-connection §5.1).
    pub fn classify_text(&self, frame: &[u8]) -> ProfileAction {
        classify_jsonrpc_20_text_frame(frame)
    }

    /// Canonical peer id encoding used inside frames: strings are
    /// JSON-encoded, safe integers are canonical decimal.
    pub fn encode_peer_id(&self, id: &OpaquePeerId) -> String {
        match id {
            OpaquePeerId::String(value) => encode_json_string(value),
            OpaquePeerId::SafeInteger(value) => value.to_string(),
        }
    }

    /// Validates that the Runtime payload is a lexical JSON object/array
    /// within the profile budget and returns the opaque slice verbatim
    /// (C-ws §4.2(7) resourceLimit on failure).
    pub fn materialize_outbound_params(&self, payload: &[u8]) -> Result<Vec<u8>, String> {
        if payload.is_empty() || payload.len() > self.limits.max_text_bytes {
            return Err(
                "outbound params must be present within the profile text limit".to_string(),
            );
        }
        if std::str::from_utf8(payload).is_err() {
            return Err("outbound params must be valid UTF-8".to_string());
        }
        if !lexical_json_container(payload, self.limits)? {
            return Err("outbound params must be a JSON object or array".to_string());
        }
        Ok(payload.to_vec())
    }

    /// Encodes an outbound Runtime request to the peer (C-model-connection
    /// §3.1; business params stay opaque).
    pub fn encode_outbound_request(
        &self,
        id: &OpaquePeerId,
        method: &str,
        params: &[u8],
    ) -> Result<String, String> {
        if method.is_empty() {
            return Err("outbound JSON-RPC method must be non-empty".to_string());
        }
        let frame = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"method\":{},\"params\":{}}}",
            self.encode_peer_id(id),
            encode_json_string(method),
            String::from_utf8_lossy(params)
        );
        self.assert_encoded_frame(frame)
    }

    /// Encodes an inbound success terminal to the peer.
    pub fn encode_result(&self, id: &OpaquePeerId, result: &[u8]) -> Result<String, String> {
        let frame = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{}}}",
            self.encode_peer_id(id),
            String::from_utf8_lossy(result)
        );
        self.assert_encoded_frame(frame)
    }

    /// Encodes a platform error terminal to the peer (id may be null).
    pub fn encode_platform_error(
        &self,
        id: Option<&OpaquePeerId>,
        kind: PlatformErrorKind,
    ) -> Result<String, String> {
        let id = id
            .map(|id| self.encode_peer_id(id))
            .unwrap_or_else(|| "null".to_string());
        let frame = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{},\"error\":{{\"code\":{},\"message\":{}}}}}",
            id,
            kind.code(),
            encode_json_string(kind.message())
        );
        self.assert_encoded_frame(frame)
    }

    /// Extracts the opaque response terminal from a frame already classified
    /// as `Response`. Strict error member validation (only code/message/data,
    /// no duplicates) matches the TS profile (C-model-connection §5.1.6).
    pub fn peer_response_terminal(
        &self,
        frame: &[u8],
        id: &str,
    ) -> Result<PeerResponseTerminal, String> {
        if id.is_empty() {
            return Err("response id must be non-empty".to_string());
        }
        let source = std::str::from_utf8(frame)
            .map_err(|_| "response frame must be valid UTF-8".to_string())?;
        let members = top_level_members(source)
            .ok_or_else(|| "response frame must be a single JSON object".to_string())?;
        let result = member_span(&members, "result");
        let error = member_span(&members, "error");
        match (result, error) {
            (Some(span), None) => Ok(PeerResponseTerminal::Success {
                result: source[span].as_bytes().to_vec(),
            }),
            (None, Some(error_span)) => {
                let error_source = &source[error_span];
                let error_members = top_level_members(error_source)
                    .ok_or_else(|| "response error must be a JSON object".to_string())?;
                if error_members.len() > 3 {
                    return Err("response error object has extra members".to_string());
                }
                let code_span = member_span(&error_members, "code")
                    .ok_or_else(|| "response error requires code".to_string())?;
                let message_span = member_span(&error_members, "message")
                    .ok_or_else(|| "response error requires message".to_string())?;
                let data_span = member_span(&error_members, "data");
                let code_lexeme = error_source[code_span].trim();
                let code = parse_safe_integer_i64(code_lexeme)
                    .ok_or_else(|| "response error code must be a safe integer".to_string())?;
                let message_raw = error_source[message_span].trim();
                let message = decode_json_string(message_raw)
                    .ok_or_else(|| "response error message must be a string".to_string())?;
                if message.is_empty() || message.len() > self.limits.max_string_bytes {
                    return Err("response error message must be non-empty and bounded".to_string());
                }
                let data = data_span.map(|span| error_source[span].as_bytes().to_vec());
                Ok(PeerResponseTerminal::RemoteError {
                    code,
                    message,
                    data,
                })
            }
            _ => Err("response must contain exactly one of result or error".to_string()),
        }
    }

    fn assert_encoded_frame(&self, frame: String) -> Result<String, String> {
        if frame.len() > self.limits.max_text_bytes {
            return Err("encoded JSON-RPC frame exceeds the profile text limit".to_string());
        }
        Ok(frame)
    }
}

/// Decodes a strict JSON string literal (escapes included).
fn decode_json_string(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut index = 1;
    while index + 1 < bytes.len() {
        let byte = bytes[index];
        index += 1;
        match byte {
            b'\\' => {
                let escaped = *bytes.get(index)?;
                index += 1;
                match escaped {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        let hex = bytes.get(index..index + 4)?;
                        index += 4;
                        let codepoint =
                            u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        out.push(char::from_u32(codepoint as u32)?);
                    }
                    _ => return None,
                }
            }
            byte if byte < 0x20 => return None,
            _ => {
                // Corpus frames are UTF-8; copy the full character.
                let rest = &raw[index - 1..];
                let ch = rest.chars().next()?;
                out.push(ch);
                index += ch.len_utf8() - 1;
            }
        }
    }
    Some(out)
}

/// Encodes a string as a strict JSON string literal.
fn encode_json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Lexically validates that `payload` is exactly one JSON object or array
/// within depth/node/string budgets.
fn lexical_json_container(payload: &[u8], limits: ProfileLimits) -> Result<bool, String> {
    let source = std::str::from_utf8(payload).map_err(|_| "payload must be UTF-8".to_string())?;
    let mut scanner = Scanner {
        source,
        index: 0,
        depth: 0,
        nodes: 0,
        limits,
    };
    scanner.skip_ws();
    let first = scanner.source.as_bytes().get(scanner.index).copied();
    if !matches!(first, Some(b'{') | Some(b'[')) {
        return Ok(false);
    }
    let valid = scanner.scan_value();
    scanner.skip_ws();
    Ok(valid && scanner.index == scanner.source.len())
}

struct Scanner<'a> {
    source: &'a str,
    index: usize,
    depth: usize,
    nodes: usize,
    limits: ProfileLimits,
}

impl Scanner<'_> {
    fn skip_ws(&mut self) {
        while matches!(
            self.source.as_bytes().get(self.index),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.index += 1;
        }
    }

    fn scan_value(&mut self) -> bool {
        self.nodes += 1;
        if self.nodes > self.limits.max_json_nodes {
            return false;
        }
        self.skip_ws();
        let Some(byte) = self.source.as_bytes().get(self.index).copied() else {
            return false;
        };
        match byte {
            b'{' => self.scan_object(),
            b'[' => self.scan_array(),
            b'"' => self.scan_string(),
            b't' => self.consume_literal("true"),
            b'f' => self.consume_literal("false"),
            b'n' => self.consume_literal("null"),
            b'-' | b'0'..=b'9' => self.scan_number(),
            _ => false,
        }
    }

    fn scan_object(&mut self) -> bool {
        self.depth += 1;
        if self.depth > self.limits.max_json_depth {
            return false;
        }
        self.index += 1;
        self.skip_ws();
        if self.source.as_bytes().get(self.index) == Some(&b'}') {
            self.index += 1;
            self.depth -= 1;
            return true;
        }
        loop {
            self.skip_ws();
            if !self.scan_string() {
                return false;
            }
            self.skip_ws();
            if self.source.as_bytes().get(self.index) != Some(&b':') {
                return false;
            }
            self.index += 1;
            if !self.scan_value() {
                return false;
            }
            self.skip_ws();
            match self.source.as_bytes().get(self.index) {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b'}') => {
                    self.index += 1;
                    self.depth -= 1;
                    return true;
                }
                _ => return false,
            }
        }
    }

    fn scan_array(&mut self) -> bool {
        self.depth += 1;
        if self.depth > self.limits.max_json_depth {
            return false;
        }
        self.index += 1;
        self.skip_ws();
        if self.source.as_bytes().get(self.index) == Some(&b']') {
            self.index += 1;
            self.depth -= 1;
            return true;
        }
        loop {
            if !self.scan_value() {
                return false;
            }
            self.skip_ws();
            match self.source.as_bytes().get(self.index) {
                Some(b',') => {
                    self.index += 1;
                }
                Some(b']') => {
                    self.index += 1;
                    self.depth -= 1;
                    return true;
                }
                _ => return false,
            }
        }
    }

    fn scan_string(&mut self) -> bool {
        let bytes = self.source.as_bytes();
        if bytes.get(self.index) != Some(&b'"') {
            return false;
        }
        self.index += 1;
        let start = self.index;
        loop {
            let Some(byte) = bytes.get(self.index).copied() else {
                return false;
            };
            self.index += 1;
            match byte {
                b'"' => {
                    if self.index - start > self.limits.max_string_bytes {
                        return false;
                    }
                    return true;
                }
                b'\\' => {
                    if self.index >= bytes.len() {
                        return false;
                    }
                    self.index += 1;
                }
                byte if byte < 0x20 => return false,
                _ => {}
            }
        }
    }

    fn scan_number(&mut self) -> bool {
        let bytes = self.source.as_bytes();
        let start = self.index;
        if bytes.get(self.index) == Some(&b'-') {
            self.index += 1;
        }
        match bytes.get(self.index) {
            Some(b'0') => self.index += 1,
            Some(b'1'..=b'9') => {
                while matches!(bytes.get(self.index), Some(b'0'..=b'9')) {
                    self.index += 1;
                }
            }
            _ => return false,
        }
        if bytes.get(self.index) == Some(&b'.') {
            self.index += 1;
            let fraction_start = self.index;
            while matches!(bytes.get(self.index), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == fraction_start {
                return false;
            }
        }
        if matches!(bytes.get(self.index), Some(b'e') | Some(b'E')) {
            self.index += 1;
            if matches!(bytes.get(self.index), Some(b'+') | Some(b'-')) {
                self.index += 1;
            }
            let exponent_start = self.index;
            while matches!(bytes.get(self.index), Some(b'0'..=b'9')) {
                self.index += 1;
            }
            if self.index == exponent_start {
                return false;
            }
        }
        self.index > start
    }

    fn consume_literal(&mut self, literal: &str) -> bool {
        if self.source[self.index..].starts_with(literal) {
            self.index += literal.len();
            true
        } else {
            false
        }
    }
}

/// Returns the byte span of one top-level member value, or `None`.
fn member_span(
    members: &[(String, std::ops::Range<usize>)],
    key: &str,
) -> Option<std::ops::Range<usize>> {
    members
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, span)| span.clone())
}

/// Scans a JSON object's top-level members (duplicate keys rejected) and
/// returns `(key, value_span)` pairs. The value span is lexeme-preserving.
fn top_level_members(source: &str) -> Option<Vec<(String, std::ops::Range<usize>)>> {
    let bytes = source.as_bytes();
    let mut index = 0;
    skip_ws_at(bytes, &mut index);
    if bytes.get(index) != Some(&b'{') {
        return None;
    }
    index += 1;
    skip_ws_at(bytes, &mut index);
    if bytes.get(index) == Some(&b'}') {
        return Some(Vec::new());
    }
    let mut members = Vec::new();
    let mut seen = std::collections::HashSet::new();
    loop {
        skip_ws_at(bytes, &mut index);
        let key_start = index;
        if !scan_string_span(bytes, &mut index) {
            return None;
        }
        let key = std::str::from_utf8(&bytes[key_start..index])
            .ok()
            .and_then(decode_json_string)?;
        if !seen.insert(key.clone()) {
            return None;
        }
        skip_ws_at(bytes, &mut index);
        if bytes.get(index) != Some(&b':') {
            return None;
        }
        index += 1;
        skip_ws_at(bytes, &mut index);
        let value_start = index;
        if !scan_value_span(bytes, &mut index) {
            return None;
        }
        members.push((key, value_start..index));
        skip_ws_at(bytes, &mut index);
        match bytes.get(index) {
            Some(b',') => {
                index += 1;
            }
            Some(b'}') => return Some(members),
            _ => return None,
        }
    }
}

fn skip_ws_at(bytes: &[u8], index: &mut usize) {
    while matches!(bytes.get(*index), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *index += 1;
    }
}

fn scan_string_span(bytes: &[u8], index: &mut usize) -> bool {
    if bytes.get(*index) != Some(&b'"') {
        return false;
    }
    *index += 1;
    loop {
        let Some(byte) = bytes.get(*index).copied() else {
            return false;
        };
        *index += 1;
        match byte {
            b'"' => return true,
            b'\\' => {
                if *index >= bytes.len() {
                    return false;
                }
                *index += 1;
            }
            byte if byte < 0x20 => return false,
            _ => {}
        }
    }
}

fn scan_value_span(bytes: &[u8], index: &mut usize) -> bool {
    skip_ws_at(bytes, index);
    let Some(byte) = bytes.get(*index).copied() else {
        return false;
    };
    match byte {
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut in_string = false;
            loop {
                let Some(current) = bytes.get(*index).copied() else {
                    return false;
                };
                *index += 1;
                if in_string {
                    match current {
                        b'\\' => {
                            if *index >= bytes.len() {
                                return false;
                            }
                            *index += 1;
                        }
                        b'"' => in_string = false,
                        _ => {}
                    }
                    continue;
                }
                match current {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        b'"' => scan_string_span(bytes, index),
        b't' => consume_at(bytes, index, "true"),
        b'f' => consume_at(bytes, index, "false"),
        b'n' => consume_at(bytes, index, "null"),
        b'-' | b'0'..=b'9' => {
            let start = *index;
            let mut cursor = *index;
            if bytes.get(cursor) == Some(&b'-') {
                cursor += 1;
            }
            while matches!(bytes.get(cursor), Some(b'0'..=b'9')) {
                cursor += 1;
            }
            if bytes.get(cursor) == Some(&b'.') {
                cursor += 1;
                while matches!(bytes.get(cursor), Some(b'0'..=b'9')) {
                    cursor += 1;
                }
            }
            if matches!(bytes.get(cursor), Some(b'e') | Some(b'E')) {
                cursor += 1;
                if matches!(bytes.get(cursor), Some(b'+') | Some(b'-')) {
                    cursor += 1;
                }
                while matches!(bytes.get(cursor), Some(b'0'..=b'9')) {
                    cursor += 1;
                }
            }
            if cursor == start {
                return false;
            }
            *index = cursor;
            true
        }
        _ => false,
    }
}

fn consume_at(bytes: &[u8], index: &mut usize, literal: &str) -> bool {
    if bytes
        .get(*index..)
        .is_some_and(|rest| rest.starts_with(literal.as_bytes()))
    {
        *index += literal.len();
        true
    } else {
        false
    }
}

/// Lexeme-exact safe integer parse (mirror of C-model-connection §5.2 rules;
/// bounded to i64 since the frozen wire codes are i64 safe integers).
pub(crate) fn parse_safe_integer_i64(lexeme: &str) -> Option<i64> {
    if lexeme.is_empty() {
        return None;
    }
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
    const MAX_SAFE: i128 = 9_007_199_254_740_991;
    if !(-MAX_SAFE..=MAX_SAFE).contains(&exact) {
        return None;
    }
    Some(exact as i64)
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

impl fmt::Debug for Scanner<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Scanner")
            .field("index", &self.index)
            .field("depth", &self.depth)
            .field("nodes", &self.nodes)
            .finish_non_exhaustive()
    }
}

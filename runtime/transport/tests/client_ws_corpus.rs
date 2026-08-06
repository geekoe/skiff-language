//! Byte-exact client WS corpus verifier for C-model-connection + C-client-lifecycle + C-ws
//! (`doc/implementation/router-rust-migration-c-model-connection-contract.md`,
//! `...-c-client-lifecycle-contract.md`, `...-c-ws-contract.md`).
//!
//! This is a TEST-ONLY reference model. It is not production code, is not
//! imported by any production crate, and must not be treated as the
//! W-WebSocket implementation. W-WebSocket must implement the frozen
//! semantics (owner/invariant in the contract docs) and consume the same
//! fixtures.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use serde_json::{json, Value};
use skiff_artifact_model::WebSocketEntryId;
use skiff_runtime_transport::{
    connection_protocol::{
        decode_connection_request_cancel_frame, decode_connection_request_frame,
        decode_connection_response_frame, encode_connection_request_cancel_frame,
        encode_connection_request_frame, encode_connection_response_frame,
        ConnectionRemoteErrorFrameHeader, ConnectionRequestCancelFrameHeader,
        ConnectionRequestFrameHeader, ConnectionResponseFrameHeader, ConnectionResponseOutcome,
        WebSocketRpcProfile,
    },
    protocol::{RuntimeDeadlineFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION},
    websocket_generation_lifecycle::{
        assert_websocket_generation_lifecycle_response_matches,
        decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
        WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
        WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleRejectionCode,
        WebSocketGenerationLifecycleSender, WebSocketGenerationLifecycleTuple,
    },
};

const REQUIRED_FRAMES: [&str; 17] = [
    "connection.request.object",
    "connection.request.array",
    "connection.request.no-deadline",
    "connection.request.cancel",
    "connection.response.success",
    "connection.response.remote",
    "connection.response.deadline-exceeded",
    "connection.response.connection-unavailable",
    "connection.response.transport-unavailable",
    "connection.response.protocol-error",
    "connection.response.resource-limit",
    "lifecycle.acquire",
    "lifecycle.release",
    "lifecycle.ack.acquire",
    "lifecycle.ack.release",
    "lifecycle.reject.acquire",
    "lifecycle.reject.release",
];

const REQUIRED_SCENARIOS: [&str; 23] = [
    "01-accept-and-rpc-roundtrip",
    "02-peer-close-terminal",
    "03-business-replacement-close-oldest",
    "04-ranked-replacement-supersedes",
    "05-reject-new-preserves-existing",
    "06-runtime-disconnect-terminal",
    "07-shutdown-drains-finalizers",
    "08-slow-client-saturation",
    "09-captured-writer-stale-write-fence",
    "10-outbound-deadline-terminal",
    "11-broker-tombstone-late-response-isolation",
    "12-four-way-replacement-then-peer-close",
    "13-four-way-peer-close-then-replacement",
    "14-four-way-replacement-then-runtime-disconnect",
    "15-four-way-runtime-disconnect-then-replacement",
    "16-four-way-peer-close-then-shutdown",
    "17-four-way-shutdown-then-peer-close",
    "18-four-way-runtime-disconnect-then-shutdown",
    "19-release-timeout-terminal",
    "20-inbound-deadline-terminal",
    "21-broker-outbound-capacity-resource-limit",
    "22-duplicate-peer-request-id",
    "23-runtime-cancel-outbound",
];

fn tuple() -> WebSocketGenerationLifecycleTuple {
    WebSocketGenerationLifecycleTuple {
        router_session_id: "skiff-router-session-v1:opaque:router-1".to_string(),
        service_id: "example.com/chat".to_string(),
        build_id: format!("skiff-service-deployment-v2:sha256:{}", "a".repeat(64)),
        websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64)),
        connection_id: "connection-1".to_string(),
    }
}

fn request_header(deadline: bool) -> ConnectionRequestFrameHeader {
    ConnectionRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request".to_string(),
        request_id: "conn-req-1".to_string(),
        service_id: "example.com/chat".to_string(),
        websocket_entry_id: WebSocketEntryId::parse(format!(
            "skiff-websocket-entry-v1:sha256:{}",
            "b".repeat(64)
        ))
        .unwrap(),
        connection_id: "connection-1".to_string(),
        profile: WebSocketRpcProfile::JsonRpc2_0Text,
        method: "chat.send".to_string(),
        deadline: deadline.then(|| RuntimeDeadlineFrameHeader {
            timeout_ms: 120_000,
            expires_at: "2026-08-02T00:00:00Z".to_string(),
        }),
    }
}

fn cancel_header() -> ConnectionRequestCancelFrameHeader {
    ConnectionRequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request.cancel".to_string(),
        request_id: "conn-req-1".to_string(),
        reason: skiff_runtime_transport::cancel_reason::RequestCancelReason::ClientDisconnect,
    }
}

fn response_header(outcome: ConnectionResponseOutcome) -> ConnectionResponseFrameHeader {
    ConnectionResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.response".to_string(),
        request_id: "conn-req-1".to_string(),
        outcome,
        remote: None,
    }
}

fn lifecycle_control(
    action: &str,
    operation: WebSocketGenerationLifecycleOperation,
    code: Option<WebSocketGenerationLifecycleRejectionCode>,
) -> WebSocketGenerationLifecycleControl {
    let tuple = tuple();
    match action {
        "acquire" => WebSocketGenerationLifecycleControl::Acquire {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            request_id: "skiff-websocket-lifecycle-request-v1:opaque:acquire-1".to_string(),
            sender: WebSocketGenerationLifecycleSender::Runtime,
            tuple,
        },
        "release" => WebSocketGenerationLifecycleControl::Release {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            request_id: "skiff-websocket-lifecycle-request-v1:opaque:release-1".to_string(),
            sender: WebSocketGenerationLifecycleSender::Router,
            tuple,
        },
        "ack" => WebSocketGenerationLifecycleControl::Ack {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            operation,
            request_id: request_id_for(operation),
            sender: sender_for(operation),
            tuple,
        },
        "reject" => WebSocketGenerationLifecycleControl::Reject {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            operation,
            request_id: request_id_for(operation),
            sender: sender_for(operation),
            tuple,
            code: code.unwrap_or(WebSocketGenerationLifecycleRejectionCode::TupleMismatch),
            reason: "tuple does not match the frozen expectation".to_string(),
        },
        other => panic!("unknown lifecycle action {other}"),
    }
}

fn request_id_for(operation: WebSocketGenerationLifecycleOperation) -> String {
    match operation {
        WebSocketGenerationLifecycleOperation::Acquire => {
            "skiff-websocket-lifecycle-request-v1:opaque:acquire-1".to_string()
        }
        WebSocketGenerationLifecycleOperation::Release => {
            "skiff-websocket-lifecycle-request-v1:opaque:release-1".to_string()
        }
    }
}

fn sender_for(
    operation: WebSocketGenerationLifecycleOperation,
) -> WebSocketGenerationLifecycleSender {
    match operation {
        WebSocketGenerationLifecycleOperation::Acquire => {
            WebSocketGenerationLifecycleSender::Router
        }
        WebSocketGenerationLifecycleOperation::Release => {
            WebSocketGenerationLifecycleSender::Runtime
        }
    }
}

fn direction_for(
    action: &str,
    operation: Option<WebSocketGenerationLifecycleOperation>,
) -> WebSocketGenerationLifecycleDirection {
    match action {
        "acquire" => WebSocketGenerationLifecycleDirection::RuntimeToRouter,
        "release" => WebSocketGenerationLifecycleDirection::RouterToRuntime,
        "ack" | "reject" => match operation.expect("operation required") {
            WebSocketGenerationLifecycleOperation::Acquire => {
                WebSocketGenerationLifecycleDirection::RouterToRuntime
            }
            WebSocketGenerationLifecycleOperation::Release => {
                WebSocketGenerationLifecycleDirection::RuntimeToRouter
            }
        },
        other => panic!("unknown lifecycle action {other}"),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn load_catalog() -> Catalog {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("client-ws")
        .join("frames.json");
    let text = std::fs::read_to_string(&path).expect("frames.json must exist");
    serde_json::from_str(&text).expect("frames.json must parse")
}

#[derive(Debug, Clone)]
enum CatalogFrame {
    ConnectionRequest,
    ConnectionCancel,
    ConnectionResponse,
    Lifecycle {
        action: String,
        operation: Option<WebSocketGenerationLifecycleOperation>,
    },
}

fn decode_catalog_frame(entry: &FrameEntry) -> CatalogFrame {
    let bytes = hex_decode(&entry.frame_hex);
    match entry.decode_as.as_str() {
        "ConnectionRequest" => {
            let (header, payload) =
                decode_connection_request_frame(&bytes).expect("connection request decodes");
            let reencoded = encode_connection_request_frame(&header, &payload)
                .expect("connection request re-encodes");
            assert_eq!(reencoded, bytes, "connection request must be byte-exact");
            assert_eq!(
                serde_json::to_value(&header).unwrap(),
                entry.header,
                "connection request header JSON must match"
            );
            let _ = (header.deadline.is_some(), payload);
            CatalogFrame::ConnectionRequest
        }
        "ConnectionCancel" => {
            let header =
                decode_connection_request_cancel_frame(&bytes).expect("connection cancel decodes");
            let reencoded = encode_connection_request_cancel_frame(&header)
                .expect("connection cancel re-encodes");
            assert_eq!(reencoded, bytes, "connection cancel must be byte-exact");
            assert_eq!(
                serde_json::to_value(&header).unwrap(),
                entry.header,
                "connection cancel header JSON must match"
            );
            CatalogFrame::ConnectionCancel
        }
        "ConnectionResponse" => {
            let (header, payload) =
                decode_connection_response_frame(&bytes).expect("connection response decodes");
            let reencoded = encode_connection_response_frame(&header, &payload)
                .expect("connection response re-encodes");
            assert_eq!(reencoded, bytes, "connection response must be byte-exact");
            assert_eq!(
                serde_json::to_value(&header).unwrap(),
                entry.header,
                "connection response header JSON must match"
            );
            let _ = (header.outcome, payload);
            CatalogFrame::ConnectionResponse
        }
        "Lifecycle" => {
            let direction = match entry.direction.as_str() {
                "RouterToRuntime" => WebSocketGenerationLifecycleDirection::RouterToRuntime,
                "RuntimeToRouter" => WebSocketGenerationLifecycleDirection::RuntimeToRouter,
                other => panic!("unknown lifecycle direction {other}"),
            };
            let control = decode_websocket_generation_lifecycle_frame(direction, &bytes)
                .expect("lifecycle decodes");
            let reencoded = encode_websocket_generation_lifecycle_frame(direction, &control)
                .expect("lifecycle re-encodes");
            assert_eq!(reencoded, bytes, "lifecycle must be byte-exact");
            assert_eq!(
                serde_json::to_value(&control).unwrap(),
                entry.header,
                "lifecycle header JSON must match"
            );
            let (action, operation) = match &control {
                WebSocketGenerationLifecycleControl::Acquire { .. } => ("acquire", None),
                WebSocketGenerationLifecycleControl::Release { .. } => ("release", None),
                WebSocketGenerationLifecycleControl::Ack { operation, .. } => {
                    ("ack", Some(*operation))
                }
                WebSocketGenerationLifecycleControl::Reject { operation, .. } => {
                    ("reject", Some(*operation))
                }
            };
            CatalogFrame::Lifecycle {
                action: action.to_string(),
                operation,
            }
        }
        other => panic!("unknown decodeAs {other}"),
    }
}

#[test]
fn frame_catalog_is_byte_exact_and_complete() {
    let catalog = load_catalog();
    assert_eq!(catalog.schema_version, 1);
    assert_eq!(catalog.corpus, "client-ws-frames-v1");
    for required in REQUIRED_FRAMES {
        assert!(
            catalog.frames.contains_key(required),
            "required frame {required} missing from catalog"
        );
    }
    for (name, entry) in &catalog.frames {
        let semantic = decode_catalog_frame(entry);
        let expected_frame_type = match &semantic {
            CatalogFrame::ConnectionRequest => "connection.request",
            CatalogFrame::ConnectionCancel => "connection.request.cancel",
            CatalogFrame::ConnectionResponse => "connection.response",
            CatalogFrame::Lifecycle { .. } => "websocket.generation.lifecycle",
        };
        assert_eq!(
            entry.frame_type, expected_frame_type,
            "{name}: frameType must match decodeAs"
        );
        match &semantic {
            CatalogFrame::ConnectionRequest | CatalogFrame::ConnectionCancel => {
                assert_eq!(entry.direction, "RuntimeToRouter");
            }
            CatalogFrame::ConnectionResponse => {
                assert_eq!(entry.direction, "RouterToRuntime");
            }
            CatalogFrame::Lifecycle { action, operation } => {
                assert_eq!(
                    entry.direction,
                    match direction_for(action, *operation) {
                        WebSocketGenerationLifecycleDirection::RouterToRuntime => "RouterToRuntime",
                        WebSocketGenerationLifecycleDirection::RuntimeToRouter => {
                            "RuntimeToRouter"
                        }
                    }
                );
            }
        }
        assert!(
            !entry.frame_hex.is_empty() && entry.frame_hex.len() % 2 == 0,
            "{name}: frameHex must be even-length hex"
        );
    }
}

#[test]
fn lifecycle_responses_must_echo_exact_request() {
    let acquire = lifecycle_control(
        "acquire",
        WebSocketGenerationLifecycleOperation::Acquire,
        None,
    );
    let ack = lifecycle_control("ack", WebSocketGenerationLifecycleOperation::Acquire, None);
    let reject = lifecycle_control(
        "reject",
        WebSocketGenerationLifecycleOperation::Acquire,
        None,
    );
    assert!(
        assert_websocket_generation_lifecycle_response_matches(&acquire, &ack).is_ok(),
        "acquire ack must echo the exact request"
    );
    assert!(
        assert_websocket_generation_lifecycle_response_matches(&acquire, &reject).is_ok(),
        "acquire reject must echo the exact request"
    );

    let release = lifecycle_control(
        "release",
        WebSocketGenerationLifecycleOperation::Release,
        None,
    );
    let ack = lifecycle_control("ack", WebSocketGenerationLifecycleOperation::Release, None);
    assert!(
        assert_websocket_generation_lifecycle_response_matches(&release, &ack).is_ok(),
        "release ack must echo the exact request"
    );
    // Cross-operation responses must not match.
    assert!(
        assert_websocket_generation_lifecycle_response_matches(&acquire, &ack).is_err(),
        "release ack must not satisfy an acquire request"
    );
}

#[test]
fn mutated_frame_is_rejected_by_codec() {
    let catalog = load_catalog();
    let request = catalog
        .frames
        .get("connection.request.object")
        .expect("connection request frame");
    let mut bytes = hex_decode(&request.frame_hex);
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    assert!(
        decode_connection_request_frame(&bytes).is_err(),
        "mutated request bytes must not decode; byte-exactness is a hard contract"
    );
}

// ---------------------------------------------------------------------------
// JSON-RPC 2.0 text numeric id lexical corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct JsonRpcIdCase {
    name: String,
    frame: String,
    kind: String,
    #[serde(rename = "idKind")]
    id_kind: Option<String>,
    id: Option<String>,
    #[serde(rename = "peerKey")]
    peer_key: Option<String>,
    #[serde(rename = "errorKind")]
    error_kind: Option<String>,
    code: Option<u16>,
}

#[derive(Debug, Clone, Deserialize)]
struct JsonRpcIdCorpus {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    cases: Vec<JsonRpcIdCase>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PeerId {
    String(String),
    SafeInteger(i128),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LexicalAction {
    Request { id: PeerId, method: String },
    Response { id: String },
    Notification { method: String },
    PlatformError { error: String },
    Close { code: u16 },
}

/// TEST-ONLY lexeme-preserving JSON subset parser. It keeps number lexemes
/// verbatim (`1e0`, `-0`, `1.0000000000000000001`) because serde_json's
/// default `Number` normalizes them and would corrupt the frozen lexical
/// contract (C-model-connection §5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
enum LexNode {
    Object(Vec<(String, LexNode)>),
    Array(Vec<LexNode>),
    String(String),
    Number(String),
    Bool(bool),
    Null,
}

fn parse_lex_node(source: &[u8], cursor: &mut usize) -> Option<LexNode> {
    skip_ws(source, cursor);
    match source.get(*cursor).copied()? {
        b'{' => {
            *cursor += 1;
            let mut members = Vec::new();
            skip_ws(source, cursor);
            if source.get(*cursor) == Some(&b'}') {
                *cursor += 1;
                return Some(LexNode::Object(members));
            }
            loop {
                let key = parse_lex_string(source, cursor)?;
                skip_ws(source, cursor);
                if source.get(*cursor) != Some(&b':') {
                    return None;
                }
                *cursor += 1;
                let value = parse_lex_node(source, cursor)?;
                members.push((key, value));
                skip_ws(source, cursor);
                match source.get(*cursor) {
                    Some(b',') => {
                        *cursor += 1;
                    }
                    Some(b'}') => {
                        *cursor += 1;
                        return Some(LexNode::Object(members));
                    }
                    _ => return None,
                }
            }
        }
        b'[' => {
            *cursor += 1;
            let mut items = Vec::new();
            skip_ws(source, cursor);
            if source.get(*cursor) == Some(&b']') {
                *cursor += 1;
                return Some(LexNode::Array(items));
            }
            loop {
                items.push(parse_lex_node(source, cursor)?);
                skip_ws(source, cursor);
                match source.get(*cursor) {
                    Some(b',') => {
                        *cursor += 1;
                    }
                    Some(b']') => {
                        *cursor += 1;
                        return Some(LexNode::Array(items));
                    }
                    _ => return None,
                }
            }
        }
        b'"' => Some(LexNode::String(parse_lex_string(source, cursor)?)),
        b't' => {
            if source.get(*cursor..*cursor + 4) == Some(b"true") {
                *cursor += 4;
                Some(LexNode::Bool(true))
            } else {
                None
            }
        }
        b'f' => {
            if source.get(*cursor..*cursor + 5) == Some(b"false") {
                *cursor += 5;
                Some(LexNode::Bool(false))
            } else {
                None
            }
        }
        b'n' => {
            if source.get(*cursor..*cursor + 4) == Some(b"null") {
                *cursor += 4;
                Some(LexNode::Null)
            } else {
                None
            }
        }
        b'-' | b'0'..=b'9' => parse_lex_number(source, cursor),
        _ => None,
    }
}

fn skip_ws(source: &[u8], cursor: &mut usize) {
    while matches!(source.get(*cursor), Some(b' ' | b'\t' | b'\n' | b'\r')) {
        *cursor += 1;
    }
}

fn parse_lex_string(source: &[u8], cursor: &mut usize) -> Option<String> {
    if source.get(*cursor) != Some(&b'"') {
        return None;
    }
    *cursor += 1;
    let mut value = String::new();
    while let Some(byte) = source.get(*cursor).copied() {
        *cursor += 1;
        match byte {
            b'"' => return Some(value),
            b'\\' => {
                let escaped = *source.get(*cursor)?;
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
                        let hex = source.get(*cursor..*cursor + 4)?;
                        *cursor += 4;
                        let codepoint =
                            u16::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
                        value.push(char::from_u32(codepoint as u32)?);
                    }
                    _ => return None,
                }
            }
            byte if byte < 0x20 => return None,
            _ => {
                // Corpus frames are ASCII; decode single byte into the
                // surrounding valid UTF-8 string.
                value.push(byte as char);
            }
        }
    }
    None
}

fn parse_lex_number(source: &[u8], cursor: &mut usize) -> Option<LexNode> {
    let start = *cursor;
    if source.get(*cursor) == Some(&b'-') {
        *cursor += 1;
    }
    let int_start = *cursor;
    while source.get(*cursor).is_some_and(u8::is_ascii_digit) {
        *cursor += 1;
    }
    if *cursor == int_start {
        return None;
    }
    let integer = &source[int_start..*cursor];
    if integer.len() > 1 && integer[0] == b'0' {
        // JSON numbers must not have leading zeros; the lossless parser
        // rejects the frame at parse time (parse error, not invalidRequest).
        return None;
    }
    if source.get(*cursor) == Some(&b'.') {
        *cursor += 1;
        let fraction_start = *cursor;
        while source.get(*cursor).is_some_and(u8::is_ascii_digit) {
            *cursor += 1;
        }
        if *cursor == fraction_start {
            return None;
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
            return None;
        }
    }
    Some(LexNode::Number(
        std::str::from_utf8(&source[start..*cursor])
            .ok()?
            .to_string(),
    ))
}

fn load_jsonrpc_ids() -> JsonRpcIdCorpus {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("client-ws")
        .join("jsonrpc-ids.json");
    let text = std::fs::read_to_string(&path).expect("jsonrpc-ids.json must exist");
    serde_json::from_str(&text).expect("jsonrpc-ids.json must parse")
}

/// TEST-ONLY lexical classifier frozen by C-model-connection §5. It mirrors
/// the canonical TS `JsonRpc20TextProfile.classifyText` behavior for the
/// corpus subset: object frames with jsonrpc/id/method/params/result/error.
fn classify_lexeme(source: &str) -> LexicalAction {
    let mut cursor = 0;
    let node = match parse_lex_node(source.as_bytes(), &mut cursor) {
        Some(node) if cursor == source.len() => node,
        _ => {
            return LexicalAction::PlatformError {
                error: "parse".to_string(),
            }
        }
    };
    let LexNode::Object(members) = node else {
        return LexicalAction::PlatformError {
            error: "invalidRequest".to_string(),
        };
    };
    let mut seen = HashSet::new();
    for (key, _) in &members {
        if !seen.insert(key.clone()) {
            // `uniqueObjectMembers` treats duplicate members as an invalid
            // request, not a parse error.
            return LexicalAction::PlatformError {
                error: "invalidRequest".to_string(),
            };
        }
    }
    let field_names = members
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<HashSet<_>>();
    let response_candidate = field_names.contains("result")
        || field_names.contains("error")
        || (field_names.contains("id") && !field_names.contains("method"));
    if response_candidate {
        return classify_response(source, &members);
    }
    if field_names.contains("method")
        && !field_names.contains("result")
        && !field_names.contains("error")
    {
        return classify_request_or_notification(source, &members);
    }
    LexicalAction::PlatformError {
        error: "invalidRequest".to_string(),
    }
}

fn classify_request_or_notification(_source: &str, members: &[(String, LexNode)]) -> LexicalAction {
    let allowed_with_id = ["jsonrpc", "id", "method", "params"];
    let allowed_without_id = ["jsonrpc", "method", "params"];
    let has_id = members.iter().any(|(key, _)| key == "id");
    let allowed = if has_id {
        &allowed_with_id[..]
    } else {
        &allowed_without_id[..]
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
        return LexicalAction::PlatformError {
            error: "invalidRequest".to_string(),
        };
    }
    let method = match member("method") {
        Some(LexNode::String(method)) if !method.is_empty() => method.clone(),
        _ => {
            return LexicalAction::PlatformError {
                error: "invalidRequest".to_string(),
            }
        }
    };
    if !has_id {
        return LexicalAction::Notification { method };
    }
    let id = match parse_peer_id(member("id")) {
        Some(id) => id,
        None => {
            return LexicalAction::PlatformError {
                error: "invalidRequest".to_string(),
            }
        }
    };
    match member("params") {
        Some(LexNode::Object(_)) | Some(LexNode::Array(_)) => LexicalAction::Request { id, method },
        Some(_) | None => LexicalAction::PlatformError {
            error: "invalidParams".to_string(),
        },
    }
}

fn classify_response(_source: &str, members: &[(String, LexNode)]) -> LexicalAction {
    let member = |key: &str| {
        members
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    let has_result = members.iter().any(|(key, _)| key == "result");
    let has_error = members.iter().any(|(key, _)| key == "error");
    if has_result == has_error || member("jsonrpc") != Some(&LexNode::String("2.0".to_string())) {
        return LexicalAction::Close { code: 1002 };
    }
    let id_node = member("id");
    let id = match id_node {
        Some(LexNode::String(value)) if !value.is_empty() => value.clone(),
        _ => return LexicalAction::Close { code: 1002 },
    };
    if has_result {
        if members
            .iter()
            .any(|(key, _)| !["jsonrpc", "id", "result"].contains(&key.as_str()))
        {
            return LexicalAction::Close { code: 1002 };
        }
        return LexicalAction::Response { id };
    }
    if members
        .iter()
        .any(|(key, _)| !["jsonrpc", "id", "error"].contains(&key.as_str()))
    {
        return LexicalAction::Close { code: 1002 };
    }
    let error = member("error");
    let Some(LexNode::Object(error)) = error else {
        return LexicalAction::Close { code: 1002 };
    };
    let error_member = |key: &str| {
        error
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    };
    if error
        .iter()
        .any(|(key, _)| !["code", "message", "data"].contains(&key.as_str()))
        || parse_safe_integer(error_member("code")).is_none()
        || !matches!(error_member("message"), Some(LexNode::String(message)) if !message.is_empty())
    {
        return LexicalAction::Close { code: 1002 };
    }
    LexicalAction::Response { id }
}

/// TEST-ONLY lexeme validator/canonicalizer frozen by C-model-connection §5.2:
/// validate the number lexeme as an exact safe integer, then canonicalize
/// (`1e0` -> `1`, `-0` -> `0`). Mirrors the canonical TS
/// `parseSafeInteger`/`canonicalSafeInteger`.
fn parse_safe_integer(node: Option<&LexNode>) -> Option<i128> {
    let LexNode::Number(lexeme) = node? else {
        return None;
    };
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
    if exact > 9_007_199_254_740_991 || exact < -9_007_199_254_740_991 {
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

fn parse_peer_id(node: Option<&LexNode>) -> Option<PeerId> {
    match node? {
        LexNode::String(value) if !value.is_empty() => Some(PeerId::String(value.clone())),
        LexNode::Number(lexeme) => {
            let canonical = parse_safe_integer(Some(&LexNode::Number(lexeme.clone())))?;
            Some(PeerId::SafeInteger(canonical))
        }
        _ => None,
    }
}

fn peer_key(id: &PeerId) -> String {
    match id {
        PeerId::String(value) => format!("s:{value}"),
        PeerId::SafeInteger(value) => format!("n:{value}"),
    }
}

#[test]
fn jsonrpc_numeric_id_lexemes_match_frozen_corpus() {
    let corpus = load_jsonrpc_ids();
    assert_eq!(corpus.schema_version, 1);
    assert_eq!(corpus.corpus, "jsonrpc-peer-id-v1");
    assert!(
        corpus.cases.len() >= 16,
        "numeric id corpus must cover canonicalization and rejection families"
    );
    for case in &corpus.cases {
        let action = classify_lexeme(&case.frame);
        match (case.kind.as_str(), &action) {
            ("request", LexicalAction::Request { id, method }) => {
                assert!(
                    !method.is_empty(),
                    "{}: method must be non-empty",
                    case.name
                );
                let (kind, canonical) = match id {
                    PeerId::String(value) => ("string", value.clone()),
                    PeerId::SafeInteger(value) => ("safeInteger", value.to_string()),
                };
                assert_eq!(
                    Some(kind.to_string()),
                    case.id_kind,
                    "{}: idKind",
                    case.name
                );
                assert_eq!(Some(canonical), case.id, "{}: canonical id", case.name);
                assert_eq!(Some(peer_key(id)), case.peer_key, "{}: peerKey", case.name);
            }
            ("response", LexicalAction::Response { id }) => {
                assert_eq!(Some(id.clone()), case.id, "{}: response id", case.name);
                assert_eq!(
                    case.id_kind.as_deref(),
                    Some("string"),
                    "{}: idKind",
                    case.name
                );
            }
            ("notification", LexicalAction::Notification { method }) => {
                assert!(!method.is_empty(), "{}: method", case.name);
            }
            ("platformError", LexicalAction::PlatformError { error }) => {
                assert_eq!(
                    case.error_kind.as_deref(),
                    Some(error.as_str()),
                    "{}: error kind",
                    case.name
                );
            }
            ("close", LexicalAction::Close { code }) => {
                assert_eq!(Some(*code), case.code, "{}: close code", case.name);
            }
            (kind, other) => panic!("{}: expected kind {kind}, got {other:?}", case.name),
        }
    }
}

// ---------------------------------------------------------------------------
// Composed client lifecycle + pin ledger + broker scenario corpus
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Reserved,
    Admitted,
    Attached,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Terminal {
    None,
    PeerClose,
    RuntimeDisconnect,
    Shutdown,
    SlowClient,
    Replacement,
    PolicyRejected,
    ProtocolClose,
    ReleaseTimeout,
}

impl Terminal {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::PeerClose => "PeerClose",
            Self::RuntimeDisconnect => "RuntimeDisconnect",
            Self::Shutdown => "Shutdown",
            Self::SlowClient => "SlowClient",
            Self::Replacement => "Replacement",
            Self::PolicyRejected => "PolicyRejected",
            Self::ProtocolClose => "ProtocolClose",
            Self::ReleaseTimeout => "ReleaseTimeout",
        }
    }
}

#[derive(Debug, Clone)]
struct Conn {
    id: String,
    phase: Phase,
    generation: Option<String>,
    runtime: Option<String>,
    terminal: Terminal,
    pin_acquired: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct LimitsValue {
    #[serde(rename = "connectionLimit", default = "default_connection_limit")]
    connection_limit: usize,
    #[serde(
        rename = "slowClientBudgetBytes",
        default = "default_slow_client_budget"
    )]
    slow_client_budget_bytes: u64,
    #[serde(
        rename = "perGenerationCapacity",
        default = "default_per_generation_capacity"
    )]
    per_generation_capacity: usize,
}

fn default_connection_limit() -> usize {
    5000
}

fn default_slow_client_budget() -> u64 {
    16 * 1024 * 1024
}

fn default_per_generation_capacity() -> usize {
    128
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum EventValue {
    AcceptConnection {
        connection: String,
        #[serde(rename = "businessKey")]
        business_key: Option<String>,
        rank: Option<u64>,
        #[serde(rename = "maxConnections", default = "default_max_connections")]
        max_connections: usize,
        #[serde(default = "default_overflow")]
        overflow: String,
    },
    Attach {
        connection: String,
        #[serde(rename = "socketGeneration")]
        socket_generation: String,
        runtime: String,
    },
    AcquirePin {
        connection: String,
        runtime: String,
    },
    ReleasePin {
        connection: String,
        mode: String,
    },
    PeerClose {
        connection: String,
    },
    RuntimeDisconnect {
        runtime: String,
    },
    Shutdown,
    SlowClient {
        connection: String,
        bytes: u64,
    },
    CapturedWrite {
        connection: String,
        #[serde(rename = "socketGeneration")]
        socket_generation: String,
        bytes: u64,
    },
    RuntimeRequest {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
        #[serde(rename = "deadlineMs")]
        deadline_ms: Option<u64>,
    },
    Deadline {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    PeerResponse {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    PeerRequest {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
    InboundDispatch {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
        result: String,
    },
    RuntimeCancel {
        connection: String,
        #[serde(rename = "requestId")]
        request_id: String,
    },
    LateResponse {
        connection: String,
        #[serde(rename = "peerId")]
        peer_id: String,
    },
}

fn default_max_connections() -> usize {
    1
}

fn default_overflow() -> String {
    "close-oldest".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExpectValue {
    terminals: HashMap<String, String>,
    #[serde(rename = "connectionCount", default)]
    connection_count: usize,
    #[serde(rename = "generationCount", default)]
    generation_count: usize,
    #[serde(rename = "outboundPending", default)]
    outbound_pending: usize,
    #[serde(rename = "inboundPending", default)]
    inbound_pending: usize,
    #[serde(default)]
    tombstones: usize,
    #[serde(rename = "pinsAcquired", default)]
    pins_acquired: usize,
    #[serde(rename = "pinsPendingRelease", default)]
    pins_pending_release: usize,
    #[serde(rename = "releaseAcks", default)]
    release_acks: u64,
    #[serde(rename = "finalizerCount", default)]
    finalizer_count: u64,
    #[serde(rename = "runtimeClosed", default)]
    runtime_closed: bool,
    #[serde(rename = "failStop", default)]
    fail_stop: bool,
    #[serde(rename = "openConnections", default)]
    open_connections: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ScenarioFile {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    limits: LimitsValue,
    events: Vec<EventValue>,
    expect: ExpectValue,
}

struct Machine {
    connections: Vec<Conn>,
    business_order: HashMap<String, Vec<String>>,
    high_water: HashMap<String, u64>,
    broker: HashMap<String, BrokerGeneration>,
    outbound_by_peer: HashMap<String, String>,
    outbound_by_request: HashMap<String, String>,
    inbound_by_peer: HashMap<String, String>,
    tombstones: HashMap<String, String>,
    outbound_seq: HashMap<String, u64>,
    pins_acquired: usize,
    pending_release: HashSet<String>,
    release_acks: u64,
    finalizer_count: u64,
    runtime_closed: bool,
    shutting_down: bool,
    fail_stop: bool,
    limits: LimitsValue,
}

#[derive(Debug, Clone)]
struct BrokerGeneration {
    open: bool,
    outbound_active: usize,
    inbound_active: usize,
}

impl Machine {
    fn new(limits: LimitsValue) -> Self {
        Self {
            connections: Vec::new(),
            business_order: HashMap::new(),
            high_water: HashMap::new(),
            broker: HashMap::new(),
            outbound_by_peer: HashMap::new(),
            outbound_by_request: HashMap::new(),
            inbound_by_peer: HashMap::new(),
            tombstones: HashMap::new(),
            outbound_seq: HashMap::new(),
            pins_acquired: 0,
            pending_release: HashSet::new(),
            release_acks: 0,
            finalizer_count: 0,
            runtime_closed: false,
            shutting_down: false,
            fail_stop: false,
            limits,
        }
    }

    fn conn_mut(&mut self, id: &str) -> &mut Conn {
        self.connections
            .iter_mut()
            .find(|conn| conn.id == id)
            .unwrap_or_else(|| panic!("unknown connection {id}"))
    }

    fn conn(&self, id: &str) -> &Conn {
        self.connections
            .iter()
            .find(|conn| conn.id == id)
            .unwrap_or_else(|| panic!("unknown connection {id}"))
    }

    fn existing_for(&self, business_key: &str) -> Vec<String> {
        self.business_order
            .get(business_key)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|id| self.conn(id).phase != Phase::Closed)
            .collect()
    }

    fn accept_connection(
        &mut self,
        connection: &str,
        business_key: Option<&str>,
        rank: Option<u64>,
        max_connections: usize,
        overflow: &str,
    ) {
        if self.shutting_down {
            self.fail_stop = true;
            return;
        }
        if self.connections.len() >= self.limits.connection_limit {
            self.fail_stop = true;
            return;
        }
        let mut conn = Conn {
            id: connection.to_string(),
            phase: Phase::Reserved,
            generation: None,
            runtime: None,
            terminal: Terminal::None,
            pin_acquired: false,
        };
        if let Some(key) = business_key {
            let existing = self.existing_for(key);
            let high_water = self.high_water.get(key).copied();
            if high_water.is_some() && rank.is_none_or(|rank| rank <= high_water.unwrap()) {
                self.finish_new(&mut conn, Terminal::PolicyRejected);
            } else if rank.is_some() {
                self.high_water.insert(key.to_string(), rank.unwrap());
                for id in existing {
                    self.finish_connection(&id, Terminal::Replacement);
                }
            } else if overflow == "reject-new" && existing.len() >= max_connections {
                self.finish_new(&mut conn, Terminal::PolicyRejected);
            } else if overflow == "close-oldest" && existing.len() + 1 > max_connections {
                let overflow_count = existing.len() + 1 - max_connections;
                for id in existing.into_iter().take(overflow_count) {
                    self.finish_connection(&id, Terminal::Replacement);
                }
            }
        }
        if conn.phase == Phase::Reserved {
            conn.phase = Phase::Admitted;
            if let Some(key) = business_key {
                self.business_order
                    .entry(key.to_string())
                    .or_default()
                    .push(connection.to_string());
            }
        }
        self.connections.push(conn);
    }

    fn finish_new(&mut self, conn: &mut Conn, terminal: Terminal) {
        conn.phase = Phase::Closed;
        conn.terminal = terminal;
        self.finalizer_count += 1;
    }

    fn finish_connection(&mut self, connection: &str, terminal: Terminal) {
        let conn = self.conn_mut(connection);
        if conn.phase == Phase::Closed {
            return;
        }
        conn.phase = Phase::Closed;
        conn.terminal = terminal;
        let generation = conn.generation.clone();
        let pin_acquired = conn.pin_acquired;
        if pin_acquired {
            conn.pin_acquired = false;
        }
        let _ = conn;
        if pin_acquired {
            self.pins_acquired -= 1;
            if terminal != Terminal::RuntimeDisconnect {
                self.pending_release.insert(connection.to_string());
            }
        }
        if let Some(generation) = generation {
            self.close_generation(connection, &generation);
        }
        self.finalizer_count += 1;
        if terminal == Terminal::ReleaseTimeout {
            self.runtime_closed = true;
        }
    }

    fn close_generation(&mut self, connection: &str, generation: &str) {
        let _ = generation;
        if let Some(broker) = self.broker.get_mut(connection) {
            broker.open = false;
            broker.outbound_active = 0;
            broker.inbound_active = 0;
        }
        self.outbound_by_peer.retain(|_, owner| owner != connection);
        self.outbound_by_request
            .retain(|_, owner| owner != connection);
        self.inbound_by_peer.retain(|_, owner| owner != connection);
        self.tombstones.retain(|_, owner| owner != connection);
    }

    fn attach(&mut self, connection: &str, socket_generation: &str, runtime: &str) {
        let conn = self.conn_mut(connection);
        assert_eq!(conn.phase, Phase::Admitted, "attach requires admitted");
        conn.generation = Some(socket_generation.to_string());
        conn.runtime = Some(runtime.to_string());
        conn.phase = Phase::Attached;
        self.broker.insert(
            connection.to_string(),
            BrokerGeneration {
                open: true,
                outbound_active: 0,
                inbound_active: 0,
            },
        );
        self.outbound_seq.insert(connection.to_string(), 0);
    }

    fn acquire_pin(&mut self, connection: &str, runtime: &str) {
        let conn = self.conn_mut(connection);
        assert_eq!(conn.phase, Phase::Attached, "acquire requires attached");
        assert_eq!(
            conn.runtime.as_deref(),
            Some(runtime),
            "pin runtime must match"
        );
        if !conn.pin_acquired {
            conn.pin_acquired = true;
            self.pins_acquired += 1;
        }
    }

    fn release_pin(&mut self, connection: &str, mode: &str) {
        match mode {
            "initiate" => {
                let pin_acquired = {
                    let conn = self.conn(connection);
                    conn.pin_acquired && !self.pending_release.contains(connection)
                };
                if pin_acquired {
                    let conn = self.conn_mut(connection);
                    conn.pin_acquired = false;
                    self.pins_acquired -= 1;
                    self.pending_release.insert(connection.to_string());
                }
            }
            "ack" => {
                assert!(
                    self.pending_release.remove(connection),
                    "release ack requires pending for {connection}"
                );
                self.release_acks += 1;
            }
            "timeout" => {
                assert!(
                    self.pending_release.remove(connection),
                    "release timeout requires pending for {connection}"
                );
                self.runtime_closed = true;
                let conn = self.conn_mut(connection);
                if conn.terminal == Terminal::None {
                    conn.terminal = Terminal::ReleaseTimeout;
                }
            }
            other => panic!("unknown release mode {other}"),
        }
    }

    fn peer_close(&mut self, connection: &str) {
        if self.conn(connection).phase == Phase::Attached {
            self.finish_connection(connection, Terminal::PeerClose);
        }
    }

    fn runtime_disconnect(&mut self, runtime: &str) {
        let affected = self
            .connections
            .iter()
            .filter(|conn| {
                conn.runtime.as_deref() == Some(runtime) && conn.phase == Phase::Attached
            })
            .map(|conn| conn.id.clone())
            .collect::<Vec<_>>();
        for id in &affected {
            self.finish_connection(id, Terminal::RuntimeDisconnect);
        }
        // Runtime disconnect resolves every pending release for that runtime
        // without an ACK; the exact Runtime session is gone. This includes
        // releases already pending for a closed connection whose pin was
        // held by the disconnected runtime.
        let pending = self
            .pending_release
            .iter()
            .filter(|id| self.conn(id).runtime.as_deref() == Some(runtime))
            .cloned()
            .collect::<Vec<_>>();
        for id in pending {
            self.pending_release.remove(&id);
        }
    }

    fn shutdown(&mut self) {
        self.shutting_down = true;
        let affected = self
            .connections
            .iter()
            .filter(|conn| conn.phase != Phase::Closed)
            .map(|conn| conn.id.clone())
            .collect::<Vec<_>>();
        for id in affected {
            self.finish_connection(&id, Terminal::Shutdown);
        }
    }

    fn slow_client(&mut self, connection: &str, bytes: u64) {
        let conn = self.conn_mut(connection);
        assert_eq!(conn.phase, Phase::Attached, "slow client requires attached");
        let _ = conn;
        if bytes > self.limits.slow_client_budget_bytes {
            self.finish_connection(connection, Terminal::SlowClient);
        }
    }

    fn captured_write(&mut self, connection: &str, socket_generation: &str, bytes: u64) {
        let conn = self.conn(connection);
        if conn.phase == Phase::Closed || conn.generation.as_deref() != Some(socket_generation) {
            // Captured writer fence: a stale generation cannot write the
            // replacement socket.
            return;
        }
        let _ = conn;
        if bytes > self.limits.slow_client_budget_bytes {
            self.finish_connection(connection, Terminal::SlowClient);
        }
    }

    fn runtime_request(&mut self, connection: &str, request_id: &str, deadline_ms: Option<u64>) {
        let conn = self.conn(connection);
        assert_eq!(
            conn.phase,
            Phase::Attached,
            "runtime request requires attached"
        );
        let generation = conn.generation.clone().expect("generation");
        let _ = conn;
        let broker = self.broker.get(connection).expect("broker generation");
        if !broker.open || broker.outbound_active >= self.limits.per_generation_capacity {
            // resourceLimit: no peer write, no tombstone.
            return;
        }
        let seq = self.outbound_seq.get_mut(connection).expect("outbound seq");
        let peer_key = format!("{generation}:{seq}");
        *seq += 1;
        self.outbound_by_peer
            .insert(peer_key.clone(), connection.to_string());
        self.outbound_by_request
            .insert(request_key(connection, request_id), peer_key);
        self.broker
            .get_mut(connection)
            .expect("broker generation")
            .outbound_active += 1;
        let _ = deadline_ms;
    }

    fn deadline(&mut self, connection: &str, request_id: &str) {
        let key = request_key(connection, request_id);
        if let Some(peer_key) = self.outbound_by_request.remove(&key) {
            self.settle_outbound(connection, &peer_key);
        }
    }

    fn peer_response(&mut self, connection: &str, peer_id: &str) {
        if let Some(owner) = self.outbound_by_peer.get(peer_id) {
            if owner == connection {
                self.settle_outbound(connection, peer_id);
                return;
            }
        }
        if self.tombstones.get(peer_id).map(String::as_str) == Some(connection) {
            // Late response isolated by the tombstone fence.
            return;
        }
        self.finish_connection(connection, Terminal::ProtocolClose);
    }

    fn peer_request(&mut self, connection: &str, peer_id: &str) {
        if self.inbound_by_peer.contains_key(peer_id)
            || self.tombstones.get(peer_id).map(String::as_str) == Some(connection)
        {
            self.finish_connection(connection, Terminal::ProtocolClose);
            return;
        }
        self.inbound_by_peer
            .insert(peer_id.to_string(), connection.to_string());
        self.broker
            .get_mut(connection)
            .expect("broker generation")
            .inbound_active += 1;
    }

    fn inbound_dispatch(&mut self, connection: &str, peer_id: &str, result: &str) {
        if self.inbound_by_peer.remove(peer_id).as_deref() != Some(connection) {
            panic!("inbound dispatch for unknown peer id {peer_id}");
        }
        self.broker
            .get_mut(connection)
            .expect("broker generation")
            .inbound_active -= 1;
        self.tombstones
            .insert(peer_id.to_string(), connection.to_string());
        let _ = result;
    }

    fn runtime_cancel(&mut self, connection: &str, request_id: &str) {
        let key = request_key(connection, request_id);
        if let Some(peer_key) = self.outbound_by_request.remove(&key) {
            self.settle_outbound(connection, &peer_key);
        }
    }

    fn late_response(&mut self, connection: &str, peer_id: &str) {
        if self.tombstones.get(peer_id).map(String::as_str) == Some(connection) {
            return;
        }
        if !self.outbound_by_peer.contains_key(peer_id) {
            self.finish_connection(connection, Terminal::ProtocolClose);
        }
    }

    fn settle_outbound(&mut self, connection: &str, peer_key: &str) {
        if self.outbound_by_peer.remove(peer_key).as_deref() != Some(connection) {
            return;
        }
        self.outbound_by_request
            .retain(|_, value| value != peer_key);
        self.broker
            .get_mut(connection)
            .expect("broker generation")
            .outbound_active -= 1;
        self.tombstones
            .insert(peer_key.to_string(), connection.to_string());
    }

    fn run(&mut self, event: &EventValue) {
        match event {
            EventValue::AcceptConnection {
                connection,
                business_key,
                rank,
                max_connections,
                overflow,
            } => self.accept_connection(
                connection,
                business_key.as_deref(),
                *rank,
                *max_connections,
                overflow,
            ),
            EventValue::Attach {
                connection,
                socket_generation,
                runtime,
            } => self.attach(connection, socket_generation, runtime),
            EventValue::AcquirePin {
                connection,
                runtime,
            } => {
                self.acquire_pin(connection, runtime);
            }
            EventValue::ReleasePin { connection, mode } => {
                self.release_pin(connection, mode);
            }
            EventValue::PeerClose { connection } => self.peer_close(connection),
            EventValue::RuntimeDisconnect { runtime } => self.runtime_disconnect(runtime),
            EventValue::Shutdown => self.shutdown(),
            EventValue::SlowClient { connection, bytes } => {
                self.slow_client(connection, *bytes);
            }
            EventValue::CapturedWrite {
                connection,
                socket_generation,
                bytes,
            } => self.captured_write(connection, socket_generation, *bytes),
            EventValue::RuntimeRequest {
                connection,
                request_id,
                deadline_ms,
            } => self.runtime_request(connection, request_id, *deadline_ms),
            EventValue::Deadline {
                connection,
                request_id,
            } => self.deadline(connection, request_id),
            EventValue::PeerResponse {
                connection,
                peer_id,
            } => {
                self.peer_response(connection, peer_id);
            }
            EventValue::PeerRequest {
                connection,
                peer_id,
            } => {
                self.peer_request(connection, peer_id);
            }
            EventValue::InboundDispatch {
                connection,
                peer_id,
                result,
            } => self.inbound_dispatch(connection, peer_id, result),
            EventValue::RuntimeCancel {
                connection,
                request_id,
            } => self.runtime_cancel(connection, request_id),
            EventValue::LateResponse {
                connection,
                peer_id,
            } => {
                self.late_response(connection, peer_id);
            }
        }
    }

    fn outcome(&self, connection: &str) -> String {
        self.conn(connection).terminal.as_str().to_string()
    }
}

fn request_key(connection: &str, request_id: &str) -> String {
    format!("{connection}\0{request_id}")
}

fn load_scenarios() -> Vec<ScenarioFile> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata")
        .join("client-ws")
        .join("scenarios");
    let mut paths = std::fs::read_dir(&dir)
        .expect("scenarios dir must exist")
        .map(|entry| entry.expect("scenario entry"))
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("scenario must be readable");
            serde_json::from_str(&text).expect("scenario must parse")
        })
        .collect()
}

#[test]
fn client_ws_scenarios_match_frozen_semantics() {
    let scenarios = load_scenarios();
    let scenario_names = scenarios
        .iter()
        .map(|scenario| scenario.scenario.as_str())
        .collect::<HashSet<_>>();
    for required in REQUIRED_SCENARIOS {
        assert!(
            scenario_names.contains(required),
            "required scenario {required} missing"
        );
    }
    for scenario in &scenarios {
        assert_eq!(scenario.schema_version, 1);
        let mut machine = Machine::new(scenario.limits.clone());
        for event in &scenario.events {
            machine.run(event);
        }

        let expect = &scenario.expect;
        for (id, expected_terminal) in &expect.terminals {
            assert_eq!(
                machine.outcome(id),
                *expected_terminal,
                "scenario {}: terminal for {id}",
                scenario.scenario
            );
        }
        let open = machine
            .connections
            .iter()
            .filter(|conn| conn.phase != Phase::Closed)
            .map(|conn| conn.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            open, expect.open_connections,
            "scenario {}: openConnections",
            scenario.scenario
        );
        assert_eq!(
            open.len(),
            expect.connection_count,
            "scenario {}: connectionCount",
            scenario.scenario
        );
        let generation_count = machine
            .broker
            .values()
            .filter(|generation| generation.open)
            .count();
        assert_eq!(
            generation_count, expect.generation_count,
            "scenario {}: generationCount",
            scenario.scenario
        );
        let outbound_pending = machine
            .broker
            .values()
            .map(|generation| generation.outbound_active)
            .sum::<usize>();
        let inbound_pending = machine
            .broker
            .values()
            .map(|generation| generation.inbound_active)
            .sum::<usize>();
        assert_eq!(
            outbound_pending, expect.outbound_pending,
            "scenario {}: outboundPending",
            scenario.scenario
        );
        assert_eq!(
            inbound_pending, expect.inbound_pending,
            "scenario {}: inboundPending",
            scenario.scenario
        );
        assert_eq!(
            machine.tombstones.len(),
            expect.tombstones,
            "scenario {}: tombstones",
            scenario.scenario
        );
        assert_eq!(
            machine.pins_acquired, expect.pins_acquired,
            "scenario {}: pinsAcquired",
            scenario.scenario
        );
        assert_eq!(
            machine.pending_release.len(),
            expect.pins_pending_release,
            "scenario {}: pinsPendingRelease",
            scenario.scenario
        );
        assert_eq!(
            machine.release_acks, expect.release_acks,
            "scenario {}: releaseAcks",
            scenario.scenario
        );
        assert_eq!(
            machine.finalizer_count, expect.finalizer_count,
            "scenario {}: finalizerCount",
            scenario.scenario
        );
        assert_eq!(
            machine.runtime_closed, expect.runtime_closed,
            "scenario {}: runtimeClosed",
            scenario.scenario
        );
        assert!(
            !machine.fail_stop && !expect.fail_stop,
            "scenario {}: failStop must be false for the frozen corpus",
            scenario.scenario
        );
        // Terminal lease count invariant (§10): every owned pending either
        // settled or was removed with its generation.
        assert_eq!(
            machine.outbound_by_peer.len(),
            outbound_pending,
            "scenario {}: outbound peer index must equal broker pending",
            scenario.scenario
        );
        assert_eq!(
            machine.inbound_by_peer.len(),
            inbound_pending,
            "scenario {}: inbound peer index must equal broker pending",
            scenario.scenario
        );
    }
}

// ---------------------------------------------------------------------------
// Canonical frame catalog regeneration. The checked-in fixture is the frozen
// artifact; this test proves it can be regenerated byte-for-byte from the
// canonical codec, so a codec drift is a fixture failure, not a silent
// golden update.
// ---------------------------------------------------------------------------

fn generate_frame_catalog_document() -> Value {
    let mut frames = BTreeMap::<String, Value>::new();
    let mut add = |name: &str,
                   direction: &str,
                   decode_as: &str,
                   frame_type: &str,
                   bytes: Vec<u8>,
                   header: Value| {
        frames.insert(
            name.to_string(),
            json!({
                "direction": direction,
                "frameType": frame_type,
                "decodeAs": decode_as,
                "frameHex": hex_encode(&bytes),
                "header": header,
            }),
        );
    };

    let request = request_header(true);
    let payload = br#"{"n":1}"#;
    add(
        "connection.request.object",
        "RuntimeToRouter",
        "ConnectionRequest",
        "connection.request",
        encode_connection_request_frame(&request, payload).unwrap(),
        serde_json::to_value(&request).unwrap(),
    );
    let mut array_request = request_header(true);
    array_request.request_id = "conn-req-2".to_string();
    add(
        "connection.request.array",
        "RuntimeToRouter",
        "ConnectionRequest",
        "connection.request",
        encode_connection_request_frame(&array_request, br#"[1,2]"#).unwrap(),
        serde_json::to_value(&array_request).unwrap(),
    );
    let mut no_deadline = request_header(false);
    no_deadline.request_id = "conn-req-3".to_string();
    add(
        "connection.request.no-deadline",
        "RuntimeToRouter",
        "ConnectionRequest",
        "connection.request",
        encode_connection_request_frame(&no_deadline, br#"{"n":2}"#).unwrap(),
        serde_json::to_value(&no_deadline).unwrap(),
    );
    add(
        "connection.request.cancel",
        "RuntimeToRouter",
        "ConnectionCancel",
        "connection.request.cancel",
        encode_connection_request_cancel_frame(&cancel_header()).unwrap(),
        serde_json::to_value(&cancel_header()).unwrap(),
    );

    add(
        "connection.response.success",
        "RouterToRuntime",
        "ConnectionResponse",
        "connection.response",
        encode_connection_response_frame(
            &response_header(ConnectionResponseOutcome::Success),
            br#"{"ok":true}"#,
        )
        .unwrap(),
        serde_json::to_value(&response_header(ConnectionResponseOutcome::Success)).unwrap(),
    );
    let mut remote_header = response_header(ConnectionResponseOutcome::Remote);
    remote_header.remote = Some(ConnectionRemoteErrorFrameHeader {
        code: -32_009,
        message: "peer failed".to_string(),
        data_present: true,
    });
    add(
        "connection.response.remote",
        "RouterToRuntime",
        "ConnectionResponse",
        "connection.response",
        encode_connection_response_frame(&remote_header, br#"{"x":1}"#).unwrap(),
        serde_json::to_value(&remote_header).unwrap(),
    );
    for (name, outcome) in [
        (
            "connection.response.deadline-exceeded",
            ConnectionResponseOutcome::DeadlineExceeded,
        ),
        (
            "connection.response.connection-unavailable",
            ConnectionResponseOutcome::ConnectionUnavailable,
        ),
        (
            "connection.response.transport-unavailable",
            ConnectionResponseOutcome::TransportUnavailable,
        ),
        (
            "connection.response.protocol-error",
            ConnectionResponseOutcome::ProtocolError,
        ),
        (
            "connection.response.resource-limit",
            ConnectionResponseOutcome::ResourceLimit,
        ),
    ] {
        let header = response_header(outcome);
        add(
            name,
            "RouterToRuntime",
            "ConnectionResponse",
            "connection.response",
            encode_connection_response_frame(&header, &[]).unwrap(),
            serde_json::to_value(&header).unwrap(),
        );
    }

    let acquire = lifecycle_control(
        "acquire",
        WebSocketGenerationLifecycleOperation::Acquire,
        None,
    );
    add(
        "lifecycle.acquire",
        "RuntimeToRouter",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(direction_for("acquire", None), &acquire)
            .unwrap(),
        serde_json::to_value(&acquire).unwrap(),
    );
    let release = lifecycle_control(
        "release",
        WebSocketGenerationLifecycleOperation::Release,
        None,
    );
    add(
        "lifecycle.release",
        "RouterToRuntime",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(direction_for("release", None), &release)
            .unwrap(),
        serde_json::to_value(&release).unwrap(),
    );
    let ack_acquire =
        lifecycle_control("ack", WebSocketGenerationLifecycleOperation::Acquire, None);
    add(
        "lifecycle.ack.acquire",
        "RouterToRuntime",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(
            direction_for("ack", Some(WebSocketGenerationLifecycleOperation::Acquire)),
            &ack_acquire,
        )
        .unwrap(),
        serde_json::to_value(&ack_acquire).unwrap(),
    );
    let ack_release =
        lifecycle_control("ack", WebSocketGenerationLifecycleOperation::Release, None);
    add(
        "lifecycle.ack.release",
        "RuntimeToRouter",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(
            direction_for("ack", Some(WebSocketGenerationLifecycleOperation::Release)),
            &ack_release,
        )
        .unwrap(),
        serde_json::to_value(&ack_release).unwrap(),
    );
    let reject_acquire = lifecycle_control(
        "reject",
        WebSocketGenerationLifecycleOperation::Acquire,
        Some(WebSocketGenerationLifecycleRejectionCode::TupleMismatch),
    );
    add(
        "lifecycle.reject.acquire",
        "RouterToRuntime",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(
            direction_for(
                "reject",
                Some(WebSocketGenerationLifecycleOperation::Acquire),
            ),
            &reject_acquire,
        )
        .unwrap(),
        serde_json::to_value(&reject_acquire).unwrap(),
    );
    let reject_release = lifecycle_control(
        "reject",
        WebSocketGenerationLifecycleOperation::Release,
        Some(WebSocketGenerationLifecycleRejectionCode::NotAcquired),
    );
    add(
        "lifecycle.reject.release",
        "RuntimeToRouter",
        "Lifecycle",
        "websocket.generation.lifecycle",
        encode_websocket_generation_lifecycle_frame(
            direction_for(
                "reject",
                Some(WebSocketGenerationLifecycleOperation::Release),
            ),
            &reject_release,
        )
        .unwrap(),
        serde_json::to_value(&reject_release).unwrap(),
    );

    json!({
        "schemaVersion": 1,
        "corpus": "client-ws-frames-v1",
        "frames": frames,
    })
}

#[test]
fn frame_catalog_regenerates_from_canonical_codec() {
    let loaded: Value = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("testdata")
                .join("client-ws")
                .join("frames.json"),
        )
        .expect("frames.json must exist"),
    )
    .expect("frames.json must parse");
    assert_eq!(
        generate_frame_catalog_document(),
        loaded,
        "frames.json must be byte-exact regenerable from the canonical codec"
    );
}

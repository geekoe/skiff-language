//! M-connection consumer gate: skiff-router consumes the frozen
//! C-model-connection corpus (`runtime/transport/testdata/client-ws/`)
//! directly through the W-model transport codecs and the production
//! JSON-RPC 2.0 text profile classifier.
//!
//! The Router side of the client WebSocket wire (C-model-connection §3/§4)
//! receives `connection.request` / `connection.request.cancel` and sends
//! `connection.response`. The `websocket.generation.lifecycle` family is
//! retired: client ws connections are stateless and the router connection
//! registry is the only accounting authority. Router also owns peer lexical
//! classification (`WebSocketRpcProfile`), so the numeric id corpus is
//! consumed through the production classifier.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::connection_protocol::{
    classify_jsonrpc_20_text_frame, decode_connection_request_cancel_frame,
    decode_connection_request_frame, decode_connection_response_frame,
    encode_connection_request_cancel_frame, encode_connection_request_frame,
    encode_connection_response_frame, JsonRpcPlatformErrorKind, OpaquePeerId, ProfileAction,
};
const REQUIRED_FRAMES: [&str; 11] = [
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
];

#[derive(Debug, Clone, Deserialize)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

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

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/client-ws")
}

fn catalog() -> Catalog {
    let text = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("client-ws frames.json must be readable");
    serde_json::from_str(&text).expect("client-ws frames.json must decode")
}

fn jsonrpc_ids() -> Vec<JsonRpcIdCase> {
    let text = fs::read_to_string(corpus_dir().join("jsonrpc-ids.json"))
        .expect("client-ws jsonrpc-ids.json must be readable");
    let value: Value = serde_json::from_str(&text).expect("client-ws jsonrpc-ids.json must decode");
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["corpus"], "jsonrpc-peer-id-v1");
    serde_json::from_value(value["cases"].clone()).expect("jsonrpc id cases must decode")
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn roundtrip_connection_request(name: &str, bytes: &[u8]) {
    let (header, payload) = decode_connection_request_frame(bytes)
        .unwrap_or_else(|error| panic!("{name} request decode: {error}"));
    let reencoded = encode_connection_request_frame(&header, &payload)
        .unwrap_or_else(|error| panic!("{name} request encode: {error}"));
    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
}

fn roundtrip_connection_cancel(name: &str, bytes: &[u8]) {
    let header = decode_connection_request_cancel_frame(bytes)
        .unwrap_or_else(|error| panic!("{name} cancel decode: {error}"));
    let reencoded = encode_connection_request_cancel_frame(&header)
        .unwrap_or_else(|error| panic!("{name} cancel encode: {error}"));
    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
}

fn roundtrip_connection_response(name: &str, bytes: &[u8]) {
    let (header, payload) = decode_connection_response_frame(bytes)
        .unwrap_or_else(|error| panic!("{name} response decode: {error}"));
    let reencoded = encode_connection_response_frame(&header, &payload)
        .unwrap_or_else(|error| panic!("{name} response encode: {error}"));
    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_consumer_roundtrips_connection_corpus_through_w_model_codecs() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "client-ws-frames-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        for (name, entry) in &catalog.frames {
            let bytes = hex_to_bytes(&entry.frame_hex);
            match entry.decode_as.as_str() {
                "ConnectionRequest" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    roundtrip_connection_request(name, &bytes);
                }
                "ConnectionCancel" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    roundtrip_connection_cancel(name, &bytes);
                }
                "ConnectionResponse" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    roundtrip_connection_response(name, &bytes);
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }
    }

    #[test]
    fn router_consumer_classifies_all_frozen_jsonrpc_peer_ids_with_production_codec() {
        let cases = jsonrpc_ids();
        assert!(cases.len() >= 22, "numeric id corpus must stay exhaustive");
        for case in &cases {
            let action = classify_jsonrpc_20_text_frame(case.frame.as_bytes());
            match (case.kind.as_str(), &action) {
                ("request", ProfileAction::Request { id, method }) => {
                    assert!(!method.is_empty(), "{}: method", case.name);
                    let (id_kind, canonical) = match id {
                        OpaquePeerId::String(value) => ("string", value.clone()),
                        OpaquePeerId::SafeInteger(value) => ("safeInteger", value.to_string()),
                    };
                    assert_eq!(
                        Some(id_kind.to_string()),
                        case.id_kind,
                        "{}: idKind",
                        case.name
                    );
                    assert_eq!(Some(canonical), case.id, "{}: canonical id", case.name);
                    assert_eq!(
                        Some(id.canonical_key()),
                        case.peer_key,
                        "{}: peerKey",
                        case.name
                    );
                }
                ("response", ProfileAction::Response { id }) => {
                    assert_eq!(Some(id.clone()), case.id, "{}: response id", case.name);
                    assert_eq!(
                        case.id_kind.as_deref(),
                        Some("string"),
                        "{}: idKind",
                        case.name
                    );
                }
                ("notification", ProfileAction::Notification { method }) => {
                    assert!(!method.is_empty(), "{}: method", case.name);
                }
                ("platformError", ProfileAction::PlatformError { kind }) => {
                    let expected = match kind {
                        JsonRpcPlatformErrorKind::Parse => "parse",
                        JsonRpcPlatformErrorKind::InvalidRequest => "invalidRequest",
                        JsonRpcPlatformErrorKind::InvalidParams => "invalidParams",
                    };
                    assert_eq!(
                        case.error_kind.as_deref(),
                        Some(expected),
                        "{}: errorKind",
                        case.name
                    );
                }
                ("close", ProfileAction::Close { code }) => {
                    assert_eq!(Some(*code), case.code, "{}: close code", case.name);
                }
                (kind, other) => panic!("{}: expected kind {kind}, got {other:?}", case.name),
            }
        }
    }
}

//! M-connection consumer gate: the `runtime` crate consumes the frozen
//! C-model-connection corpus (`testdata/client-ws/`) directly through the
//! W-model transport codecs.
//!
//! The Runtime side of the client WebSocket wire (C-model-connection §3/§4)
//! sends `connection.request` / `connection.request.cancel` and receives
//! `connection.response`; for `websocket.generation.lifecycle` it sends
//! `Acquire` and receives Ack/Reject for acquire, receives `Release` and sends
//! Ack/Reject for release. Every frozen frame must roundtrip byte-exact and
//! lifecycle responses must exact-echo their request.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::connection_protocol::{
    decode_connection_request_cancel_frame, decode_connection_request_frame,
    decode_connection_response_frame, encode_connection_request_cancel_frame,
    encode_connection_request_frame, encode_connection_response_frame,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    assert_websocket_generation_lifecycle_response_matches,
    decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
    WebSocketGenerationLifecycleOperation,
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

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transport/testdata/client-ws")
}

fn catalog() -> Catalog {
    let text = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("client-ws frames.json must be readable");
    serde_json::from_str(&text).expect("client-ws frames.json must decode")
}

fn jsonrpc_ids() -> Value {
    let text = fs::read_to_string(corpus_dir().join("jsonrpc-ids.json"))
        .expect("client-ws jsonrpc-ids.json must be readable");
    serde_json::from_str(&text).expect("client-ws jsonrpc-ids.json must decode")
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

fn lifecycle_direction(value: &str) -> WebSocketGenerationLifecycleDirection {
    match value {
        "RouterToRuntime" => WebSocketGenerationLifecycleDirection::RouterToRuntime,
        "RuntimeToRouter" => WebSocketGenerationLifecycleDirection::RuntimeToRouter,
        other => panic!("unknown lifecycle direction {other}"),
    }
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

fn roundtrip_lifecycle(
    name: &str,
    direction: WebSocketGenerationLifecycleDirection,
    bytes: &[u8],
) -> WebSocketGenerationLifecycleControl {
    let control = decode_websocket_generation_lifecycle_frame(direction, bytes)
        .unwrap_or_else(|error| panic!("{name} lifecycle decode: {error}"));
    let reencoded = encode_websocket_generation_lifecycle_frame(direction, &control)
        .unwrap_or_else(|error| panic!("{name} lifecycle encode: {error}"));
    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
    control
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_consumer_roundtrips_connection_corpus_through_w_model_codecs() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "client-ws-frames-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        let mut acquire = None;
        let mut release = None;
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
                "Lifecycle" => {
                    let direction = lifecycle_direction(&entry.direction);
                    let control = roundtrip_lifecycle(name, direction, &bytes);
                    match &control {
                        WebSocketGenerationLifecycleControl::Acquire { .. } => {
                            assert_eq!(entry.direction, "RuntimeToRouter");
                            acquire = Some(control);
                        }
                        WebSocketGenerationLifecycleControl::Release { .. } => {
                            assert_eq!(entry.direction, "RouterToRuntime");
                            release = Some(control);
                        }
                        WebSocketGenerationLifecycleControl::Ack { operation, .. } => {
                            assert_eq!(
                                entry.direction,
                                match operation {
                                    WebSocketGenerationLifecycleOperation::Acquire => {
                                        "RouterToRuntime"
                                    }
                                    WebSocketGenerationLifecycleOperation::Release => {
                                        "RuntimeToRouter"
                                    }
                                }
                            );
                        }
                        WebSocketGenerationLifecycleControl::Reject { operation, .. } => {
                            assert_eq!(
                                entry.direction,
                                match operation {
                                    WebSocketGenerationLifecycleOperation::Acquire => {
                                        "RouterToRuntime"
                                    }
                                    WebSocketGenerationLifecycleOperation::Release => {
                                        "RuntimeToRouter"
                                    }
                                }
                            );
                        }
                    }
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }

        // Lifecycle responses exact-echo the frozen request with the same
        // operation/requestId/tuple.
        for (request, response) in [
            (
                acquire.as_ref().expect("acquire frame"),
                catalog.frames["lifecycle.ack.acquire"].clone(),
            ),
            (
                acquire.as_ref().expect("acquire frame"),
                catalog.frames["lifecycle.reject.acquire"].clone(),
            ),
            (
                release.as_ref().expect("release frame"),
                catalog.frames["lifecycle.ack.release"].clone(),
            ),
            (
                release.as_ref().expect("release frame"),
                catalog.frames["lifecycle.reject.release"].clone(),
            ),
        ] {
            let response_bytes = hex_to_bytes(&response.frame_hex);
            let response_control = decode_websocket_generation_lifecycle_frame(
                lifecycle_direction(&response.direction),
                &response_bytes,
            )
            .expect("lifecycle response decodes");
            assert!(
                assert_websocket_generation_lifecycle_response_matches(request, &response_control)
                    .is_ok(),
                "lifecycle response must exact-echo its request"
            );
        }
    }

    #[test]
    fn runtime_consumer_sees_all_frozen_jsonrpc_peer_id_cases() {
        let corpus = jsonrpc_ids();
        assert_eq!(corpus["schemaVersion"], 1);
        assert_eq!(corpus["corpus"], "jsonrpc-peer-id-v1");
        let cases = corpus["cases"].as_array().expect("cases");
        assert!(cases.len() >= 22, "numeric id corpus must stay exhaustive");
        let mut kinds = Vec::new();
        for case in cases {
            kinds.push(case["kind"].as_str().expect("case kind").to_string());
        }
        for kind in [
            "request",
            "notification",
            "response",
            "platformError",
            "close",
        ] {
            assert!(
                kinds.iter().any(|value| value == kind),
                "corpus must contain kind {kind}"
            );
        }
    }
}

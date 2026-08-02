//! W-model-connection corpus gate.
//!
//! Consumes the frozen C-model-connection corpus
//! (`testdata/client-ws/`) through the production W-model frame codecs and the
//! production JSON-RPC 2.0 text profile classifier, and proves:
//! `encode(decode(frameHex)) == frameHex` for every frozen frame, the frozen
//! direction/payload rules, lifecycle response exact-echo, and the numeric id
//! lexeme validation/canonicalization corpus. The corpus is owned by the
//! contracts-ws pack; this file adds the production-codec consumer gate
//! required by W-model-connection / M-connection (plan §5.3).

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
    WebSocketRpcProfile,
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

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/client-ws")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("client-ws frames.json must be readable");
    serde_json::from_str(&value).expect("client-ws frames.json must decode")
}

fn jsonrpc_ids() -> JsonRpcIdCorpus {
    let value = fs::read_to_string(corpus_dir().join("jsonrpc-ids.json"))
        .expect("client-ws jsonrpc-ids.json must be readable");
    serde_json::from_str(&value).expect("client-ws jsonrpc-ids.json must decode")
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

fn lifecycle_parts(
    control: &WebSocketGenerationLifecycleControl,
) -> (&'static str, Option<WebSocketGenerationLifecycleOperation>) {
    match control {
        WebSocketGenerationLifecycleControl::Acquire { .. } => ("acquire", None),
        WebSocketGenerationLifecycleControl::Release { .. } => ("release", None),
        WebSocketGenerationLifecycleControl::Ack { operation, .. } => ("ack", Some(*operation)),
        WebSocketGenerationLifecycleControl::Reject { operation, .. } => {
            ("reject", Some(*operation))
        }
    }
}

fn expected_lifecycle_direction(
    action: &str,
    operation: Option<WebSocketGenerationLifecycleOperation>,
) -> &'static str {
    match action {
        "acquire" => "RuntimeToRouter",
        "release" => "RouterToRuntime",
        "ack" | "reject" => match operation.expect("operation required") {
            WebSocketGenerationLifecycleOperation::Acquire => "RouterToRuntime",
            WebSocketGenerationLifecycleOperation::Release => "RuntimeToRouter",
        },
        other => panic!("unknown lifecycle action {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_and_lifecycle_frames_roundtrip_byte_exact_through_w_model_codecs() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "client-ws-frames-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        let mut lifecycle_requests = Vec::new();
        let mut lifecycle_responses = Vec::new();
        for (name, entry) in &catalog.frames {
            let bytes = hex_to_bytes(&entry.frame_hex);
            match entry.decode_as.as_str() {
                "ConnectionRequest" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, "connection.request");
                    let (header, payload) = decode_connection_request_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} request decode: {error}"));
                    assert_eq!(
                        header.profile,
                        WebSocketRpcProfile::JsonRpc2_0Text,
                        "{name} profile"
                    );
                    assert_eq!(
                        serde_json::to_value(&header).expect("header serializes"),
                        entry.header,
                        "{name} header JSON must match fixture"
                    );
                    let reencoded = encode_connection_request_frame(&header, &payload)
                        .unwrap_or_else(|error| panic!("{name} request encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "ConnectionCancel" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, "connection.request.cancel");
                    let header = decode_connection_request_cancel_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} cancel decode: {error}"));
                    assert_eq!(
                        serde_json::to_value(&header).expect("header serializes"),
                        entry.header,
                        "{name} header JSON must match fixture"
                    );
                    let reencoded = encode_connection_request_cancel_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} cancel encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "ConnectionResponse" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, "connection.response");
                    let (header, payload) = decode_connection_response_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} response decode: {error}"));
                    assert_eq!(
                        serde_json::to_value(&header).expect("header serializes"),
                        entry.header,
                        "{name} header JSON must match fixture"
                    );
                    let reencoded = encode_connection_response_frame(&header, &payload)
                        .unwrap_or_else(|error| panic!("{name} response encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "Lifecycle" => {
                    assert_eq!(entry.frame_type, "websocket.generation.lifecycle");
                    let control = decode_websocket_generation_lifecycle_frame(
                        lifecycle_direction(&entry.direction),
                        &bytes,
                    )
                    .unwrap_or_else(|error| panic!("{name} lifecycle decode: {error}"));
                    let (action, operation) = lifecycle_parts(&control);
                    assert_eq!(
                        entry.direction,
                        expected_lifecycle_direction(action, operation),
                        "{name} direction"
                    );
                    assert_eq!(
                        serde_json::to_value(&control).expect("control serializes"),
                        entry.header,
                        "{name} control JSON must match fixture"
                    );
                    let reencoded = encode_websocket_generation_lifecycle_frame(
                        lifecycle_direction(&entry.direction),
                        &control,
                    )
                    .unwrap_or_else(|error| panic!("{name} lifecycle encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                    match action {
                        "acquire" | "release" => lifecycle_requests.push(control),
                        "ack" | "reject" => lifecycle_responses.push(control),
                        other => panic!("{name} unknown lifecycle action {other}"),
                    }
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }
        assert_eq!(lifecycle_requests.len(), 2);
        assert_eq!(lifecycle_responses.len(), 4);
        for response in &lifecycle_responses {
            let matched = lifecycle_requests
                .iter()
                .filter(|request| {
                    assert_websocket_generation_lifecycle_response_matches(request, response)
                        .is_ok()
                })
                .count();
            assert_eq!(
                matched, 1,
                "lifecycle response must exact-echo exactly one frozen request"
            );
        }
    }

    #[test]
    fn jsonrpc_ids_are_classified_by_production_profile_codec() {
        let corpus = jsonrpc_ids();
        assert_eq!(corpus.schema_version, 1);
        assert_eq!(corpus.corpus, "jsonrpc-peer-id-v1");
        assert!(corpus.cases.len() >= 22);

        for case in &corpus.cases {
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

    #[test]
    fn client_ws_scenarios_are_complete_and_reference_only_known_frames() {
        let catalog = catalog();
        let scenario_dir = corpus_dir().join("scenarios");
        let mut found = Vec::new();
        for entry in fs::read_dir(&scenario_dir).expect("scenarios dir must be readable") {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            assert_eq!(value["schemaVersion"], 1, "{}", path.display());
            let name = value["scenario"].as_str().expect("scenario name");
            found.push(name.to_string());
            for event in value["events"].as_array().expect("events") {
                if let Some(frame) = event["frame"].as_str() {
                    assert!(
                        catalog.frames.contains_key(frame),
                        "{name} references unknown frame {frame}"
                    );
                }
            }
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                found.iter().any(|name| name == required),
                "corpus must contain required scenario {required}"
            );
        }
        assert_eq!(found.len(), REQUIRED_SCENARIOS.len());
    }
}

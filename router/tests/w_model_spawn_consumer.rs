//! M-spawn consumer gate: skiff-router consumes the frozen C-model-spawn
//! corpus (`runtime/transport/testdata/spawn-wire/`) through the canonical
//! spawn codec (`callerKind` closed enum). The legacy old shape is rejected
//! with no compatible reader; `frameHex` values are byte-exact golden bytes.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_error_frame, decode_spawn_submit_request_frame,
    decode_spawn_submit_response_frame, encode_binary_frame, encode_spawn_submit_error_frame,
    encode_spawn_submit_request_frame, encode_spawn_submit_response_frame,
    SpawnSubmitRequestFrameHeaderV2,
};

const REQUIRED_FRAMES: [&str; 5] = [
    "spawn.submit.request.function",
    "spawn.submit.request.actorMethod",
    "spawn.submit.request.legacy-no-caller-kind",
    "spawn.submit.response",
    "spawn.submit.error.parentNotFound",
];

const REQUIRED_SCENARIOS: [&str; 10] = [
    "resolve-function-parent-exact",
    "resolve-actor-invocation-parent-exact",
    "same-request-id-both-namespaces-no-collision",
    "missing-caller-kind-legacy-cut-rejected",
    "parent-terminal-before-submit-rejected",
    "parent-replaced-before-submit-rejected",
    "parent-connection-mismatch-rejected",
    "authority-mismatch-rejected",
    "accepted-spawn-outlives-parent-terminal",
    "target-kind-mismatch-rejected",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    #[serde(rename = "legacyCut")]
    legacy_cut: bool,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/spawn-wire")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("spawn-wire frames.json must be readable");
    serde_json::from_str(&value).expect("spawn-wire frames.json must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn frame_payload(bytes: &[u8]) -> Vec<u8> {
    let header_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let payload_len = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    bytes[14 + header_len..14 + header_len + payload_len].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_consumer_roundtrips_spawn_wire_corpus_through_canonical_codec() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "spawn-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "corpus must contain required frame {required}"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        assert!(
            catalog.frames["spawn.submit.request.legacy-no-caller-kind"].legacy_cut,
            "legacy old-shape frame must be legacyCut"
        );
        for (name, entry) in &catalog.frames {
            assert_eq!(entry.direction, "RouterToRuntime", "{name}: direction");
            assert_eq!(
                entry.frame_type,
                expected_frame_type(name),
                "{name}: frameType"
            );
            assert_eq!(
                entry.decode_as,
                expected_decode_as(name),
                "{name}: decodeAs"
            );
            assert_eq!(
                entry.payload_presence,
                expected_payload_presence(name),
                "{name}: payloadPresence"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v3",
                "{name}: header schemaVersion"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
        }

        for name in [
            "spawn.submit.request.function",
            "spawn.submit.request.actorMethod",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_spawn_submit_request_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                encode_spawn_submit_request_frame(&header, &payload).expect("re-encode"),
                bytes,
                "{name} must roundtrip byte-exact"
            );
            assert!(
                !frame_payload(&bytes).is_empty(),
                "{name}: spawn.submit.request payload must be present"
            );
            let fixture_header: SpawnSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert!(!entry.legacy_cut, "{name} must not be legacy cut");
        }

        let entry = &catalog.frames["spawn.submit.response"];
        let header = decode_spawn_submit_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("spawn.submit.response must decode");
        assert_eq!(
            encode_spawn_submit_response_frame(&header).expect("response re-encode"),
            hex_bytes(&entry.frame_hex),
            "spawn.submit.response must be byte-exact"
        );
        assert_eq!(header.status, "submitted");

        let entry = &catalog.frames["spawn.submit.error.parentNotFound"];
        let header = decode_spawn_submit_error_frame(&hex_bytes(&entry.frame_hex))
            .expect("spawn.submit.error must decode");
        assert_eq!(
            encode_spawn_submit_error_frame(&header).expect("error re-encode"),
            hex_bytes(&entry.frame_hex),
            "spawn.submit.error must be byte-exact"
        );
        assert_eq!(header.error.code, "ParentNotFound");
    }

    #[test]
    fn router_consumer_rejects_legacy_old_shape_with_no_compatible_reader() {
        let catalog = catalog();
        let entry = &catalog.frames["spawn.submit.request.legacy-no-caller-kind"];
        assert!(entry.legacy_cut);
        let error = decode_spawn_submit_request_frame(&hex_bytes(&entry.frame_hex))
            .expect_err("legacy old-shape frame must be rejected");
        assert!(
            error.to_string().contains("callerKind"),
            "legacy rejection must name callerKind, got {error}"
        );

        let invalid_kind = serde_json::json!({
            "schemaVersion": "skiff-runtime-frame-v3",
            "type": "spawn.submit.request",
            "rpcId": "rpc:probe-1",
            "runtimeId": "runtime-a",
            "callerKind": "function",
            "callerRequestId": "parent-1",
            "targetKind": "function",
            "serviceId": "example.com/docs",
            "serviceVersion": "1.0.0",
            "serviceProtocolIdentity": "example.com/docs:1.0.0",
            "target": "example.com/fn",
            "activationIdentity": {
                "assemblyIdentity": format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "a".repeat(64)
                ),
                "generation": 42,
                "runtimeReplicaId": "runtime-a",
                "deploymentRevision": "rev-1"
            }
        });
        let bytes = encode_binary_frame(&invalid_kind, b"").expect("probe frame must encode");
        assert!(
            decode_spawn_submit_request_frame(&bytes).is_err(),
            "callerKind=function must be rejected by the closed enum"
        );
    }

    #[test]
    fn router_consumer_sees_all_frozen_spawn_scenarios() {
        let mut names = Vec::new();
        for entry in fs::read_dir(corpus_dir().join("scenarios"))
            .expect("spawn scenarios dir must be readable")
        {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            names.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "required spawn scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }

    fn expected_frame_type(name: &str) -> &str {
        match name {
            "spawn.submit.request.function"
            | "spawn.submit.request.actorMethod"
            | "spawn.submit.request.legacy-no-caller-kind" => "spawn.submit.request",
            "spawn.submit.response" => "spawn.submit.response",
            "spawn.submit.error.parentNotFound" => "spawn.submit.error",
            _ => panic!("unexpected spawn frame {name}"),
        }
    }

    fn expected_decode_as(name: &str) -> &'static str {
        match name {
            "spawn.submit.request.function"
            | "spawn.submit.request.actorMethod"
            | "spawn.submit.request.legacy-no-caller-kind" => "SpawnSubmitRequest",
            "spawn.submit.response" => "SpawnSubmitResponse",
            "spawn.submit.error.parentNotFound" => "SpawnSubmitError",
            _ => panic!("unexpected spawn frame {name}"),
        }
    }

    fn expected_payload_presence(name: &str) -> &'static str {
        match name {
            "spawn.submit.request.function"
            | "spawn.submit.request.actorMethod"
            | "spawn.submit.request.legacy-no-caller-kind" => "required",
            "spawn.submit.response" | "spawn.submit.error.parentNotFound" => "empty",
            _ => panic!("unexpected spawn frame {name}"),
        }
    }
}

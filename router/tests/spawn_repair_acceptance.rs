//! M-spawn-repair acceptance data-surface tests (C-model-spawn §7.2,
//! C-spawn §3.3).
//!
//! `SpawnSubmitAcceptance` must carry everything the real execution sink
//! needs to reconstruct the outbound `spawn.submit.request` wire without
//! re-parsing: the raw wire header (service/activation identity, actorMethod
//! metadata) plus the immutable opaque args payload, and the Router-generated
//! `spawnId`/`requestId` correlation keys.

use std::collections::BTreeMap;

use serde::Deserialize;
use skiff_runtime_transport::protocol::{
    decode_spawn_submit_request_frame, decode_spawn_submit_response_frame,
    encode_spawn_submit_request_frame, encode_spawn_submit_response_frame,
    spawn_submit_frame_direction, FrameDirection, RuntimeFrameFamily, SpawnSubmitAcceptance,
    SpawnSubmitRequestFrame,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    frames: BTreeMap<String, FrameEntry>,
}

fn catalog() -> Catalog {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("spawn-wire")
        .join("frames.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
        .expect("spawn wire corpus must decode")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_repair_acceptance_preserves_raw_wire_request_for_reconstruction() {
        let catalog = catalog();
        for name in [
            "spawn.submit.request.function",
            "spawn.submit.request.actorMethod",
        ] {
            let bytes = hex_bytes(&catalog.frames[name].frame_hex);
            let (header, payload) = decode_spawn_submit_request_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let frame = SpawnSubmitRequestFrame { header, payload };

            // Raw wire header + args bytes survive the acceptance boundary
            // and reconstruct the exact outbound frame bytes.
            assert_eq!(
                encode_spawn_submit_request_frame(&frame.header, &frame.payload)
                    .expect("reconstruct"),
                bytes,
                "{name}: acceptance projection must be byte-exact"
            );
            assert!(!frame.payload.is_empty(), "{name}: args bytes required");
            assert_eq!(frame.header.runtime_id, "runtime-a", "{name}");
            assert_eq!(
                frame.header.activation_identity.assembly_identity,
                format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64)),
                "{name}: activation identity preserved"
            );
            if name == "spawn.submit.request.actorMethod" {
                let actor_method = frame
                    .header
                    .actor_method
                    .expect("actorMethod metadata must be preserved");
                assert_eq!(
                    actor_method.actor_ref.service_id, "example.com/docs",
                    "actorMethod.serviceId"
                );
                assert!(
                    actor_method
                        .method_identity
                        .as_str()
                        .starts_with("skiff-actor-method-v1:sha256:"),
                    "actorMethod.methodIdentity preserved"
                );
            }
        }
    }

    #[test]
    fn spawn_repair_acceptance_response_projection_matches_frozen_corpus() {
        let catalog = catalog();
        let request_bytes = hex_bytes(&catalog.frames["spawn.submit.request.function"].frame_hex);
        let (header, payload) = decode_spawn_submit_request_frame(&request_bytes)
            .expect("spawn.submit.request.function must decode");
        let acceptance = SpawnSubmitAcceptance {
            request: SpawnSubmitRequestFrame { header, payload },
            spawn_id: "spawn-1".to_string(),
            request_id: "req:spawned-1".to_string(),
        };

        let response = acceptance.response_header();
        assert_eq!(response.rpc_id, "rpc:spawn-1");
        assert_eq!(response.spawn_id, "spawn-1");
        assert_eq!(response.request_id, "req:spawned-1");
        assert_eq!(response.status, "submitted");
        let response_bytes =
            encode_spawn_submit_response_frame(&response).expect("acceptance response must encode");
        assert_eq!(
            response_bytes,
            hex_bytes(&catalog.frames["spawn.submit.response"].frame_hex),
            "acceptance response projection must be byte-exact with the frozen corpus"
        );
        assert_eq!(
            decode_spawn_submit_response_frame(&response_bytes).expect("must decode"),
            response
        );
    }

    #[test]
    fn spawn_repair_acceptance_corpus_directions_match_registry_and_frame_table() {
        let catalog = catalog();
        assert_eq!(
            RuntimeFrameFamily::Spawn.direction(),
            FrameDirection::Either,
            "spawn family is mixed-direction at family level"
        );
        for (name, entry) in &catalog.frames {
            let (frame_type, expected) = match name.as_str() {
                "spawn.submit.request.function"
                | "spawn.submit.request.actorMethod"
                | "spawn.submit.request.legacy-no-caller-kind" => {
                    ("spawn.submit.request", FrameDirection::RuntimeToRouter)
                }
                "spawn.submit.response" => {
                    ("spawn.submit.response", FrameDirection::RouterToRuntime)
                }
                "spawn.submit.error.parentNotFound" => {
                    ("spawn.submit.error", FrameDirection::RouterToRuntime)
                }
                _ => panic!("unexpected spawn frame {name}"),
            };
            assert_eq!(
                spawn_submit_frame_direction(frame_type),
                Some(expected),
                "{name}: frame-level direction table"
            );
            let corpus_direction = match entry.direction.as_str() {
                "RuntimeToRouter" => FrameDirection::RuntimeToRouter,
                "RouterToRuntime" => FrameDirection::RouterToRuntime,
                other => panic!("{name}: unexpected corpus direction {other}"),
            };
            assert_eq!(corpus_direction, expected, "{name}: corpus direction");
        }
    }
}

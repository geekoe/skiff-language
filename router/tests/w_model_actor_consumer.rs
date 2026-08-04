//! M-actor consumer gate: skiff-router consumes the frozen C-model-actor
//! corpus (`runtime/transport/testdata/actor-wire/`) through the canonical
//! transport codecs and asserts the 20-frame actor wire is byte-exact on both
//! decode and re-encode.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::actor_method::{decode_actor_method_frame, encode_actor_method_frame};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    decode_actor_owner_invoke_frame, encode_actor_owner_control_ack_frame,
    encode_actor_owner_control_frame, encode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, ActorFindRequestFrameHeader,
    ActorFindResponseFrameHeader, ActorGetOrCreateRequestFrameHeader,
    ActorGetOrCreateResponseFrameHeader, ActorRemoveRequestFrameHeader,
    ActorRemoveResponseFrameHeader, ActorReplaceRequestFrameHeader,
    ActorReplaceResponseFrameHeader, ActorTaskRuntimeErrorFrameHeader, FrameDirection,
    PayloadPresenceRule, RuntimeFrameFamily,
};

const REQUIRED_FRAMES: [&str; 20] = [
    "actor.getOrCreate.request",
    "actor.getOrCreate.response",
    "actor.getOrCreate.error",
    "actor.replace.request",
    "actor.replace.response",
    "actor.replace.error",
    "actor.find.request",
    "actor.find.response",
    "actor.find.error",
    "actor.remove.request",
    "actor.remove.response",
    "actor.remove.error",
    "actor.method.invoke",
    "actor.method.return",
    "actor.method.error",
    "actor.method.cancel",
    "actor.owner.invoke",
    "actor.owner.control.activateInitial",
    "actor.owner.control.ack",
    "actor.owner.failure",
];

const REQUIRED_SCENARIOS: [&str; 22] = [
    "claim-reserve-commit-single-owner",
    "claim-reserve-conflict-while-owner-held",
    "claim-abort-no-effect",
    "claim-commit-twice-rejected",
    "claim-reservation-not-owner",
    "lease-expire-releases-fence",
    "get-or-create-first-joins-same-outcome",
    "get-or-create-lineage-conflict",
    "get-or-create-existing-no-reserve",
    "get-or-create-ack-timeout-aborts-token",
    "invoke-return-exact-owner",
    "invoke-error-caller-forward",
    "invoke-cancel-correlation",
    "invoke-duplicate-settle-rejected",
    "invoke-owner-disconnect-terminals-pending",
    "control-ack-exact-correlation",
    "control-ack-timeout-rejected",
    "control-late-ack-tombstone",
    "control-ack-wrong-operation-rejected",
    "lease-sweep-expire-and-idle-evict",
    "lease-eviction-ack-clears-request",
    "lease-eviction-retry-bounded",
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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/actor-wire")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("actor-wire frames.json must be readable");
    serde_json::from_str(&value).expect("actor-wire frames.json must decode")
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
    fn router_consumer_roundtrips_actor_wire_corpus_through_canonical_codecs() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "actor-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "corpus must contain required frame {required}"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        assert_eq!(
            RuntimeFrameFamily::Actor.direction(),
            FrameDirection::Either
        );
        assert_eq!(
            RuntimeFrameFamily::Actor.payload_presence(),
            PayloadPresenceRule::Optional
        );
        assert_eq!(RuntimeFrameFamily::Actor.wire_type_prefix(), "actor.");

        for (name, entry) in &catalog.frames {
            let bytes = hex_bytes(&entry.frame_hex);
            let reencoded = match entry.decode_as.as_str() {
                "ActorGetOrCreateRequest" => {
                    let (header, payload): (ActorGetOrCreateRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorGetOrCreateResponse" => {
                    let (header, payload): (ActorGetOrCreateResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorTaskRuntimeError" => {
                    let (header, payload): (ActorTaskRuntimeErrorFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorReplaceRequest" => {
                    let (header, payload): (ActorReplaceRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorReplaceResponse" => {
                    let (header, payload): (ActorReplaceResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorFindRequest" => {
                    let (header, payload): (ActorFindRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorFindResponse" => {
                    let (header, payload): (ActorFindResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorRemoveRequest" => {
                    let (header, payload): (ActorRemoveRequestFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorRemoveResponse" => {
                    let (header, payload): (ActorRemoveResponseFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_binary_frame(&header, &payload).expect("re-encode")
                }
                "ActorMethodInvoke" | "ActorMethodReturn" | "ActorMethodError"
                | "ActorMethodCancel" => {
                    let frame = decode_actor_method_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_method_frame(&frame).expect("re-encode")
                }
                "ActorOwnerInvoke" => {
                    let (header, payload) = decode_actor_owner_invoke_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_invoke_frame(&header, &payload).expect("re-encode")
                }
                "ActorOwnerControl" => {
                    let header = decode_actor_owner_control_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_control_frame(&header).expect("re-encode")
                }
                "ActorOwnerControlAck" => {
                    let (header, payload): (ActorOwnerControlAckFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name}: {error}"));
                    assert!(payload.is_empty(), "{name}: ack payload must be empty");
                    encode_actor_owner_control_ack_frame(&header).expect("re-encode")
                }
                "ActorOwnerFailure" => {
                    let header = decode_actor_owner_failure_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    encode_actor_owner_failure_frame(&header).expect("re-encode")
                }
                other => panic!("{name}: unknown decodeAs {other}"),
            };
            assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
            if entry.payload_presence == "empty" {
                assert!(
                    frame_payload(&bytes).is_empty(),
                    "{name}: empty-presence frame has payload"
                );
            }
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
                entry.direction,
                expected_direction(name),
                "{name}: direction"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v4",
                "{name}: header schemaVersion"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
        }
    }

    #[test]
    fn router_consumer_sees_all_frozen_actor_scenarios() {
        let mut names = Vec::new();
        for entry in fs::read_dir(corpus_dir().join("scenarios"))
            .expect("actor scenarios dir must be readable")
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
                "required actor scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }

    fn expected_decode_as(name: &str) -> &'static str {
        match name {
            "actor.getOrCreate.request" => "ActorGetOrCreateRequest",
            "actor.getOrCreate.response" => "ActorGetOrCreateResponse",
            "actor.getOrCreate.error" => "ActorTaskRuntimeError",
            "actor.replace.request" => "ActorReplaceRequest",
            "actor.replace.response" => "ActorReplaceResponse",
            "actor.replace.error" => "ActorTaskRuntimeError",
            "actor.find.request" => "ActorFindRequest",
            "actor.find.response" => "ActorFindResponse",
            "actor.find.error" => "ActorTaskRuntimeError",
            "actor.remove.request" => "ActorRemoveRequest",
            "actor.remove.response" => "ActorRemoveResponse",
            "actor.remove.error" => "ActorTaskRuntimeError",
            "actor.method.invoke" => "ActorMethodInvoke",
            "actor.method.return" => "ActorMethodReturn",
            "actor.method.error" => "ActorMethodError",
            "actor.method.cancel" => "ActorMethodCancel",
            "actor.owner.invoke" => "ActorOwnerInvoke",
            "actor.owner.control.activateInitial" => "ActorOwnerControl",
            "actor.owner.control.ack" => "ActorOwnerControlAck",
            "actor.owner.failure" => "ActorOwnerFailure",
            _ => panic!("unexpected frame {name}"),
        }
    }

    fn expected_frame_type(name: &str) -> &str {
        if name == "actor.owner.control.activateInitial" {
            "actor.owner.control"
        } else {
            name
        }
    }

    fn expected_direction(name: &str) -> &'static str {
        match name {
            "actor.getOrCreate.request"
            | "actor.replace.request"
            | "actor.find.request"
            | "actor.remove.request"
            | "actor.method.invoke"
            | "actor.owner.control.ack"
            | "actor.owner.failure" => "RuntimeToRouter",
            "actor.getOrCreate.response"
            | "actor.getOrCreate.error"
            | "actor.replace.response"
            | "actor.replace.error"
            | "actor.find.response"
            | "actor.find.error"
            | "actor.remove.response"
            | "actor.remove.error"
            | "actor.owner.invoke"
            | "actor.owner.control.activateInitial" => "RouterToRuntime",
            "actor.method.return" | "actor.method.error" | "actor.method.cancel" => "Either",
            _ => panic!("unexpected frame {name}"),
        }
    }
}

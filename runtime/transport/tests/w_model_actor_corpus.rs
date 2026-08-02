//! W-model-actor corpus gate: the 20-frame actor-wire corpus is consumed
//! through the canonical production codecs (actor_method / actor_owner /
//! actor control typed DTOs) and must roundtrip byte-exact.
//!
//! Frozen contract: `doc/implementation/router-rust-migration-c-model-actor-contract.md`.

use std::collections::BTreeMap;

use base64::Engine as _;
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
    ActorReplaceResponseFrameHeader, ActorSpawnRuntimeErrorFrameHeader, FrameDirection,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "payloadBase64")]
    payload_base64: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn catalog() -> Catalog {
    serde_json::from_str(include_str!("../testdata/actor-wire/frames.json"))
        .expect("actor wire corpus must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("frameHex hex"))
        .collect()
}

fn payload_of(entry: &FrameEntry) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(&entry.payload_base64)
        .expect("payloadBase64 must be canonical base64")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w_model_actor_corpus_is_frozen() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "actor-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "required actor frame {required} is missing from frames.json"
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
    }

    #[test]
    fn w_model_actor_all_frames_roundtrip_byte_exact_through_canonical_codecs() {
        let catalog = catalog();
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
                "ActorSpawnRuntimeError" => {
                    let (header, payload): (ActorSpawnRuntimeErrorFrameHeader, Vec<u8>) =
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
                    assert!(
                        payload.is_empty(),
                        "{name}: control ack payload must be empty"
                    );
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
            assert_eq!(
                entry.direction,
                expected_direction(name),
                "{name}: direction"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v3",
                "{name}: header schemaVersion"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
            if entry.payload_presence == "empty" {
                assert!(
                    payload_of(entry).is_empty(),
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
        }
    }

    #[test]
    fn w_model_actor_method_codec_rejects_malformed_identity_and_ref() {
        let catalog = catalog();
        let entry = &catalog.frames["actor.method.invoke"];
        let base = entry.header.clone();
        let cases = [
            (
                "canonicalActorIdKeyBytesBase64",
                serde_json::Value::String("!!!not-base64!!!".to_string()),
            ),
            (
                "actorIdHash",
                serde_json::Value::String("actor-hash-1".to_string()),
            ),
        ];
        for (field, value) in cases {
            let mut mutated = base.clone();
            mutated["actorRef"][field] = value;
            let bytes =
                encode_binary_frame(&mutated, &payload_of(entry)).expect("frame must encode");
            assert!(
                decode_actor_method_frame(&bytes).is_err(),
                "actor.method.invoke must reject malformed actorRef.{field}"
            );
        }

        let mut zero_epoch = base.clone();
        zero_epoch["actorRef"]["epoch"] = serde_json::Value::from(0);
        let bytes =
            encode_binary_frame(&zero_epoch, &payload_of(entry)).expect("frame must encode");
        assert!(
            decode_actor_method_frame(&bytes).is_err(),
            "actor.method.invoke must reject zero actorRef.epoch"
        );

        let mut bad_abi = base.clone();
        bad_abi["actorAbiIdentity"] = serde_json::Value::String("actor-abi:thread".to_string());
        let bytes = encode_binary_frame(&bad_abi, &payload_of(entry)).expect("frame must encode");
        assert!(
            decode_actor_method_frame(&bytes).is_err(),
            "actor.method.invoke must reject a non-canonical actorAbiIdentity"
        );

        let mut bad_arguments = base.clone();
        bad_arguments["argumentsEncodingVersion"] =
            serde_json::Value::String("skiff-actor-arguments-v0".to_string());
        let bytes =
            encode_binary_frame(&bad_arguments, &payload_of(entry)).expect("frame must encode");
        assert!(
            decode_actor_method_frame(&bytes).is_err(),
            "actor.method.invoke must reject an unsupported argumentsEncodingVersion"
        );

        let mut empty_cancellation = base;
        empty_cancellation["cancellationCorrelation"] = serde_json::Value::String(String::new());
        let bytes = encode_binary_frame(&empty_cancellation, &payload_of(entry))
            .expect("frame must encode");
        assert!(
            decode_actor_method_frame(&bytes).is_err(),
            "actor.method.invoke must reject an empty cancellationCorrelation"
        );
    }

    #[test]
    fn w_model_actor_control_family_enforces_paired_test_authority() {
        let catalog = catalog();
        let entry = &catalog.frames["actor.getOrCreate.request"];
        let mut mutated = entry.header.clone();
        mutated["testCaseCapability"] =
            serde_json::Value::String("test-case:capability_1".to_string());
        let bytes = encode_binary_frame(&mutated, &payload_of(entry)).expect("frame must encode");
        assert!(
            decode_typed_binary_frame::<ActorGetOrCreateRequestFrameHeader>(&bytes).is_err(),
            "actor.getOrCreate.request must reject a testCaseCapability without testCaseParentRequestId"
        );
    }

    #[test]
    fn w_model_actor_owner_control_enforces_operation_constraints() {
        let catalog = catalog();
        let entry = &catalog.frames["actor.owner.control.activateInitial"];
        let mut without_bootstrap = entry.header.clone();
        without_bootstrap
            .as_object_mut()
            .expect("header object")
            .remove("bootstrap");
        let bytes = encode_binary_frame(&without_bootstrap, &[]).expect("frame must encode");
        assert!(
            decode_actor_owner_control_frame(&bytes).is_err(),
            "activateInitial must require bootstrap"
        );

        let mut idle_evict_with_transition = entry.header.clone();
        idle_evict_with_transition["operation"] =
            serde_json::Value::String("idleEvict".to_string());
        idle_evict_with_transition["transition"] = serde_json::json!({
            "oldEpoch": 6,
            "newEpoch": 7,
            "actorAbiIdentity": idle_evict_with_transition["fence"]["actorAbiIdentity"].clone(),
            "targetImplementationIdentity": idle_evict_with_transition["fence"]["actorImplementationIdentity"].clone(),
            "bootstrapEncodingVersion": "skiff-canonical-v1",
            "bootstrapPayloadBase64": "Cgs="
        });
        let bytes =
            encode_binary_frame(&idle_evict_with_transition, &[]).expect("frame must encode");
        assert!(
            decode_actor_owner_control_frame(&bytes).is_err(),
            "idleEvict must not carry transition"
        );
    }

    #[test]
    fn w_model_actor_scenario_names_are_frozen() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/actor-wire/scenarios"
        ))
        .expect("actor scenarios dir must be readable")
        {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(&path).expect("scenario must be readable"),
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
            "actor.getOrCreate.error" => "ActorSpawnRuntimeError",
            "actor.replace.request" => "ActorReplaceRequest",
            "actor.replace.response" => "ActorReplaceResponse",
            "actor.replace.error" => "ActorSpawnRuntimeError",
            "actor.find.request" => "ActorFindRequest",
            "actor.find.response" => "ActorFindResponse",
            "actor.find.error" => "ActorSpawnRuntimeError",
            "actor.remove.request" => "ActorRemoveRequest",
            "actor.remove.response" => "ActorRemoveResponse",
            "actor.remove.error" => "ActorSpawnRuntimeError",
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

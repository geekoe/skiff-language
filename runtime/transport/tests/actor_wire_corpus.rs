//! Byte-exact actor-family wire corpus verifier for C-model-actor
//! (`doc/implementation/router-rust-migration-c-model-actor-contract.md`).
//!
//! This is a TEST-ONLY reference model. It is not production code, is not
//! imported by any production crate, and must not be treated as the W-actor
//! implementation. W-actor must implement the frozen semantics and consume
//! the same fixtures through the real codecs.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::actor_method::{
    decode_actor_method_frame, encode_actor_method_frame, ActorMethodFrame,
};
use skiff_runtime_transport::actor_owner::{
    decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
    decode_actor_owner_invoke_frame, encode_actor_owner_control_ack_frame,
    encode_actor_owner_control_frame, encode_actor_owner_failure_frame,
    encode_actor_owner_invoke_frame, ActorOwnerControlAckFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, ActorFindRequestFrameHeader, ActorFindResponseFrameHeader,
    ActorGetOrCreateRequestFrameHeader, ActorGetOrCreateResponseFrameHeader,
    ActorRemoveRequestFrameHeader, ActorRemoveResponseFrameHeader,
    ActorReplaceRequestFrameHeader, ActorReplaceResponseFrameHeader,
    ActorTaskRuntimeErrorFrameHeader, FrameDirection, PayloadPresenceRule, RuntimeFrameFamily,
    RuntimeFrameFamilyRule,
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
    serde_json::from_str(include_str!(
        "../testdata/actor-wire/frames.json"
    ))
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

fn assert_frame_bytes(hex: &str, expected_header: &Value, expected_payload: &[u8]) {
    let bytes = hex_bytes(hex);
    let frame = skiff_runtime_transport::protocol::decode_binary_frame(&bytes)
        .expect("frameHex must decode as a binary frame");
    assert_eq!(&frame.header, expected_header, "frame header JSON mismatch");
    assert_eq!(&frame.payload_bytes, expected_payload, "payload mismatch");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_required_frames_are_frozen() {
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
    }

    #[test]
    fn actor_frame_family_rules_are_frozen() {
        assert_eq!(RuntimeFrameFamily::Actor.direction(), FrameDirection::Either);
        assert_eq!(
            RuntimeFrameFamily::Actor.payload_presence(),
            PayloadPresenceRule::Optional
        );
        assert_eq!(RuntimeFrameFamily::Actor.wire_type_prefix(), "actor.");
        let rule = runtime_actor_rule();
        assert_eq!(rule.family, RuntimeFrameFamily::Actor);
        assert_eq!(rule.direction, FrameDirection::Either);
        assert_eq!(rule.payload_presence, PayloadPresenceRule::Optional);
    }

    #[test]
    fn all_frames_are_byte_exact_and_payload_consistent() {
        let catalog = catalog();
        for (name, entry) in &catalog.frames {
            let payload = payload_of(entry);
            assert_frame_bytes(&entry.frame_hex, &entry.header, &payload);
            assert_eq!(
                entry.frame_type,
                expected_frame_type(name),
                "{name}: frameType"
            );
            assert!(
                matches!(
                    entry.direction.as_str(),
                    "RouterToRuntime" | "RuntimeToRouter" | "Either"
                ),
                "{name}: unexpected direction {}",
                entry.direction
            );
            assert!(
                matches!(
                    entry.payload_presence.as_str(),
                    "empty" | "optional" | "required"
                ),
                "{name}: unexpected payloadPresence {}",
                entry.payload_presence
            );
            if entry.payload_presence == "empty" {
                assert!(payload.is_empty(), "{name}: empty-presence frame has payload");
            }
            assert_eq!(entry.decode_as, expected_decode_as(name), "{name}: decodeAs");
        }
    }

    #[test]
    fn actor_control_frames_decode_through_canonical_codecs() {
        let catalog = catalog();
        let entry = &catalog.frames["actor.getOrCreate.request"];
        let (header, payload) = decode_typed_binary_frame::<ActorGetOrCreateRequestFrameHeader>(
            &hex_bytes(&entry.frame_hex),
        )
        .expect("getOrCreate.request must decode");
        assert_eq!(
            serde_json::from_value::<ActorGetOrCreateRequestFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );
        assert_eq!(payload, payload_of(entry));

        let entry = &catalog.frames["actor.getOrCreate.response"];
        let header: ActorGetOrCreateResponseFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("getOrCreate.response must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorGetOrCreateResponseFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.getOrCreate.error"];
        let header: ActorTaskRuntimeErrorFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("getOrCreate.error must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorTaskRuntimeErrorFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.replace.request"];
        let header: ActorReplaceRequestFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("replace.request must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorReplaceRequestFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.replace.response"];
        let header: ActorReplaceResponseFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("replace.response must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorReplaceResponseFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.find.request"];
        let header: ActorFindRequestFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("find.request must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorFindRequestFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.find.response"];
        let header: ActorFindResponseFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("find.response must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorFindResponseFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.remove.request"];
        let header: ActorRemoveRequestFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("remove.request must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorRemoveRequestFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );

        let entry = &catalog.frames["actor.remove.response"];
        let header: ActorRemoveResponseFrameHeader =
            decode_typed_binary_frame(&hex_bytes(&entry.frame_hex))
                .expect("remove.response must decode")
                .0;
        assert_eq!(
            serde_json::from_value::<ActorRemoveResponseFrameHeader>(entry.header.clone())
                .expect("fixture header must be typed"),
            header
        );
    }

    #[test]
    fn actor_method_frames_round_trip_through_canonical_codec() {
        let catalog = catalog();
        for name in [
            "actor.method.invoke",
            "actor.method.return",
            "actor.method.error",
            "actor.method.cancel",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let frame = decode_actor_method_frame(&bytes).expect("method frame must decode");
            let reencoded = encode_actor_method_frame(&frame).expect("method frame must re-encode");
            assert_eq!(bytes, reencoded, "{name} must be byte-exact");
            assert_eq!(
                payload_of(entry),
                frame_payload(&frame),
                "{name} payload mismatch"
            );
        }
    }

    #[test]
    fn actor_owner_frames_round_trip_through_canonical_codec() {
        let catalog = catalog();
        let entry = &catalog.frames["actor.owner.invoke"];
        let bytes = hex_bytes(&entry.frame_hex);
        let (header, payload) =
            decode_actor_owner_invoke_frame(&bytes).expect("owner.invoke must decode");
        assert_eq!(
            bytes,
            encode_actor_owner_invoke_frame(&header, &payload).expect("owner.invoke re-encode"),
            "actor.owner.invoke must be byte-exact"
        );
        assert_eq!(payload, payload_of(entry));

        let entry = &catalog.frames["actor.owner.control.activateInitial"];
        let bytes = hex_bytes(&entry.frame_hex);
        let header =
            decode_actor_owner_control_frame(&bytes).expect("owner.control must decode");
        assert_eq!(
            bytes,
            encode_actor_owner_control_frame(&header).expect("owner.control re-encode"),
            "actor.owner.control must be byte-exact"
        );

        let entry = &catalog.frames["actor.owner.control.ack"];
        let bytes = hex_bytes(&entry.frame_hex);
        let header: ActorOwnerControlAckFrameHeader =
            decode_typed_binary_frame(&bytes).expect("owner.control.ack must decode").0;
        assert_eq!(
            bytes,
            encode_actor_owner_control_ack_frame(&header).expect("owner.control.ack re-encode"),
            "actor.owner.control.ack must be byte-exact"
        );

        let entry = &catalog.frames["actor.owner.failure"];
        let bytes = hex_bytes(&entry.frame_hex);
        let header = decode_actor_owner_failure_frame(&bytes).expect("owner.failure must decode");
        assert_eq!(
            bytes,
            encode_actor_owner_failure_frame(&header).expect("owner.failure re-encode"),
            "actor.owner.failure must be byte-exact"
        );
    }

    fn frame_payload(frame: &ActorMethodFrame) -> Vec<u8> {
        match frame {
            ActorMethodFrame::Invoke(_, payload) | ActorMethodFrame::Return(_, payload) => {
                payload.clone()
            }
            ActorMethodFrame::Error(_) | ActorMethodFrame::Cancel(_) => Vec::new(),
        }
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

    fn expected_frame_type<'a>(name: &'a str) -> &'a str {
        if name == "actor.owner.control.activateInitial" {
            "actor.owner.control"
        } else {
            name
        }
    }

    fn runtime_actor_rule() -> RuntimeFrameFamilyRule {
        RuntimeFrameFamilyRule {
            family: RuntimeFrameFamily::Actor,
            direction: RuntimeFrameFamily::Actor.direction(),
            payload_presence: RuntimeFrameFamily::Actor.payload_presence(),
        }
    }
}

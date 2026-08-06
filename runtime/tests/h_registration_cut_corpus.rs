//! H-registration-cut shared-corpus consumer gate (Runtime crate side).
//!
//! Consumes the frozen C-model-registration corpus
//! (`transport/testdata/registration-handshake/`) through the Runtime-side
//! transport codecs and asserts the frozen direction roles:
//! `router.bootstrap` / `runtime.registered` are inbound (Router -> Runtime);
//! `runtime.capabilities` / `assembly.activation:Register` / `runtime.health`
//! are outbound (Runtime -> Router); the legacy `runtime.register` frame is
//! not a handshake frame. Every frozen frame must roundtrip byte-exact.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame, decode_runtime_capabilities_frame, decode_runtime_health_frame,
    decode_runtime_registered_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_router_bootstrap_frame, encode_runtime_capabilities_frame, encode_runtime_health_frame,
    encode_runtime_registered_frame, RuntimeRegisterFrameHeader, ROUTER_BOOTSTRAP_FRAME_TYPE,
    RUNTIME_CAPABILITIES_FRAME_TYPE, RUNTIME_HEALTH_FRAME_TYPE, RUNTIME_REGISTERED_FRAME_TYPE,
};

const REQUIRED_FRAMES: [&str; 12] = [
    "bootstrap.prod.42",
    "capabilities.runtime-a",
    "capabilities.runtime-b",
    "register.prod.42.a",
    "register.prod.42.b",
    "register.prod.41.a",
    "register.prod.42.other-assembly",
    "register.prod.43.a",
    "registered.runtime-a",
    "registered.runtime-b",
    "health.empty",
    "legacy.runtime.register",
];

const REQUIRED_SCENARIOS: [&str; 20] = [
    "accept-sequence",
    "wrong-order-health-before-capabilities",
    "wrong-order-register-before-capabilities",
    "legacy-register-rejected",
    "identity-change-register-replica",
    "identity-change-capabilities-replica",
    "duplicate-register-pre-ack",
    "stale-register-old-generation",
    "tuple-mismatch-assembly",
    "new-generation-before-epoch-swap",
    "ack-loss",
    "health-before-ack-no-observation",
    "pre-auth-limit",
    "bootstrap-timeout",
    "capabilities-timeout",
    "register-timeout",
    "disconnect-mid-handshake",
    "re-register-exact-idempotent",
    "re-register-stale-after-ack",
    "capabilities-refresh-same-replica",
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

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transport/testdata/registration-handshake")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("registration-handshake frames.json must be readable");
    serde_json::from_str(&value).expect("registration-handshake frames.json must decode")
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

fn assert_hex_roundtrip(name: &str, frame: &[u8], reencoded: &[u8]) {
    assert_eq!(
        reencoded, frame,
        "{name} must roundtrip byte-exact through the Runtime-side codecs"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_side_handshake_frames_roundtrip_byte_exact_with_frozen_directions() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "registration-handshake-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        for (name, entry) in &catalog.frames {
            let bytes = hex_to_bytes(&entry.frame_hex);
            match entry.decode_as.as_str() {
                // Inbound (Router -> Runtime) frames.
                "RouterBootstrap" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let header = decode_router_bootstrap_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} bootstrap decode: {error}"));
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    assert_eq!(header.activation.profile, "prod");
                    assert_eq!(header.activation.generation, 42);
                    let reencoded = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} bootstrap encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Registered" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, RUNTIME_REGISTERED_FRAME_TYPE);
                    let header = decode_runtime_registered_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} registered decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_registered_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} registered encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                // Outbound (Runtime -> Router) frames.
                "Capabilities" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
                    let header = decode_runtime_capabilities_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} capabilities decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_capabilities_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} capabilities encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "AssemblyRegister" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert!(entry.frame_type.starts_with("assembly.activation"));
                    let control = decode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &bytes,
                    )
                    .unwrap_or_else(|error| panic!("{name} register decode: {error}"));
                    let AssemblyActivationControl::Register {
                        profile,
                        generation,
                        replica_id,
                        ..
                    } = &control
                    else {
                        panic!("{name} must decode as Register");
                    };
                    assert_eq!(profile, "prod");
                    assert_eq!(
                        *generation,
                        entry.header["control"]["generation"].as_u64().unwrap()
                    );
                    assert_eq!(
                        replica_id,
                        entry.header["control"]["replicaId"]
                            .as_str()
                            .expect("replicaId")
                    );
                    let reencoded = encode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &control,
                    )
                    .unwrap_or_else(|error| panic!("{name} register encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Health" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, RUNTIME_HEALTH_FRAME_TYPE);
                    let header = decode_runtime_health_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} health decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        entry.header["runtimeId"].as_str().expect("runtimeId"),
                        "{name} runtimeId must match fixture header"
                    );
                    let reencoded = encode_runtime_health_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} health encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "LegacyRegister" => {
                    // Not a target handshake frame: strict terminal
                    // (`LegacyRegisterRejected`) until the wire consumer
                    // deletes inbound legacy registration.
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, "runtime.register");
                    let (header, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name} legacy decode: {error}"));
                    assert!(payload.is_empty(), "{name} legacy payload must be empty");
                    let reencoded = encode_binary_frame(&header, &payload)
                        .expect("legacy register must encode");
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }
    }

    #[test]
    fn runtime_side_sees_all_frozen_handshake_scenarios_with_consistent_directions() {
        let catalog = catalog();
        let scenario_dir = corpus_dir().join("scenarios");
        let mut names = Vec::new();
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
            names.push(name.to_string());
            for event in value["events"].as_array().expect("events") {
                let kind = event["kind"].as_str().expect("event kind");
                let frame_name = event["frame"].as_str();
                match kind {
                    "accept" | "timeout" | "disconnect" => {}
                    "write" | "writeFail" => {
                        // Router -> Runtime outbound from the Router's
                        // perspective; inbound for the Runtime consumer.
                        let frame = frame_name.expect("write frame name");
                        let entry = catalog.frames.get(frame).expect("known frame");
                        assert_eq!(
                            entry.direction, "RouterToRuntime",
                            "{name}/{frame} must be RouterToRuntime"
                        );
                    }
                    "read" => {
                        // Runtime -> Router frames are read by the Router;
                        // they are outbound for the Runtime consumer.
                        let frame = frame_name.expect("read frame name");
                        let entry = catalog.frames.get(frame).expect("known frame");
                        assert_eq!(
                            entry.direction, "RuntimeToRouter",
                            "{name}/{frame} must be RuntimeToRouter"
                        );
                    }
                    other => panic!("{name} has unknown event kind {other}"),
                }
            }
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "corpus must contain required scenario {required}"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }
}

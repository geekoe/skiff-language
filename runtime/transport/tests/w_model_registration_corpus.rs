//! W-model-registration corpus gate.
//!
//! Consumes the frozen C-model-registration corpus
//! (`testdata/registration-handshake/`) through the W-model frame codecs and
//! proves the target handshake bytes roundtrip exactly:
//! `encode(decode(frameHex)) == frameHex`. The corpus itself is owned by the
//! contracts-session pack; this file adds the production-codec consumer gate
//! required by W-model-registration / M-registration (plan §5.3).

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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/registration-handshake")
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
        "{name} must roundtrip byte-exact through the W-model codec"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_frames_roundtrip_byte_exact_through_w_model_codecs() {
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
                "RouterBootstrap" => {
                    assert_eq!(entry.direction, "RouterToRuntime");
                    assert_eq!(entry.frame_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let header = decode_router_bootstrap_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} bootstrap decode: {error}"));
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let reencoded = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} bootstrap encode: {error}"));
                    assert_hex_roundtrip(name, &bytes, &reencoded);
                }
                "Capabilities" => {
                    assert_eq!(entry.direction, "RuntimeToRouter");
                    assert_eq!(entry.frame_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
                    let header = decode_runtime_capabilities_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} capabilities decode: {error}"));
                    assert_eq!(header.envelope_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
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
                    assert!(
                        matches!(control, AssemblyActivationControl::Register { .. }),
                        "{name} must decode as Register"
                    );
                    let value = serde_json::to_value(&control).expect("register must serialize");
                    assert_eq!(
                        value["replicaId"], entry.header["control"]["replicaId"],
                        "{name} replicaId must match fixture header"
                    );
                    assert_eq!(
                        value["generation"], entry.header["control"]["generation"],
                        "{name} generation must match fixture header"
                    );
                    let reencoded = encode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &control,
                    )
                    .unwrap_or_else(|error| panic!("{name} register encode: {error}"));
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
                    // Explicitly NOT a target handshake frame: the contracts-session
                    // pack freezes it as a strict terminal (`LegacyRegisterRejected`)
                    // until H-registration-cut deletes inbound legacy registration.
                    // The generic typed codec still roundtrips the frozen bytes.
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
    fn required_scenarios_exist_and_reference_only_known_frames() {
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
            assert_eq!(
                value["schemaVersion"],
                1,
                "{} schemaVersion",
                path.display()
            );
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
            assert!(
                value["epoch"]["profile"].is_string()
                    && value["epoch"]["generation"].is_u64()
                    && value["epoch"]["assembly"]["assemblyIdentity"].is_string()
                    && value["epoch"]["configSnapshot"]["snapshotId"].is_string(),
                "{name} must declare the frozen epoch tuple"
            );
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

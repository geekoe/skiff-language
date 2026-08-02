//! M-registration consumer gate: skiff-router consumes the frozen
//! C-model-registration corpus directly through the W-model transport codecs.
//!
//! The Router side of the target handshake (plan §3.5) decodes
//! `runtime.capabilities` / `assembly.activation:Register` / `runtime.health`
//! (RuntimeToRouter) and encodes `router.bootstrap` / `runtime.registered`
//! (RouterToRuntime). Every frozen frame must roundtrip byte-exact.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
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

const REQUIRED_SCENARIOS: [&str; 19] = [
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
];

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../runtime/transport/testdata/registration-handshake")
}

fn catalog() -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_consumer_roundtrips_handshake_corpus_through_w_model_codecs() {
        let catalog = catalog();
        assert_eq!(catalog["schemaVersion"], 1);
        assert_eq!(catalog["corpus"], "registration-handshake-v1");
        let frames = catalog["frames"]
            .as_object()
            .expect("frames must be an object");
        for name in REQUIRED_FRAMES {
            assert!(
                frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        for (name, entry) in frames {
            let entry = entry.as_object().expect("frame entry must be an object");
            let direction = entry["direction"].as_str().expect("direction");
            let frame_type = entry["frameType"].as_str().expect("frameType");
            let decode_as = entry["decodeAs"].as_str().expect("decodeAs");
            let frame_hex = entry["frameHex"].as_str().expect("frameHex");
            let fixture_header = &entry["header"];
            let bytes = hex_to_bytes(frame_hex);
            match decode_as {
                "RouterBootstrap" => {
                    assert_eq!(direction, "RouterToRuntime");
                    assert_eq!(frame_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let header = decode_router_bootstrap_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} bootstrap decode: {error}"));
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    assert_eq!(header.activation.environment, "prod");
                    assert_eq!(header.activation.generation, 42);
                    assert_eq!(
                    header.activation.assembly.assembly_identity.as_str(),
                    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                );
                    let reencoded = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} bootstrap encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "Capabilities" => {
                    assert_eq!(direction, "RuntimeToRouter");
                    assert_eq!(frame_type, RUNTIME_CAPABILITIES_FRAME_TYPE);
                    let header = decode_runtime_capabilities_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} capabilities decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        fixture_header["runtimeId"].as_str().expect("runtimeId")
                    );
                    let reencoded = encode_runtime_capabilities_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} capabilities encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "AssemblyRegister" => {
                    assert_eq!(direction, "RuntimeToRouter");
                    assert!(frame_type.starts_with("assembly.activation"));
                    let control = decode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &bytes,
                    )
                    .unwrap_or_else(|error| panic!("{name} register decode: {error}"));
                    let value = serde_json::to_value(&control).expect("register must serialize");
                    assert_eq!(value["type"], "register", "{name} must decode as Register");
                    assert_eq!(
                        value["replicaId"], fixture_header["control"]["replicaId"],
                        "{name} replicaId must match fixture"
                    );
                    assert_eq!(
                        value["generation"], fixture_header["control"]["generation"],
                        "{name} generation must match fixture"
                    );
                    let reencoded = encode_assembly_activation_frame(
                        AssemblyActivationFrameDirection::RuntimeToRouter,
                        &control,
                    )
                    .unwrap_or_else(|error| panic!("{name} register encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "Registered" => {
                    assert_eq!(direction, "RouterToRuntime");
                    assert_eq!(frame_type, RUNTIME_REGISTERED_FRAME_TYPE);
                    let header = decode_runtime_registered_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} registered decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        fixture_header["runtimeId"].as_str().expect("runtimeId")
                    );
                    let reencoded = encode_runtime_registered_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} registered encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "Health" => {
                    assert_eq!(direction, "RuntimeToRouter");
                    assert_eq!(frame_type, RUNTIME_HEALTH_FRAME_TYPE);
                    let header = decode_runtime_health_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} health decode: {error}"));
                    assert_eq!(
                        header.runtime_id,
                        fixture_header["runtimeId"].as_str().expect("runtimeId")
                    );
                    let reencoded = encode_runtime_health_frame(&header)
                        .unwrap_or_else(|error| panic!("{name} health encode: {error}"));
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                "LegacyRegister" => {
                    // Not a target handshake frame; frozen as strict terminal until
                    // H-registration-cut. The Router consumer still verifies the
                    // legacy bytes stay decodable through the generic typed codec.
                    assert_eq!(direction, "RuntimeToRouter");
                    assert_eq!(frame_type, "runtime.register");
                    let (header, payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes)
                            .unwrap_or_else(|error| panic!("{name} legacy decode: {error}"));
                    assert!(payload.is_empty());
                    let reencoded = encode_binary_frame(&header, &payload)
                        .expect("legacy register must encode");
                    assert_eq!(reencoded, bytes, "{name} must roundtrip byte-exact");
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            }
        }
    }

    #[test]
    fn router_consumer_sees_all_frozen_handshake_scenarios() {
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
            found.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
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

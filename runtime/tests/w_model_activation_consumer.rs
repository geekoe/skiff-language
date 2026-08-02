//! M-activation consumer gate: the `runtime` crate consumes the frozen
//! C-model-activation corpus directly through the W-model transport codecs.
//!
//! The Runtime side of the activation transaction wire (plan §4.1,
//! C-model-activation §1/§2) decodes `prepare` / `commit` / `abort`
//! (RouterToRuntime) and encodes `prepared` / `reject` (RuntimeToRouter).
//! Every frozen transaction frame must roundtrip byte-exact and the reverse
//! direction must fail closed.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};

const REQUIRED_TRANSACTION_FRAMES: [&str; 5] = ["prepare", "prepared", "reject", "commit", "abort"];

const REQUIRED_TRANSACTION_CASES: [&str; 22] = [
    "live-commit-happy-path",
    "live-stale-ack-old-session-rejected",
    "live-ack-from-nonparticipant-rejected-then-disconnect-aborts",
    "live-duplicate-ack-rejected",
    "live-reject-aborts",
    "live-disconnect-pre-decision-aborts",
    "live-replacement-pre-decision-aborts",
    "live-timeout-pre-decision-aborts",
    "live-queue-full-prepare-aborts",
    "live-queue-full-commit-aborts-session-only",
    "live-disconnect-after-commit-reconciles",
    "live-commit-cas-mismatch-durable-committed",
    "live-commit-cas-mismatch-durable-aborted",
    "live-shutdown-before-decision-aborts",
    "live-revalidation-stale-fails-closed",
    "live-prepare-cas-mismatch-fails-closed",
    "cold-recovery-committed-only",
    "cold-recovery-rebind-and-commit",
    "cold-recovery-waiting-for-participants",
    "cold-recovery-candidate-load-failure-aborts",
    "cold-recovery-stale-ack-after-rebind-rejected",
    "cold-recovery-exit-after-commit-before-swap",
];

#[derive(Debug, Clone, Deserialize)]
struct GoldenFrame {
    name: String,
    direction: String,
    #[serde(rename = "controlIndex")]
    control_index: usize,
    #[serde(rename = "frameHex")]
    frame_hex: String,
}

fn cross_system_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cross-system-fixtures/package-service-ecosystem")
}

fn runtime_wire() -> Value {
    let text = fs::read_to_string(cross_system_dir().join("runtime-wire.json"))
        .expect("runtime-wire.json must be readable");
    serde_json::from_str(&text).expect("runtime-wire.json must decode")
}

fn controls() -> Vec<AssemblyActivationControl> {
    let text = fs::read_to_string(cross_system_dir().join("control-wire.json"))
        .expect("control-wire.json must be readable");
    serde_json::from_str(&text).expect("control-wire.json must decode")
}

fn transaction_cases() -> Value {
    let text = fs::read_to_string(cross_system_dir().join("activation-transaction-cases.json"))
        .expect("activation-transaction-cases.json must be readable");
    serde_json::from_str(&text).expect("activation-transaction-cases.json must decode")
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

fn direction(value: &str) -> AssemblyActivationFrameDirection {
    match value {
        "routerToRuntime" => AssemblyActivationFrameDirection::RouterToRuntime,
        "runtimeToRouter" => AssemblyActivationFrameDirection::RuntimeToRouter,
        other => panic!("unknown direction {other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_consumer_roundtrips_activation_transaction_frames_through_w_model_codecs() {
        let wire = runtime_wire();
        let controls = controls();
        let frames = wire["assemblyActivationFrames"]
            .as_array()
            .expect("assemblyActivationFrames");
        let transaction: Vec<GoldenFrame> = frames
            .iter()
            .filter(|frame| frame["name"].as_str() != Some("register"))
            .map(|frame| {
                serde_json::from_value(frame.clone()).expect("transaction frame must decode")
            })
            .collect();
        assert_eq!(transaction.len(), REQUIRED_TRANSACTION_FRAMES.len());

        for frame in &transaction {
            assert!(
                REQUIRED_TRANSACTION_FRAMES.contains(&frame.name.as_str()),
                "unexpected transaction frame {}",
                frame.name
            );
            let expected_direction = match frame.name.as_str() {
                "prepare" | "commit" | "abort" => "routerToRuntime",
                "prepared" | "reject" => "runtimeToRouter",
                other => panic!("unexpected transaction frame {other}"),
            };
            assert_eq!(
                frame.direction, expected_direction,
                "{} direction",
                frame.name
            );
            let bytes = hex_to_bytes(&frame.frame_hex);
            let control = decode_assembly_activation_frame(direction(&frame.direction), &bytes)
                .unwrap_or_else(|error| panic!("{} decode: {error}", frame.name));
            assert_eq!(
                control, controls[frame.control_index],
                "{} golden control",
                frame.name
            );
            let reencoded = encode_assembly_activation_frame(direction(&frame.direction), &control)
                .unwrap_or_else(|error| panic!("{} encode: {error}", frame.name));
            assert_eq!(
                reencoded, bytes,
                "{} must roundtrip byte-exact through the W-model codec",
                frame.name
            );
            let opposite = match frame.direction.as_str() {
                "routerToRuntime" => AssemblyActivationFrameDirection::RuntimeToRouter,
                "runtimeToRouter" => AssemblyActivationFrameDirection::RouterToRuntime,
                other => panic!("unknown direction {other}"),
            };
            assert!(
                encode_assembly_activation_frame(opposite, &control).is_err(),
                "{} reverse encode must fail",
                frame.name
            );
        }
    }

    #[test]
    fn runtime_consumer_sees_all_frozen_activation_transaction_cases() {
        let corpus = transaction_cases();
        assert_eq!(
            corpus["schemaVersion"],
            "skiff-activation-transaction-corpus-v1"
        );
        let cases = corpus["cases"].as_array().expect("cases");
        let mut names = Vec::new();
        for case in cases {
            names.push(case["name"].as_str().expect("case name").to_string());
        }
        for required in REQUIRED_TRANSACTION_CASES {
            assert!(
                names.iter().any(|name| name == required),
                "corpus must contain required case {required}"
            );
        }
        assert_eq!(names.len(), REQUIRED_TRANSACTION_CASES.len());
    }
}

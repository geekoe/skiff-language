//! W-model-activation corpus gate.
//!
//! Consumes the frozen C-model-activation corpus (shared
//! `cross-system-fixtures/package-service-ecosystem/` fixtures:
//! `control-wire.json`, `runtime-wire.json`, `activation-transaction-cases.json`)
//! through the production W-model frame codec and proves the transaction wire
//! (`prepare` / `prepared` / `reject` / `commit` / `abort`) roundtrips
//! byte-exactly, follows the frozen direction matrix, rejects every mutation
//! case, and that every live/cold recovery transaction tuple is consumable by
//! the strict production DTO. This is the transport-side consumer gate
//! required by W-model-activation / M-activation (plan §5.3).

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId,
    RuntimeConfigSnapshotRef,
};
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

#[derive(Debug, Clone, Deserialize)]
struct Mutation {
    name: String,
    direction: String,
    #[serde(rename = "inputKind")]
    input_kind: String,
    input: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeWireCorpus {
    #[serde(rename = "assemblyActivationFrames")]
    assembly_activation_frames: Vec<GoldenFrame>,
    #[serde(rename = "assemblyActivationMutations")]
    assembly_activation_mutations: Vec<Mutation>,
}

fn cross_system_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../cross-system-fixtures/package-service-ecosystem")
}

fn runtime_wire() -> RuntimeWireCorpus {
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

fn assembly_ref(byte: char) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            byte.to_string().repeat(64)
        )),
    }
}

fn config_ref(byte: char) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(format!(
            "skiff-runtime-config-snapshot-v1:{}",
            byte.to_string().repeat(32)
        ))
        .expect("valid config snapshot id"),
    }
}

fn assert_tx_tuple_wire_valid(
    environment: &str,
    activation_id: &str,
    expected: u64,
    candidate: u64,
) {
    AssemblyActivationControl::Prepare {
        environment: environment.to_string(),
        activation_id: activation_id.to_string(),
        expected_generation: expected,
        candidate_generation: candidate,
        assembly: assembly_ref('a'),
        config_snapshot: config_ref('a'),
        replica_id: "replica-1".to_string(),
        service_db: None,
    }
    .validate()
    .unwrap_or_else(|error| panic!("tx tuple must be wire-valid: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_transaction_frames_roundtrip_byte_exact_through_w_model_codec() {
        let corpus = runtime_wire();
        let controls = controls();
        assert_eq!(corpus.assembly_activation_frames.len(), 6);
        assert_eq!(controls.len(), 6);

        let transaction: Vec<&GoldenFrame> = corpus
            .assembly_activation_frames
            .iter()
            .filter(|frame| frame.name != "register")
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

            // The frozen direction matrix is enforced by the production codec.
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
            assert!(
                decode_assembly_activation_frame(opposite, &bytes).is_err(),
                "{} reverse decode must fail",
                frame.name
            );
        }
    }

    #[test]
    fn activation_transaction_mutations_fail_closed_through_w_model_codec() {
        let corpus = runtime_wire();
        assert_eq!(corpus.assembly_activation_mutations.len(), 7);
        for mutation in &corpus.assembly_activation_mutations {
            let frame = match mutation.input_kind.as_str() {
                "frameHex" => hex_to_bytes(&mutation.input),
                "bytesHex" => hex_to_bytes(&mutation.input),
                other => panic!("{} unknown inputKind {other}", mutation.name),
            };
            assert!(
                decode_assembly_activation_frame(direction(&mutation.direction), &frame).is_err(),
                "{} must fail closed",
                mutation.name
            );
        }
    }

    #[test]
    fn activation_transaction_cases_are_complete_and_tx_tuples_are_wire_valid() {
        let corpus = transaction_cases();
        assert_eq!(
            corpus["schemaVersion"],
            "skiff-activation-transaction-corpus-v1"
        );
        let cases = corpus["cases"].as_array().expect("cases");
        assert_eq!(cases.len(), REQUIRED_TRANSACTION_CASES.len());
        let mut live = 0;
        let mut cold = 0;
        for case in cases {
            let name = case["name"].as_str().expect("case name");
            assert!(
                REQUIRED_TRANSACTION_CASES.contains(&name),
                "unexpected case {name}"
            );
            match case["contract"].as_str() {
                Some("live") => live += 1,
                Some("coldRecovery") => cold += 1,
                other => panic!("{name} unknown contract {other:?}"),
            }
            let tx = case.get("tx").or_else(|| case["runs"][0].get("tx"));
            if let Some(tx) = tx {
                assert_tx_tuple_wire_valid(
                    tx["environment"].as_str().expect("environment"),
                    tx["activationId"].as_str().expect("activationId"),
                    tx["expectedGeneration"]
                        .as_u64()
                        .expect("expectedGeneration"),
                    tx["candidateGeneration"]
                        .as_u64()
                        .expect("candidateGeneration"),
                );
            }
            for run in case["runs"].as_array().into_iter().flatten() {
                if let Some(tx) = run.get("tx") {
                    assert_tx_tuple_wire_valid(
                        tx["environment"].as_str().expect("environment"),
                        tx["activationId"].as_str().expect("activationId"),
                        tx["expectedGeneration"]
                            .as_u64()
                            .expect("expectedGeneration"),
                        tx["candidateGeneration"]
                            .as_u64()
                            .expect("candidateGeneration"),
                    );
                }
            }
        }
        assert!(live >= 15, "live corpus must stay exhaustive");
        assert!(cold >= 5, "cold recovery corpus must stay exhaustive");
    }
}

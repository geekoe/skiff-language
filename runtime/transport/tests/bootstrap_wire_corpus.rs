//! C-model-bootstrap-wire corpus consumer.
//!
//! Freezes the Router→Runtime `router.bootstrap` frame, the
//! `RuntimeAssemblyRef` / `RuntimeConfigSnapshotRef` shapes, strict artifact
//! inputs, direction and payload presence. This is a contract pack: it
//! references existing transport/artifact-model types without writing
//! production code.

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame_header, decode_typed_binary_frame, encode_binary_frame,
    FrameDirection, PayloadPresenceRule, RouterBootstrapFrameHeader, RuntimeFrameFamily,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BootstrapWireCorpus {
    schema_version: String,
    shared_corpus: String,
    family: FamilyContract,
    assembly_refs: Vec<RefCase>,
    config_snapshot_refs: Vec<RefCase>,
    frames: Vec<FrameCase>,
    payload_presence: Vec<PayloadPresenceCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FamilyContract {
    name: String,
    frame_type: String,
    direction: String,
    payload_presence: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefCase {
    id: String,
    json: Value,
    valid: bool,
    #[serde(default)]
    expect_error_contains: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameCase {
    id: String,
    json: Value,
    valid: bool,
    #[serde(default)]
    expect_error_contains: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PayloadPresenceCase {
    id: String,
    expect_reject: bool,
    enforced_by: String,
    current_enforced: bool,
    note: String,
}

fn corpus() -> BootstrapWireCorpus {
    serde_json::from_str(include_str!(
        "../testdata/router-rust-bootstrap-wire-corpus.json"
    ))
    .expect("bootstrap wire corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_frame_family_are_frozen() {
        let corpus = corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-bootstrap-wire-corpus-v1"
        );
        assert_eq!(
            corpus.shared_corpus,
            "cross-system-fixtures/package-service-ecosystem/runtime-bootstrap-wire.json"
        );
        assert_eq!(corpus.family.name, "Session");
        assert_eq!(corpus.family.frame_type, "router.bootstrap");
        assert_eq!(corpus.family.direction, "routerToRuntime");
        assert_eq!(corpus.family.payload_presence, "empty");

        // The frame family rule is the M0-frozen rule that backs this frame-level
        // contract: Session family frames carry no payload.
        assert_eq!(
            RuntimeFrameFamily::Session.payload_presence(),
            PayloadPresenceRule::Empty
        );
        assert_eq!(RuntimeFrameFamily::Session.wire_type_prefix(), "runtime.");
        // Family-level direction is Either (session carries both directions);
        // the frame-level direction for router.bootstrap is frozen in the corpus
        // as routerToRuntime.
        assert_eq!(
            RuntimeFrameFamily::Session.direction(),
            FrameDirection::Either
        );
    }

    #[test]
    fn assembly_ref_cases_follow_strict_deserialization() {
        for case in corpus().assembly_refs {
            let result = serde_json::from_value::<RuntimeAssemblyRef>(case.json.clone());
            match (case.valid, result) {
                (true, Ok(reference)) => {
                    assert!(
                        reference
                            .assembly_identity
                            .as_str()
                            .starts_with("skiff-runtime-assembly-v3:sha256:"),
                        "{} must parse a validated assembly ref",
                        case.id
                    );
                }
                (false, Err(error)) => {
                    let message = error.to_string();
                    if let Some(expected) = case.expect_error_contains {
                        assert!(
                            message.contains(&expected),
                            "{} must fail with {expected:?}, got {message}",
                            case.id
                        );
                    }
                }
                (true, Err(error)) => panic!("{} must parse, got {error}", case.id),
                (false, Ok(_)) => panic!("{} must be rejected", case.id),
            }
        }
    }

    #[test]
    fn config_snapshot_ref_cases_follow_strict_deserialization() {
        for case in corpus().config_snapshot_refs {
            let result = serde_json::from_value::<RuntimeConfigSnapshotRef>(case.json.clone());
            match (case.valid, result) {
                (true, Ok(reference)) => {
                    assert!(
                        reference
                            .snapshot_id
                            .as_str()
                            .starts_with("skiff-runtime-config-snapshot-v1:"),
                        "{} must parse a validated config snapshot ref",
                        case.id
                    );
                }
                (false, Err(error)) => {
                    let message = error.to_string();
                    if let Some(expected) = case.expect_error_contains {
                        assert!(
                            message.contains(&expected),
                            "{} must fail with {expected:?}, got {message}",
                            case.id
                        );
                    }
                }
                (true, Err(error)) => panic!("{} must parse, got {error}", case.id),
                (false, Ok(_)) => panic!("{} must be rejected", case.id),
            }
        }
    }

    #[test]
    fn bootstrap_frame_cases_decode_strictly_and_roundtrip() {
        let corpus = corpus();
        let valid_count = corpus.frames.iter().filter(|case| case.valid).count();
        assert!(
            valid_count >= 1,
            "corpus must contain at least one canonical bootstrap frame"
        );

        for case in corpus.frames {
            let result = decode_router_bootstrap_frame_header(case.json.clone());
            match (case.valid, result) {
                (true, Ok(header)) => {
                    assert_eq!(header.envelope_type, "router.bootstrap");
                    assert_eq!(
                    header.activation.assembly.assembly_identity.as_str(),
                    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                );
                    assert_eq!(
                        header.activation.config_snapshot.snapshot_id.as_str(),
                        "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    );
                    assert_eq!(header.activation.environment, "prod");
                    assert_eq!(header.activation.generation, 7);

                    // Canonical bootstrap frames must encode/decode with an empty
                    // payload and survive a typed roundtrip unchanged.
                    let frame = encode_binary_frame(&header, &[])
                        .unwrap_or_else(|error| panic!("{} must encode: {error}", case.id));
                    let (decoded, payload): (RouterBootstrapFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&frame)
                            .unwrap_or_else(|error| panic!("{} must decode: {error}", case.id));
                    assert_eq!(decoded, header, "{} must roundtrip exactly", case.id);
                    assert!(
                        payload.is_empty(),
                        "{} must carry an empty payload",
                        case.id
                    );
                }
                (false, Err(error)) => {
                    let message = error.to_string();
                    if let Some(expected) = case.expect_error_contains {
                        assert!(
                            message.contains(&expected),
                            "{} must fail with {expected:?}, got {message}",
                            case.id
                        );
                    }
                }
                (true, Err(error)) => panic!("{} must decode, got {error}", case.id),
                (false, Ok(_)) => panic!("{} must be rejected", case.id),
            }
        }
    }

    #[test]
    fn payload_presence_is_frozen_empty_and_w_model_enforcement_is_declared() {
        let corpus = corpus();
        assert!(
            !corpus.payload_presence.is_empty(),
            "corpus must declare the payload-presence enforcement case"
        );
        for case in corpus.payload_presence {
            assert!(case.expect_reject, "{} must expect rejection", case.id);
            assert_eq!(case.enforced_by, "W-model-bootstrap-wire");
            assert!(!case.note.is_empty(), "{} must carry a rationale", case.id);
            // The enforcement lands with W-model-bootstrap-wire; until then the
            // contract only freezes the rule and the registry-backed presence.
            assert!(!case.current_enforced);
        }
    }
}

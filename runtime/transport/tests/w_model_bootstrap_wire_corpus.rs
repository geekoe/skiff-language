//! W-model-bootstrap-wire corpus gate.
//!
//! Consumes the frozen C-model-bootstrap-wire corpus
//! (`testdata/router-rust-bootstrap-wire-corpus.json`) through the W-model
//! frame codec, proves frame-level payload-presence enforcement
//! (`currentEnforced` flipped by W-model), and exercises the captured-epoch
//! provider seam used by the connection task (plan §5.5).

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame, decode_router_bootstrap_frame_header, encode_binary_frame,
    encode_router_bootstrap_frame, RouterBootstrapFrameHeader, RouterBootstrapSource,
    RuntimeBootstrapProvider, StatelessRuntimeBootstrapProvider, ROUTER_BOOTSTRAP_FRAME_TYPE,
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

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/router-rust-bootstrap-wire-corpus.json")
}

fn corpus() -> BootstrapWireCorpus {
    let value = fs::read_to_string(corpus_path())
        .expect("router-rust-bootstrap-wire-corpus.json must be readable");
    serde_json::from_str(&value).expect("bootstrap wire corpus must decode")
}

fn canonical_header() -> RouterBootstrapFrameHeader {
    let corpus = corpus();
    let case = corpus
        .frames
        .iter()
        .find(|case| case.valid)
        .expect("corpus must contain a canonical frame");
    serde_json::from_value(case.json.clone()).expect("canonical frame must deserialize")
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
        assert_eq!(corpus.family.frame_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
        assert_eq!(corpus.family.direction, "routerToRuntime");
        assert_eq!(corpus.family.payload_presence, "empty");
    }

    #[test]
    fn bootstrap_frame_cases_decode_strictly_and_roundtrip_byte_exact() {
        let corpus = corpus();
        let valid_count = corpus.frames.iter().filter(|case| case.valid).count();
        assert!(
            valid_count >= 1,
            "corpus must contain at least one canonical frame"
        );

        for case in corpus.frames {
            let result = decode_router_bootstrap_frame_header(case.json.clone());
            match (case.valid, result) {
                (true, Ok(header)) => {
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    let frame = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{} must encode: {error}", case.id));
                    let decoded = decode_router_bootstrap_frame(&frame)
                        .unwrap_or_else(|error| panic!("{} must decode: {error}", case.id));
                    assert_eq!(decoded, header, "{} must roundtrip exactly", case.id);
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
    fn payload_presence_is_enforced_by_w_model_frame_codec() {
        let corpus = corpus();
        let header = canonical_header();

        // The canonical frame itself carries no payload and roundtrips.
        let canonical =
            encode_router_bootstrap_frame(&header).expect("canonical frame must encode");
        assert_eq!(
            decode_router_bootstrap_frame(&canonical).expect("canonical frame must decode"),
            header
        );

        // Non-empty payload must fail closed at frame level (contract §6.2).
        let with_payload =
            encode_binary_frame(&header, b"intruder").expect("raw frame with payload must encode");
        let error = decode_router_bootstrap_frame(&with_payload)
            .expect_err("non-empty bootstrap payload must be rejected");
        assert!(
            error.to_string().contains("payload must be empty"),
            "unexpected error: {error}"
        );

        for case in corpus.payload_presence {
            assert_eq!(case.id, "payload-non-empty-rejected");
            assert!(case.expect_reject);
            assert_eq!(case.enforced_by, "W-model-bootstrap-wire");
            assert!(!case.note.is_empty(), "{} must carry a rationale", case.id);
            assert!(
                case.current_enforced,
                "{} must be flipped by W-model",
                case.id
            );
        }
    }

    #[test]
    fn assembly_and_config_snapshot_ref_cases_follow_strict_deserialization() {
        let corpus = corpus();
        for case in corpus.assembly_refs {
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
        for case in corpus.config_snapshot_refs {
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
    fn stateless_provider_constructs_canonical_bootstrap_from_captured_source() {
        let header = canonical_header();
        let source = RouterBootstrapSource {
            artifacts_path: header.artifacts_path.clone(),
            service_db: header.service_db.clone(),
            http: header.http.clone(),
            profile: header.activation.profile.clone(),
        };
        let provider = StatelessRuntimeBootstrapProvider;
        let constructed = provider
            .bootstrap_frame(&source)
            .expect("stateless provider must construct a bootstrap header");
        assert_eq!(constructed, header);
        let frame = encode_router_bootstrap_frame(&constructed).expect("frame must encode");
        assert_eq!(
            decode_router_bootstrap_frame(&frame).expect("frame must decode"),
            constructed
        );

        // The strict decoder rejects an invalid profile.
        let mut invalid = source;
        invalid.profile = "prod env".to_string();
        assert!(provider.bootstrap_frame(&invalid).is_err());
    }
}

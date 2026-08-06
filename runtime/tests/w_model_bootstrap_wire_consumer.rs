//! M-bootstrap-wire consumer gate: the `runtime` crate consumes the frozen
//! C-model-bootstrap-wire corpus through the W-model frame codec. The Runtime
//! is the receiver of the one-shot `router.bootstrap` frame; strict header
//! decode, empty payload enforcement and the captured-epoch provider seam are
//! all consumed from the same corpus.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame, decode_router_bootstrap_frame_header, encode_binary_frame,
    encode_router_bootstrap_frame, RouterBootstrapFrameHeader, RouterBootstrapSource,
    RuntimeBootstrapProvider, StatelessRuntimeBootstrapProvider,
    ROUTER_BOOTSTRAP_FRAME_TYPE,
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
        .join("transport/testdata/router-rust-bootstrap-wire-corpus.json")
}

fn corpus() -> BootstrapWireCorpus {
    let value = fs::read_to_string(corpus_path())
        .expect("router-rust-bootstrap-wire-corpus.json must be readable");
    serde_json::from_str(&value).expect("bootstrap wire corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_consumer_freeze_checks_bootstrap_wire_corpus() {
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
    fn runtime_consumer_decodes_and_roundtrips_bootstrap_frames_strictly() {
        let corpus = corpus();
        for case in corpus.frames {
            let result = decode_router_bootstrap_frame_header(case.json.clone());
            match (case.valid, result) {
                (true, Ok(header)) => {
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    assert_eq!(header.activation.profile, "prod");
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
    fn runtime_consumer_enforces_empty_bootstrap_payload() {
        let corpus = corpus();
        let canonical = corpus
            .frames
            .iter()
            .find(|case| case.valid)
            .expect("corpus must contain a canonical frame");
        let header: RouterBootstrapFrameHeader = serde_json::from_value(canonical.json.clone())
            .expect("canonical frame must deserialize");

        let clean = encode_router_bootstrap_frame(&header).expect("canonical frame must encode");
        assert_eq!(
            decode_router_bootstrap_frame(&clean).expect("canonical frame must decode"),
            header
        );

        let with_payload =
            encode_binary_frame(&header, b"intruder").expect("raw frame must encode");
        let error = decode_router_bootstrap_frame(&with_payload)
            .expect_err("non-empty bootstrap payload must be rejected");
        assert!(
            error.to_string().contains("payload must be empty"),
            "unexpected error: {error}"
        );

        for case in corpus.payload_presence {
            assert!(case.expect_reject);
            assert_eq!(case.enforced_by, "W-model-bootstrap-wire");
            assert!(!case.note.is_empty(), "{} must carry a rationale", case.id);
            assert!(case.current_enforced, "{} must be enforced", case.id);
        }
    }

    #[test]
    fn runtime_consumer_validates_assembly_and_config_snapshot_refs_strictly() {
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
    fn runtime_consumer_consumes_provider_constructed_bootstrap_frame() {
        let corpus = corpus();
        let canonical = corpus
            .frames
            .iter()
            .find(|case| case.valid)
            .expect("corpus must contain a canonical frame");
        let header: RouterBootstrapFrameHeader = serde_json::from_value(canonical.json.clone())
            .expect("canonical frame must deserialize");

        let source = RouterBootstrapSource {
            artifacts_path: header.artifacts_path.clone(),
            service_db: header.service_db.clone(),
            http: header.http.clone(),
            profile: header.activation.profile.clone(),
        };
        let frame = encode_router_bootstrap_frame(
            &StatelessRuntimeBootstrapProvider
                .bootstrap_frame(&source)
                .expect("stateless provider must construct a bootstrap header"),
        )
        .expect("frame must encode");
        let decoded = decode_router_bootstrap_frame(&frame).expect("frame must decode");
        assert_eq!(decoded, header);
        assert_eq!(decoded.activation.profile, "prod");
    }
}

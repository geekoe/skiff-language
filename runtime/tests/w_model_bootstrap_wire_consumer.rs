//! M-bootstrap-wire consumer gate: the `runtime` crate consumes the frozen
//! C-model-bootstrap-wire corpus through the W-model frame codec and asserts
//! the Router->Runtime `router.bootstrap` surface (strict header decode,
//! empty payload enforcement, captured-epoch provider seam).

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_router_bootstrap_frame, decode_router_bootstrap_frame_header, encode_binary_frame,
    encode_router_bootstrap_frame, RouterBootstrapFrameHeader, RouterBootstrapSource,
    RuntimeBootstrapProvider, StatelessRuntimeBootstrapProvider, ROUTER_BOOTSTRAP_FRAME_TYPE,
};

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("transport/testdata/router-rust-bootstrap-wire-corpus.json")
}

fn corpus() -> Value {
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
            corpus["schemaVersion"],
            "skiff-router-rust-bootstrap-wire-corpus-v1"
        );
        assert_eq!(
            corpus["sharedCorpus"],
            "cross-system-fixtures/package-service-ecosystem/runtime-bootstrap-wire.json"
        );
        assert_eq!(corpus["family"]["name"], "Session");
        assert_eq!(corpus["family"]["frameType"], ROUTER_BOOTSTRAP_FRAME_TYPE);
        assert_eq!(corpus["family"]["direction"], "routerToRuntime");
        assert_eq!(corpus["family"]["payloadPresence"], "empty");
        assert!(
            corpus["assemblyRefs"]
                .as_array()
                .expect("assemblyRefs")
                .iter()
                .any(|case| case["valid"].as_bool().expect("valid flag")),
            "corpus must contain a valid assembly ref case"
        );
        assert!(
            corpus["configSnapshotRefs"]
                .as_array()
                .expect("configSnapshotRefs")
                .iter()
                .any(|case| case["valid"].as_bool().expect("valid flag")),
            "corpus must contain a valid config snapshot ref case"
        );
    }

    #[test]
    fn runtime_consumer_decodes_and_roundtrips_bootstrap_frames_strictly() {
        let corpus = corpus();
        for case in corpus["frames"]
            .as_array()
            .expect("frames must be an array")
        {
            let id = case["id"].as_str().expect("frame id");
            let valid = case["valid"].as_bool().expect("valid flag");
            let result = decode_router_bootstrap_frame_header(case["json"].clone());
            match (valid, result) {
                (true, Ok(header)) => {
                    assert_eq!(header.envelope_type, ROUTER_BOOTSTRAP_FRAME_TYPE);
                    assert_eq!(header.activation.profile, "prod");

                    let frame = encode_router_bootstrap_frame(&header)
                        .unwrap_or_else(|error| panic!("{id} must encode: {error}"));
                    let decoded = decode_router_bootstrap_frame(&frame)
                        .unwrap_or_else(|error| panic!("{id} must decode: {error}"));
                    assert_eq!(decoded, header, "{id} must roundtrip exactly");
                }
                (false, Err(error)) => {
                    let message = error.to_string();
                    if let Some(expected) = case["expectErrorContains"].as_str() {
                        assert!(
                            message.contains(expected),
                            "{id} must fail with {expected:?}, got {message}"
                        );
                    }
                }
                (true, Err(error)) => panic!("{id} must decode, got {error}"),
                (false, Ok(_)) => panic!("{id} must be rejected"),
            }
        }
    }

    #[test]
    fn runtime_consumer_enforces_empty_bootstrap_payload() {
        let corpus = corpus();
        let canonical = corpus["frames"]
            .as_array()
            .expect("frames must be an array")
            .iter()
            .find(|case| case["valid"].as_bool() == Some(true))
            .expect("corpus must contain a canonical frame");
        let header: RouterBootstrapFrameHeader = serde_json::from_value(canonical["json"].clone())
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

        for case in corpus["payloadPresence"]
            .as_array()
            .expect("payloadPresence must be an array")
        {
            assert!(case["expectReject"].as_bool().expect("expectReject"));
            assert_eq!(case["enforcedBy"], "W-model-bootstrap-wire");
            assert!(
                !case["note"].as_str().expect("note").is_empty(),
                "{} must carry a rationale",
                case["id"]
            );
            assert!(
                case["currentEnforced"].as_bool().expect("currentEnforced"),
                "{} must be enforced",
                case["id"]
            );
        }
    }

    #[test]
    fn runtime_consumer_builds_bootstrap_from_captured_epoch_source() {
        let corpus = corpus();
        let canonical = corpus["frames"]
            .as_array()
            .expect("frames must be an array")
            .iter()
            .find(|case| case["valid"].as_bool() == Some(true))
            .expect("corpus must contain a canonical frame");
        let header: RouterBootstrapFrameHeader = serde_json::from_value(canonical["json"].clone())
            .expect("canonical frame must deserialize");

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
    }
}

//! A1 actor routing projection producer corpus.
//!
//! Freezes the source-free typed producer input boundary and the exact
//! `ActorRoutingProjection` output for valid and invalid cases. The corpus
//! JSON carries only framed identity strings; File IR coordinates, source and
//! executable payload facts never enter the producer boundary.

use serde::Deserialize;
use serde_json::Value;
use skiff_deployment::projection::actor_routing::{
    project_actor_routing, ActorRoutingProducerInput, ActorRoutingProjection,
    ActorRoutingProjectionError,
};

const CORPUS_SCHEMA_VERSION: &str = "skiff-actor-routing-a1-corpus-v1";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    schema_version: String,
    cases: Vec<CorpusCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CorpusCase {
    id: String,
    input: Value,
    expect: CorpusExpect,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CorpusExpect {
    valid: bool,
    projection: Option<Value>,
    error: Option<String>,
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "fixtures/a1-actor-routing-producer-corpus.json"
    ))
    .expect("a1 producer corpus must decode")
}

fn error_kind(error: &ActorRoutingProjectionError) -> &'static str {
    match error {
        ActorRoutingProjectionError::UnsupportedSchemaVersion(_) => "UnsupportedSchemaVersion",
        ActorRoutingProjectionError::ProducerUnsupportedSchemaVersion(_) => {
            "ProducerUnsupportedSchemaVersion"
        }
        ActorRoutingProjectionError::InvalidIdentity { .. } => "InvalidIdentity",
        ActorRoutingProjectionError::ServiceIdMismatch => "ServiceIdMismatch",
        ActorRoutingProjectionError::DuplicateMethod => "DuplicateMethod",
        ActorRoutingProjectionError::ProducerDuplicateActor => "ProducerDuplicateActor",
        ActorRoutingProjectionError::ProducerActorWithoutMethods => "ProducerActorWithoutMethods",
        ActorRoutingProjectionError::ProducerDuplicateActorMethod => "ProducerDuplicateActorMethod",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_is_frozen() {
        assert_eq!(corpus().schema_version, CORPUS_SCHEMA_VERSION);
    }

    #[test]
    fn corpus_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for case in corpus().cases {
            assert!(
                seen.insert(case.id.clone()),
                "duplicate corpus case id {}",
                case.id
            );
        }
    }

    #[test]
    fn corpus_expectations_are_consistent() {
        for case in corpus().cases {
            if case.expect.valid {
                assert!(
                    case.expect.projection.is_some() && case.expect.error.is_none(),
                    "case {}: valid case must carry projection and no error",
                    case.id
                );
            } else {
                assert!(
                    case.expect.projection.is_none() && case.expect.error.is_some(),
                    "case {}: invalid case must carry error and no projection",
                    case.id
                );
            }
        }
    }

    #[test]
    fn corpus_cases_match_producer_behavior() {
        for case in corpus().cases {
            match serde_json::from_value::<ActorRoutingProducerInput>(case.input.clone()) {
                Ok(input) => match project_actor_routing(input) {
                    Ok(projection) => {
                        assert!(
                            case.expect.valid,
                            "case {}: expected failure but producer accepted the input",
                            case.id
                        );
                        let actual =
                            serde_json::to_value(&projection).expect("projection serializes");
                        let expected = case
                            .expect
                            .projection
                            .expect("case {}: expected projection is missing");
                        assert_eq!(
                            actual, expected,
                            "case {}: projected output does not match the frozen corpus",
                            case.id
                        );
                    }
                    Err(error) => {
                        assert!(
                            !case.expect.valid,
                            "case {}: expected success but producer rejected the input",
                            case.id
                        );
                        let expected = case.expect.error.as_deref().expect("expected error kind");
                        assert_eq!(
                            error_kind(&error),
                            expected,
                            "case {}: error kind does not match the frozen corpus",
                            case.id
                        );
                    }
                },
                Err(error) => {
                    assert!(
                        !case.expect.valid,
                        "case {}: expected success but the producer input did not deserialize",
                        case.id
                    );
                    assert_eq!(
                        case.expect.error.as_deref(),
                        Some("Deserialize"),
                        "case {}: expected a deserialize failure",
                        case.id
                    );
                    assert!(
                        error.to_string().contains("unknown field"),
                        "case {}: deserialize failure is not an unknown-field rejection: {error}",
                        case.id
                    );
                }
            }
        }
    }

    #[test]
    fn projection_record_writer_roundtrips_and_replaces_canonical_current() {
        use std::{
            fs,
            time::{SystemTime, UNIX_EPOCH},
        };

        use skiff_canonical_json::canonical_json_bytes;
        use skiff_deployment::{
            projection::actor_routing::ACTOR_ROUTING_PROJECTION_RECORD_PATH,
            storage::CanonicalArtifactStore,
        };

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-a1-actor-routing-record-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let store = CanonicalArtifactStore::create(&root).unwrap();

        let corpus = corpus();
        let first = corpus
            .cases
            .iter()
            .find(|case| case.id == "valid-multi-package-cross-package-triple")
            .expect("corpus must contain the multi-package valid case");
        let projection: ActorRoutingProjection = project_actor_routing(
            serde_json::from_value(first.input.clone())
                .expect("valid producer input must deserialize"),
        )
        .expect("valid producer input must project");
        let record_path = store
            .write_actor_routing_projection(&projection)
            .expect("write current projection");
        assert_eq!(
            record_path,
            store.root().join(ACTOR_ROUTING_PROJECTION_RECORD_PATH),
            "record path must be the canonical A1 producer output surface"
        );

        let bytes = fs::read(&record_path).unwrap();
        assert_eq!(
            canonical_json_bytes(&projection).unwrap(),
            bytes,
            "current record bytes must be canonical JSON"
        );
        let decoded: ActorRoutingProjection =
            serde_json::from_slice(&bytes).expect("strict typed decode");
        assert_eq!(decoded, projection);

        let replacement: ActorRoutingProjection = project_actor_routing(
            serde_json::from_value(
                corpus
                    .cases
                    .iter()
                    .find(|case| case.id == "valid-empty-assembly")
                    .expect("empty assembly case")
                    .input
                    .clone(),
            )
            .expect("empty assembly input must deserialize"),
        )
        .expect("empty assembly input must project");
        store
            .write_actor_routing_projection(&replacement)
            .expect("replace current projection");
        let replaced = fs::read(&record_path).unwrap();
        assert_eq!(
            canonical_json_bytes(&replacement).unwrap(),
            replaced,
            "a later publish must atomically replace the current record"
        );
        assert_ne!(replaced, bytes);

        fs::remove_dir_all(root).unwrap();
    }
}

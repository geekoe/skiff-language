//! A1 actor routing projection producer corpus.
//!
//! Freezes the source-free typed producer input boundary and the exact
//! `ActorRoutingProjection` output for valid and invalid cases. The corpus
//! JSON carries only framed identity strings; File IR coordinates, source and
//! executable payload facts never enter the producer boundary.

use serde::Deserialize;
use serde_json::Value;
use skiff_deployment::projection::actor_routing::{
    project_actor_routing, ActorRoutingProducerInput, ActorRoutingProjectionError,
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
}

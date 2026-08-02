//! Shared corpus consumer for the A0-frozen actor routing projection types.
//!
//! The A3 wave freezes a shared record corpus at
//! `deployment/tests/fixtures/a3-actor-routing/corpus.json` (schema
//! `skiff-router-rust-actor-routing-corpus-v1`): the A1 producer and the
//! Router Rust strict reader (A3) consume the same exact record bytes. This
//! test verifies the deployment typed boundary against that corpus, including
//! the source-free shape (File IR / source / executable payload fields are
//! rejected by `deny_unknown_fields`).

use std::fs;

use serde::Deserialize;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

const FIXTURES_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/a3-actor-routing"
);

const EXPECTED_KINDS: &[&str] = &[
    "ok",
    "failSchemaVersion",
    "failMalformed",
    "failNonCanonical",
    "failMissing",
    "failInvalid",
];

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    schema_version: String,
    records: Vec<RecordCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecordCase {
    name: String,
    expected: String,
    #[serde(default)]
    content: Option<String>,
}

fn corpus() -> Corpus {
    let bytes = fs::read(format!("{FIXTURES_DIR}/corpus.json")).expect("read corpus");
    serde_json::from_slice(&bytes).expect("corpus must match the frozen schema")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_cases_are_well_formed() {
        let corpus = corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-actor-routing-corpus-v1"
        );
        let names = corpus
            .records
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>();
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            names.len(),
            "corpus case names must be unique"
        );
        for case in &corpus.records {
            assert!(
                EXPECTED_KINDS.contains(&case.expected.as_str()),
                "{} has unknown expected kind {:?}",
                case.name,
                case.expected
            );
            if case.expected == "failMissing" {
                assert!(
                    case.content.is_none(),
                    "{} must declare no content",
                    case.name
                );
            } else {
                assert!(
                    case.content
                        .as_deref()
                        .is_some_and(|content| !content.is_empty()),
                    "{} must declare non-empty record content",
                    case.name
                );
            }
        }
    }

    #[test]
    fn positive_corpus_records_match_the_typed_canonical_boundary() {
        for case in corpus()
            .records
            .into_iter()
            .filter(|case| case.expected == "ok")
        {
            let bytes = case.content.expect("positive content").into_bytes();
            let projection: ActorRoutingProjection =
                serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                    panic!(
                        "{} must deserialize into ActorRoutingProjection: {error}",
                        case.name
                    )
                });
            assert_eq!(
                projection.schema_version, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
                "{}",
                case.name
            );
            assert!(
                projection.methods.windows(2).all(|pair| pair[0] <= pair[1]),
                "{} entries must be sorted by the full typed key",
                case.name
            );
            assert_eq!(
                projection
                    .methods
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len(),
                projection.methods.len(),
                "{} entries must be unique",
                case.name
            );
            assert_eq!(
                canonical_json_bytes(&projection).expect("canonical serialization"),
                bytes,
                "{} record bytes must be canonical",
                case.name
            );
        }
    }

    #[test]
    fn invalid_corpus_records_are_rejected_by_the_typed_boundary() {
        for case in corpus()
            .records
            .into_iter()
            .filter(|case| case.expected == "failInvalid")
        {
            let bytes = case.content.expect("invalid content").into_bytes();
            let error = serde_json::from_slice::<ActorRoutingProjection>(&bytes)
                .expect_err("invalid record must be rejected");
            let message = error.to_string();
            if case.name.starts_with("forbidden-") {
                assert!(
                    message.contains("unknown field"),
                    "{} must reject the forbidden field, got: {message}",
                    case.name
                );
            } else if case.name.starts_with("bad-") {
                assert!(
                    message.contains("invalid actor routing projection field"),
                    "{} must reject the malformed identity, got: {message}",
                    case.name
                );
            } else if case.name == "duplicate-entries" {
                assert!(
                    message.contains("duplicate method entries"),
                    "{} must reject duplicates, got: {message}",
                    case.name
                );
            } else if case.name == "service-id-mismatch" {
                assert!(
                    message.contains("must match its deployment serviceId"),
                    "{} must reject service id mismatch, got: {message}",
                    case.name
                );
            } else {
                panic!("{} is not a known failInvalid case", case.name);
            }
        }
    }

    #[test]
    fn schema_version_and_malformed_corpus_records_fail_closed() {
        let records = corpus().records;
        let schema_case = records
            .iter()
            .find(|case| case.name == "schema-version-mismatch")
            .expect("schema-version-mismatch case");
        let schema_error = serde_json::from_slice::<ActorRoutingProjection>(
            schema_case.content.as_deref().expect("content").as_bytes(),
        )
        .expect_err("schema version mismatch must be rejected");
        assert!(schema_error
            .to_string()
            .contains("unsupported actor routing projection schemaVersion"));

        for name in ["malformed-json", "duplicate-keys"] {
            let case = records
                .iter()
                .find(|case| case.name == name)
                .unwrap_or_else(|| panic!("{name} case"));
            assert!(
                serde_json::from_slice::<ActorRoutingProjection>(
                    case.content.as_deref().expect("content").as_bytes()
                )
                .is_err(),
                "{name} must be rejected by strict JSON / typed parsing"
            );
        }
    }

    #[test]
    fn non_canonical_corpus_records_parse_but_are_not_canonical_bytes() {
        let records = corpus().records;
        for case in records
            .iter()
            .filter(|case| case.expected == "failNonCanonical")
        {
            let bytes = case.content.as_deref().expect("content").as_bytes();
            let projection: ActorRoutingProjection = serde_json::from_slice(bytes)
                .unwrap_or_else(|error| panic!("{} must still parse: {error}", case.name));
            assert_ne!(
                canonical_json_bytes(&projection).expect("canonical serialization"),
                bytes,
                "{} must not be canonical record bytes",
                case.name
            );
        }

        let unsorted = records
            .iter()
            .find(|case| case.name == "unsorted-entries")
            .expect("unsorted-entries case");
        let projection: ActorRoutingProjection =
            serde_json::from_slice(unsorted.content.as_deref().expect("content").as_bytes())
                .expect("unsorted entries parse into the normalized projection");
        assert!(
            projection.methods.windows(2).all(|pair| pair[0] <= pair[1]),
            "construction must normalize entry order"
        );
    }
}

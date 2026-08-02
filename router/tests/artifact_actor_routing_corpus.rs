//! Golden corpus consumer for the A3 actor routing projection strict reader.
//!
//! Reads the shared corpus at
//! `deployment/tests/fixtures/a3-actor-routing/corpus.json` (schema
//! `skiff-router-rust-actor-routing-corpus-v1`), materializes each record's
//! exact content bytes into a real temporary artifact root and asserts the
//! fail-closed reader chain: escape-proof path resolution, bounded read,
//! strict JSON, exact schema version, typed validation and canonical bytes.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use skiff_artifact_identity::ArtifactRelativePath;
use skiff_canonical_json::canonical_json_bytes;
use skiff_router::artifact::{
    ActorRoutingProjection, ActorRoutingProjectionError, ActorRoutingProjectionRef,
    ActorRoutingProjectionStore, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};

const FIXTURES_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../deployment/tests/fixtures/a3-actor-routing"
);

const EXPECTED_KINDS: &[&str] = &[
    "ok",
    "failSchemaVersion",
    "failMalformed",
    "failNonCanonical",
    "failMissing",
    "failInvalid",
];

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-router-a3-artifact-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp artifact root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn corpus() -> Corpus {
    let bytes = fs::read(format!("{FIXTURES_DIR}/corpus.json")).expect("read corpus");
    serde_json::from_slice(&bytes).expect("corpus must match the frozen schema")
}

fn record_ref(name: &str) -> ActorRoutingProjectionRef {
    ActorRoutingProjectionRef::new(
        ArtifactRelativePath::new(format!("records/{name}.json"), "corpus record")
            .expect("corpus record path must be relative and safe"),
    )
}

fn materialize(root: &Path, name: &str, content: &str) {
    let directory = root.join("records");
    fs::create_dir_all(&directory).expect("create records directory");
    fs::write(directory.join(format!("{name}.json")), content).expect("write corpus record");
}

fn load_case(case: &RecordCase) -> Result<ActorRoutingProjection, ActorRoutingProjectionError> {
    let root = TestRoot::new();
    if let Some(content) = &case.content {
        materialize(root.path(), &case.name, content);
    }
    let store = ActorRoutingProjectionStore::open(root.path()).expect("open temp store");
    store
        .load(&record_ref(&case.name))
        .map(|projection| (*projection).clone())
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
    fn all_positive_corpus_records_load_as_valid_canonical_projections() {
        for case in corpus()
            .records
            .into_iter()
            .filter(|case| case.expected == "ok")
        {
            let projection =
                load_case(&case).unwrap_or_else(|error| panic!("{} must load: {error}", case.name));
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
            let unique = projection
                .methods
                .iter()
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                unique.len(),
                projection.methods.len(),
                "{} entries must be unique",
                case.name
            );
        }
    }

    #[test]
    fn all_negative_corpus_records_fail_closed_with_frozen_error_kind() {
        for case in corpus()
            .records
            .into_iter()
            .filter(|case| case.expected != "ok")
        {
            let error = load_case(&case)
                .err()
                .unwrap_or_else(|| panic!("{} must fail closed", case.name));
            let matches = match case.expected.as_str() {
                "failSchemaVersion" => {
                    matches!(
                        error,
                        ActorRoutingProjectionError::SchemaVersionMismatch { .. }
                    )
                }
                "failMalformed" => matches!(error, ActorRoutingProjectionError::Malformed { .. }),
                "failNonCanonical" => {
                    matches!(error, ActorRoutingProjectionError::NonCanonical { .. })
                }
                "failMissing" => matches!(error, ActorRoutingProjectionError::MissingRecord { .. }),
                "failInvalid" => {
                    matches!(error, ActorRoutingProjectionError::InvalidProjection { .. })
                }
                kind => panic!("{}: unexpected expected kind {kind}", case.name),
            };
            assert!(matches, "{} failed with {error}", case.name);
        }
    }

    #[test]
    fn canonical_record_written_by_canonical_serializer_roundtrips() {
        let root = TestRoot::new();
        let case = corpus()
            .records
            .into_iter()
            .find(|case| case.name == "single-entry")
            .expect("single-entry case");
        let content = case.content.expect("single-entry content");
        let projection: ActorRoutingProjection =
            serde_json::from_slice(content.as_bytes()).expect("typed single-entry projection");
        let canonical = canonical_json_bytes(&projection).expect("canonical serialization");
        materialize(
            root.path(),
            "roundtrip",
            &String::from_utf8(canonical.clone()).expect("utf8"),
        );

        let store = ActorRoutingProjectionStore::open(root.path()).expect("open temp store");
        let loaded = store
            .load(&record_ref("roundtrip"))
            .expect("canonical record must load");
        assert_eq!(&*loaded, &projection);
    }

    #[test]
    fn loader_builds_immutable_catalog_with_exact_lookup() {
        let root = TestRoot::new();
        let case = corpus()
            .records
            .into_iter()
            .find(|case| case.name == "multi-entry-sorted")
            .expect("multi-entry-sorted case");
        materialize(
            root.path(),
            &case.name,
            case.content.as_deref().expect("multi-entry content"),
        );
        let store = ActorRoutingProjectionStore::open(root.path()).expect("open temp store");
        let catalog = store
            .load_catalog(&record_ref(&case.name))
            .expect("load catalog");

        assert_eq!(catalog.len(), 3);
        assert!(!catalog.is_empty());
        assert_eq!(
            catalog.projection().schema_version,
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
        );
        for entry in catalog.entries() {
            assert!(catalog.contains(entry));
            assert_eq!(catalog.get(entry), Some(entry));
            assert_eq!(
                catalog.methods_for_actor(&entry.actor).count(),
                1,
                "each corpus entry has a distinct actor ref"
            );
        }
        let first = &catalog.entries()[0];
        let actors = catalog.actor_refs().collect::<Vec<_>>();
        assert_eq!(actors.len(), 3);
        assert_eq!(actors[0], &first.actor);
        assert!(
            actors.windows(2).all(|pair| pair[0] != pair[1]),
            "actor refs must be unique"
        );
    }

    #[test]
    fn oversized_record_fails_closed() {
        let root = TestRoot::new();
        let directory = root.path().join("records");
        fs::create_dir_all(&directory).expect("create records directory");
        fs::write(
            directory.join("too-large.json"),
            vec![b'x'; 16 * 1024 * 1024 + 1],
        )
        .expect("write oversized record");
        let store = ActorRoutingProjectionStore::open(root.path()).expect("open temp store");
        let error = store
            .load(&record_ref("too-large"))
            .expect_err("oversized record must fail closed");
        assert!(matches!(
            error,
            ActorRoutingProjectionError::RecordTooLarge { .. }
        ));
    }

    #[test]
    fn missing_record_and_escape_paths_fail_closed() {
        let root = TestRoot::new();
        let store = ActorRoutingProjectionStore::open(root.path()).expect("open temp store");
        let error = store
            .load(&record_ref("does-not-exist"))
            .expect_err("missing record must fail closed");
        assert!(matches!(
            error,
            ActorRoutingProjectionError::MissingRecord { .. }
        ));

        let escaped = ArtifactRelativePath::new("../escape.json", "escape probe");
        assert!(
            escaped.is_err(),
            "relative record paths must reject escape segments"
        );
        let absolute = ArtifactRelativePath::new("/etc/passwd", "absolute probe");
        assert!(
            absolute.is_err(),
            "relative record paths must reject absolute paths"
        );
    }

    #[test]
    fn open_rejects_missing_and_non_directory_roots() {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let missing = std::env::temp_dir().join(format!(
            "skiff-router-a3-no-such-root-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&missing);
        let error =
            ActorRoutingProjectionStore::open(&missing).expect_err("missing root must fail closed");
        assert!(matches!(
            error,
            ActorRoutingProjectionError::InvalidRoot { .. }
        ));

        let root = TestRoot::new();
        let file = root.path().join("not-a-directory");
        fs::write(&file, b"x").expect("write non-directory root probe");
        let error = ActorRoutingProjectionStore::open(&file)
            .expect_err("non-directory root must fail closed");
        assert!(matches!(
            error,
            ActorRoutingProjectionError::RootNotDirectory { .. }
        ));
    }
}

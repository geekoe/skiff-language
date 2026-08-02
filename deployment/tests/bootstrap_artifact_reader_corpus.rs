//! C-model-artifact corpus consumer (assembly side).
//!
//! Freezes the strict reader boundary of `CanonicalArtifactStore`:
//! `RuntimeAssemblyRef` validation and the full
//! path/declared-identity/computed-identity/canonical-bytes read chain.
//! Exercises real filesystem boundaries over a temporary artifact root.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use serde_json::{json, Value};
use skiff_artifact_identity::{runtime_assembly_ref, RuntimeAssemblyRecordPath};
use skiff_artifact_model::RuntimeAssemblyRef;
use skiff_deployment::{fixtures::empty_runtime_assembly_fixture, storage::CanonicalArtifactStore};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bootstrap-artifact-corpus-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temp root");
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

fn test_store() -> (TestRoot, CanonicalArtifactStore) {
    let root = TestRoot::new();
    let store = CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    (root, store)
}

fn record_path(store: &CanonicalArtifactStore, reference: &RuntimeAssemblyRef) -> PathBuf {
    store.root().join(
        RuntimeAssemblyRecordPath::new(reference)
            .expect("record path")
            .as_relative_path()
            .as_path(),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ArtifactCorpus {
    schema_version: String,
    assembly_refs: Vec<RefCase>,
    record_cases: Vec<RecordCase>,
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
struct RecordCase {
    id: String,
    kind: String,
    expect: String,
    #[serde(default)]
    expect_error_contains: Option<String>,
}

fn corpus() -> ArtifactCorpus {
    serde_json::from_str(include_str!("fixtures/bootstrap-artifact-corpus.json"))
        .expect("artifact corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_is_frozen() {
        let corpus = corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-bootstrap-artifact-corpus-v1"
        );
        assert!(!corpus.assembly_refs.is_empty());
        assert!(!corpus.record_cases.is_empty());
    }

    #[test]
    fn assembly_ref_cases_follow_strict_deserialization() {
        for case in corpus().assembly_refs {
            let result = serde_json::from_value::<RuntimeAssemblyRef>(case.json.clone());
            match (case.valid, result) {
                (true, Ok(_)) => {}
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
    fn strict_reader_accepts_valid_record_and_rejects_corruption_matrix() {
        let (_root, store) = test_store();
        let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
        let reference = runtime_assembly_ref(&assembly).expect("assembly ref");
        store
            .write_runtime_assembly(&assembly)
            .expect("write valid assembly record");

        for case in corpus().record_cases {
            let outcome = match case.kind.as_str() {
                "valid" => store.read_runtime_assembly(&reference),
                "missing" => {
                    let missing_ref = RuntimeAssemblyRef {
                        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(format!(
                            "skiff-runtime-assembly-v3:sha256:{}",
                            "b".repeat(64)
                        )),
                    };
                    store.read_runtime_assembly(&missing_ref)
                }
                "malformed" => {
                    fs::write(record_path(&store, &reference), b"{\"schemaVersion\":").unwrap();
                    store.read_runtime_assembly(&reference)
                }
                "identityMismatch" => {
                    let mismatched_ref = RuntimeAssemblyRef {
                        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(format!(
                            "skiff-runtime-assembly-v3:sha256:{}",
                            "c".repeat(64)
                        )),
                    };
                    let bytes = skiff_canonical_json::canonical_json_bytes(&assembly).unwrap();
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    value["assemblyIdentity"] = json!(mismatched_ref.assembly_identity.as_str());
                    fs::write(
                        record_path(&store, &mismatched_ref),
                        serde_json::to_vec(&value).unwrap(),
                    )
                    .unwrap();
                    store.read_runtime_assembly(&mismatched_ref)
                }
                "nonCanonical" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&assembly).unwrap();
                    let value: Value = serde_json::from_slice(&bytes).unwrap();
                    fs::write(
                        record_path(&store, &reference),
                        serde_json::to_vec_pretty(&value).unwrap(),
                    )
                    .unwrap();
                    store.read_runtime_assembly(&reference)
                }
                "unknownField" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&assembly).unwrap();
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    value["extra"] = json!(1);
                    fs::write(
                        record_path(&store, &reference),
                        serde_json::to_vec(&value).unwrap(),
                    )
                    .unwrap();
                    store.read_runtime_assembly(&reference)
                }
                "schemaVersionMismatch" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&assembly).unwrap();
                    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
                    value["schemaVersion"] = json!("skiff-runtime-assembly-v2");
                    fs::write(
                        record_path(&store, &reference),
                        serde_json::to_vec(&value).unwrap(),
                    )
                    .unwrap();
                    store.read_runtime_assembly(&reference)
                }
                other => panic!("unsupported record case kind {other}"),
            };
            match case.expect.as_str() {
                "ok" => {
                    let loaded = outcome
                        .unwrap_or_else(|error| panic!("{} must read, got {error}", case.id));
                    assert_eq!(loaded.assembly_identity, reference.assembly_identity);
                }
                "reject" => {
                    let error = outcome
                        .expect_err(&format!("{} must fail closed", case.id))
                        .to_string();
                    if let Some(expected) = &case.expect_error_contains {
                        assert!(
                            error.contains(expected),
                            "{} must fail with {expected:?}, got {error}",
                            case.id
                        );
                    }
                }
                other => panic!("unsupported expected outcome {other}"),
            }
            // Restore the valid record before the next corruption case. Records
            // are immutable, so the corrupted file must be removed first.
            if case.expect == "reject" {
                let path = record_path(&store, &reference);
                fs::remove_file(&path).expect("remove corrupted record");
                store
                    .write_runtime_assembly(&assembly)
                    .expect("restore valid assembly record");
            }
        }
    }
}

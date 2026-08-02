//! C-model-artifact corpus consumer (config snapshot side) and strict loader
//! half of C-bootstrap.
//!
//! Freezes `RuntimeConfigSnapshotRef` validation and the
//! `RuntimeConfigSnapshotStore` strict reader boundary (canonical JSON, size
//! limit, id/path mismatch, malformed/unknown-field rejection) over a real
//! temporary store.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use serde_json::{json, Value};
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId, RuntimeConfigSnapshotId,
    RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_runtime_config_snapshot::{
    RuntimeConfigDeployment, RuntimeConfigPackage, RuntimeConfigSnapshot,
    RuntimeConfigSnapshotStore, MAX_CONFIG_SNAPSHOT_BYTES, MAX_DEPLOYMENTS_PER_SNAPSHOT,
    MAX_PACKAGES_PER_DEPLOYMENT,
};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bootstrap-snapshot-corpus-{}-{sequence}",
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

fn deployment(service_id: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("dev"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            "a".repeat(64)
        )),
    }
}

fn fixed_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("fixed snapshot ref"),
    }
}

fn snapshot() -> RuntimeConfigSnapshot {
    RuntimeConfigSnapshot::new(
        "prod",
        fixed_ref(),
        vec![RuntimeConfigDeployment::new(
            deployment("example.gateway"),
            vec![RuntimeConfigPackage::new(
                PackageBuildId::new("build-a"),
                BTreeMap::from([("apiKey".to_string(), json!("secret"))]),
            )
            .expect("package")],
        )
        .expect("deployment")],
    )
    .expect("snapshot")
}

fn snapshot_path(root: &Path, reference: &RuntimeConfigSnapshotRef) -> PathBuf {
    root.join("snapshots")
        .join(format!("{}.json", reference.snapshot_id.random_suffix()))
}

fn write_snapshot_bytes(path: &Path, bytes: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, bytes).expect("write snapshot record");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .expect("snapshot record permissions");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotCorpus {
    schema_version: String,
    snapshot_refs: Vec<RefCase>,
    record_cases: Vec<RecordCase>,
    limits: LimitsContract,
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LimitsContract {
    max_snapshot_bytes: u64,
    max_deployments_per_snapshot: usize,
    max_packages_per_deployment: usize,
}

fn corpus() -> SnapshotCorpus {
    serde_json::from_str(include_str!("fixtures/bootstrap-snapshot-corpus.json"))
        .expect("snapshot corpus must decode")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_limits_are_frozen() {
        let corpus = corpus();
        assert_eq!(
            corpus.schema_version,
            "skiff-router-rust-bootstrap-snapshot-corpus-v1"
        );
        assert_eq!(corpus.limits.max_snapshot_bytes, MAX_CONFIG_SNAPSHOT_BYTES);
        assert_eq!(
            corpus.limits.max_deployments_per_snapshot,
            MAX_DEPLOYMENTS_PER_SNAPSHOT
        );
        assert_eq!(
            corpus.limits.max_packages_per_deployment,
            MAX_PACKAGES_PER_DEPLOYMENT
        );
        assert!(!corpus.snapshot_refs.is_empty());
        assert!(!corpus.record_cases.is_empty());
    }

    #[test]
    fn snapshot_ref_cases_follow_strict_deserialization() {
        for case in corpus().snapshot_refs {
            let result = serde_json::from_value::<RuntimeConfigSnapshotRef>(case.json.clone());
            match (case.valid, result) {
                (true, Ok(reference)) => {
                    assert_eq!(
                        reference.snapshot_id.as_str(),
                        "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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
    fn strict_snapshot_reader_accepts_valid_record_and_rejects_corruption_matrix() {
        let corpus = corpus();
        let root = TestRoot::new();
        let store = RuntimeConfigSnapshotStore::create(root.path().join("store"))
            .expect("create snapshot store");
        let record = snapshot();
        store.publish(&record).expect("publish valid snapshot");

        for case in corpus.record_cases {
            let reference = fixed_ref();
            let outcome = match case.kind.as_str() {
                "valid" => store.read(&reference),
                "missing" => {
                    let other = RuntimeConfigSnapshotRef {
                        snapshot_id: RuntimeConfigSnapshotId::parse(
                            "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        )
                        .expect("other snapshot ref"),
                    };
                    store.read(&other)
                }
                "idMismatch" => {
                    let other = RuntimeConfigSnapshotRef {
                        snapshot_id: RuntimeConfigSnapshotId::parse(
                            "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                        )
                        .expect("other snapshot ref"),
                    };
                    let bytes = skiff_canonical_json::canonical_json_bytes(&record)
                        .expect("canonical snapshot bytes");
                    write_snapshot_bytes(&snapshot_path(store.root(), &other), &bytes);
                    store.read(&other)
                }
                "malformed" => {
                    let path = snapshot_path(store.root(), &reference);
                    write_snapshot_bytes(&path, b"{\"schemaVersion\":");
                    store.read(&reference)
                }
                "nonCanonical" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&record)
                        .expect("canonical snapshot bytes");
                    let value: Value = serde_json::from_slice(&bytes).expect("snapshot json");
                    write_snapshot_bytes(
                        &snapshot_path(store.root(), &reference),
                        &serde_json::to_vec_pretty(&value).expect("pretty bytes"),
                    );
                    store.read(&reference)
                }
                "schemaVersionMismatch" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&record)
                        .expect("canonical snapshot bytes");
                    let mut value: Value = serde_json::from_slice(&bytes).expect("snapshot json");
                    value["schemaVersion"] = json!("skiff-runtime-config-snapshot-record-v1");
                    write_snapshot_bytes(
                        &snapshot_path(store.root(), &reference),
                        &serde_json::to_vec(&value).expect("record bytes"),
                    );
                    store.read(&reference)
                }
                "unknownField" => {
                    let bytes = skiff_canonical_json::canonical_json_bytes(&record)
                        .expect("canonical snapshot bytes");
                    let mut value: Value = serde_json::from_slice(&bytes).expect("snapshot json");
                    value["extra"] = json!(1);
                    write_snapshot_bytes(
                        &snapshot_path(store.root(), &reference),
                        &serde_json::to_vec(&value).expect("record bytes"),
                    );
                    store.read(&reference)
                }
                other => panic!("unsupported record case kind {other}"),
            };

            match case.expect.as_str() {
                "ok" => {
                    let loaded = outcome
                        .unwrap_or_else(|error| panic!("{} must read, got {error}", case.id));
                    assert_eq!(loaded.snapshot_ref(), &reference);
                    assert_eq!(loaded.environment(), "prod");
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

            // Restore the valid record before the next corruption case.
            if case.expect == "reject" {
                let path = snapshot_path(store.root(), &reference);
                let _ = fs::remove_file(&path);
                store
                    .publish(&record)
                    .expect("restore valid snapshot record");
            }
        }
    }
}

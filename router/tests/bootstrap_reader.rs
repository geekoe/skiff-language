//! W-bootstrap reader tests: `CommittedActivationBootstrapReader` over the
//! repository read side, the frozen bootstrap-chain corpus, and the full
//! fail-closed negative matrix (missing / malformed / pending / identity
//! mismatch / repository failure).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;
use skiff_artifact_identity::runtime_assembly_ref;
use skiff_deployment::activation_state::{PrepareInput, ProfileActivationState};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::storage::{CanonicalArtifactStore, CommittedActivation};
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, ActivationStateRepository, RepositoryError,
};
use skiff_router::bootstrap::{
    BlockingLoader, BlockingLoaderOptions, BootstrapReadOutcome, CanonicalCommittedRefValidator,
    CommittedActivationBootstrapReader, CommittedBootstrapRefs,
};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-reader-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("create temp parent");
        Self {
            parent: parent.clone(),
            root: parent.join("root"),
        }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

fn snapshot_ref() -> skiff_artifact_model::RuntimeConfigSnapshotRef {
    skiff_artifact_model::RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

fn assembly_ref_hex(hex: &str) -> skiff_artifact_model::RuntimeAssemblyRef {
    skiff_artifact_model::RuntimeAssemblyRef {
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{hex}"
        )),
    }
}

fn committed(
    generation: u64,
    assembly: skiff_artifact_model::RuntimeAssemblyRef,
) -> CommittedActivation {
    CommittedActivation {
        generation,
        assembly,
        config_snapshot: snapshot_ref(),
    }
}

fn state_for(profile: &str, committed: CommittedActivation) -> ProfileActivationState {
    ProfileActivationState::initial(
        profile,
        committed.generation,
        committed.assembly,
        committed.config_snapshot,
    )
}

fn materialize_assembly(root: &Path) -> skiff_artifact_model::RuntimeAssemblyRef {
    let store = CanonicalArtifactStore::create(root).expect("create artifact store");
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    store
        .write_runtime_assembly(&assembly)
        .expect("write assembly record");
    runtime_assembly_ref(&assembly).expect("assembly ref")
}

fn reader(
    repository: Arc<dyn ActivationStateRepository>,
    root: &Path,
) -> CommittedActivationBootstrapReader {
    fs::create_dir_all(root).expect("create artifact root for validator");
    let validator = Arc::new(CanonicalCommittedRefValidator::open(root).expect("open validator"));
    let loader = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
    CommittedActivationBootstrapReader::new(repository, validator, loader)
}

fn outcome_name(outcome: &BootstrapReadOutcome) -> &'static str {
    match outcome {
        BootstrapReadOutcome::StableCommitted { .. } => "stableCommitted",
        BootstrapReadOutcome::CommittedWithPending { .. } => "committedWithPending",
        BootstrapReadOutcome::FailClosedMissing => "failClosedMissing",
        BootstrapReadOutcome::FailClosedMalformed { .. } => "failClosedMalformed",
        BootstrapReadOutcome::FailClosedIdentityMismatch { .. } => "failClosedIdentityMismatch",
        BootstrapReadOutcome::FailClosedRepository { .. } => "failClosedRepository",
    }
}

fn corpus() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../deployment/tests/fixtures/bootstrap-chain-corpus.json");
    serde_json::from_str(&fs::read_to_string(path).expect("read corpus"))
        .expect("corpus must parse")
}

/// Test double: canned `read` result for malformed/transient/closed cases.
struct CannedRepository {
    read_result: Mutex<Result<ProfileActivationState, RepositoryError>>,
}

#[async_trait]
impl ActivationStateRepository for CannedRepository {
    async fn read(&self, _profile: &str) -> Result<ProfileActivationState, RepositoryError> {
        self.read_result
            .lock()
            .expect("canned repository lock")
            .clone()
    }

    async fn initialize(
        &self,
        _state: &ProfileActivationState,
    ) -> Result<ProfileActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn prepare(
        &self,
        _input: PrepareInput,
    ) -> Result<ProfileActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn commit(
        &self,
        _input: skiff_router::activation::CommitInput,
    ) -> Result<ProfileActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn abort(
        &self,
        _input: skiff_router::activation::AbortInput,
    ) -> Result<ProfileActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn append_audit(
        &self,
        _event: &skiff_deployment::activation_state::ActivationAuditEvent,
    ) -> Result<(), RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn ensure_indexes(&self) -> Result<(), RepositoryError> {
        Ok(())
    }

    fn health(&self) -> skiff_router::activation::ActivationRepositoryHealth {
        Default::default()
    }

    async fn close(&self) -> Result<(), RepositoryError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn corpus_states_match_reader_outcomes() {
        let corpus = corpus();
        assert_eq!(
            corpus["schemaVersion"],
            "skiff-router-rust-bootstrap-chain-corpus-v1"
        );
        let states = corpus["states"].as_array().expect("states");
        assert_eq!(states.len(), 6);

        for case in states {
            let kind = case["kind"].as_str().expect("kind");
            let root = TestRoot::new();
            let repo: Arc<dyn ActivationStateRepository> = match kind {
                "committedOnly" => {
                    let assembly_ref = materialize_assembly(root.path());
                    let repo = MemoryActivationStateRepository::new();
                    repo.initialize(&state_for("prod", committed(1, assembly_ref)))
                        .await
                        .expect("initialize");
                    Arc::new(repo)
                }
                "pendingPresent" => {
                    // Plan §4.2: the committed epoch must be constructible
                    // first, so the pending case uses a real materialized
                    // assembly (the contracts-bootstrap corpus label is the
                    // legacy E-bootstrap fail-closed name; see below).
                    let assembly_ref = materialize_assembly(root.path());
                    let repo = MemoryActivationStateRepository::new();
                    repo.initialize(&state_for("prod", committed(1, assembly_ref.clone())))
                        .await
                        .expect("initialize");
                    repo.prepare(PrepareInput {
                        profile: "prod".to_string(),
                        activation_id: "act-1".to_string(),
                        expected_generation: 1,
                        candidate_generation: 2,
                        assembly: assembly_ref_hex(&"b".repeat(64)),
                        config_snapshot: snapshot_ref(),
                        participant_replica_ids: vec!["r1".to_string()],
                    })
                    .await
                    .expect("prepare pending");
                    Arc::new(repo)
                }
                "missing" => Arc::new(MemoryActivationStateRepository::new()),
                "malformed" => Arc::new(CannedRepository {
                    read_result: Mutex::new(Err(RepositoryError::InvalidRecord {
                        profile: "prod".to_string(),
                        message: "schemaVersion mismatch".to_string(),
                    })),
                }),
                "committedRefMissing" => {
                    let repo = MemoryActivationStateRepository::new();
                    repo.initialize(&state_for(
                        "prod",
                        committed(1, assembly_ref_hex(&"a".repeat(64))),
                    ))
                    .await
                    .expect("initialize");
                    Arc::new(repo)
                }
                "committedRefMismatch" => {
                    let written = materialize_assembly(root.path());
                    let repo = MemoryActivationStateRepository::new();
                    let committed_ref = if written.assembly_identity.as_str()
                        == format!("skiff-runtime-assembly-v3:sha256:{}", "a".repeat(64))
                    {
                        assembly_ref_hex(&"b".repeat(64))
                    } else {
                        assembly_ref_hex(&"a".repeat(64))
                    };
                    repo.initialize(&state_for("prod", committed(1, committed_ref)))
                        .await
                        .expect("initialize");
                    Arc::new(repo)
                }
                other => panic!("unexpected corpus kind {other}"),
            };
            let outcome = reader(repo, root.path()).read_committed("prod").await;
            // E-activation (plan §4.2) changed the pending semantics to
            // committed-published + recovery-surfaced; the deployment-owned
            // corpus labels that state `recoverPending`.
            let expected = match case["bootstrapOutcome"].as_str().expect("expected outcome") {
                "recoverPending" => "committedWithPending",
                outcome => outcome,
            };
            assert_eq!(outcome_name(&outcome), expected, "{}", case["id"]);
            match kind {
                "committedOnly" => assert!(outcome.is_stable(), "committedOnly must be stable"),
                "pendingPresent" => assert!(matches!(
                    outcome,
                    BootstrapReadOutcome::CommittedWithPending { pending, .. } if pending.activation_id == "act-1"
                )),
                "missing" => assert_eq!(outcome, BootstrapReadOutcome::FailClosedMissing),
                "malformed" => assert!(matches!(
                    outcome,
                    BootstrapReadOutcome::FailClosedMalformed { .. }
                )),
                "committedRefMissing" | "committedRefMismatch" => assert!(matches!(
                    outcome,
                    BootstrapReadOutcome::FailClosedIdentityMismatch { .. }
                )),
                _ => unreachable!(),
            }
        }
    }

    #[tokio::test]
    async fn corpus_freeze_defines_projection_and_epoch_contract() {
        let corpus = corpus();
        let projection = &corpus["projection"];
        assert_eq!(
        projection["output"],
        "CommittedBootstrapRefs { generation, assembly: RuntimeAssemblyRef, config_snapshot: RuntimeConfigSnapshotRef }"
    );
        assert_eq!(projection["pending"], "no projection");
        let fields = corpus["epoch"]["fields"].as_array().expect("epoch fields");
        let names = fields
            .iter()
            .map(|field| field.as_str().expect("field name"))
            .collect::<Vec<_>>();
        assert!(names.contains(&"profile"));
        assert!(names.contains(&"assemblyGeneration"));
        assert!(names.contains(&"assemblyIdentity"));
        assert!(names.contains(&"configSnapshotId"));
        assert!(names.contains(&"immutableIngressDeploymentActorProjection"));
        assert_eq!(corpus["epoch"]["publication"], "atomic-arc-replacement");
        assert_eq!(corpus["epoch"]["singleAuthority"], true);
        assert_eq!(corpus["epoch"]["pendingNeverEnters"], true);
    }

    #[tokio::test]
    async fn projection_maps_committed_activation_and_never_projects_pending() {
        let root = TestRoot::new();
        let assembly_ref = materialize_assembly(root.path());
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", committed(3, assembly_ref.clone())))
            .await
            .expect("initialize");
        repo.prepare(PrepareInput {
            profile: "prod".to_string(),
            activation_id: "act-pending".to_string(),
            expected_generation: 3,
            candidate_generation: 4,
            assembly: assembly_ref_hex(&"b".repeat(64)),
            config_snapshot: snapshot_ref(),
            participant_replica_ids: vec!["r1".to_string()],
        })
        .await
        .expect("prepare pending");

        let outcome = reader(Arc::new(repo), root.path())
            .read_committed("prod")
            .await;
        assert!(matches!(
            outcome,
            BootstrapReadOutcome::CommittedWithPending { .. }
        ));
        assert!(
            outcome.refs().is_some(),
            "pending state must still project the committed refs (§4.2)"
        );

        let refs = CommittedBootstrapRefs::project_committed(&committed(3, assembly_ref.clone()));
        assert_eq!(refs.generation, 3);
        assert_eq!(refs.assembly, assembly_ref);
    }

    #[tokio::test]
    async fn transient_and_closed_repository_failures_fail_closed() {
        let root = TestRoot::new();
        let transient = reader(
            Arc::new(CannedRepository {
                read_result: Mutex::new(Err(RepositoryError::Transient {
                    message: "driver reconnect".to_string(),
                })),
            }),
            root.path(),
        );
        assert!(matches!(
            transient.read_committed("prod").await,
            BootstrapReadOutcome::FailClosedRepository { .. }
        ));

        let closed = reader(
            Arc::new(CannedRepository {
                read_result: Mutex::new(Err(RepositoryError::Closed)),
            }),
            root.path(),
        );
        assert!(matches!(
            closed.read_committed("prod").await,
            BootstrapReadOutcome::FailClosedRepository { .. }
        ));
    }

    #[tokio::test]
    async fn reader_fail_closed_counters_track_negative_matrix() {
        let root = TestRoot::new();
        let repo = MemoryActivationStateRepository::new();
        let reader = reader(Arc::new(repo), root.path());
        assert_eq!(
            reader.read_committed("prod").await,
            BootstrapReadOutcome::FailClosedMissing
        );
        let counters = reader.fail_closed();
        assert_eq!(counters.missing, 1);
        assert_eq!(counters.malformed, 0);
        assert_eq!(counters.identity_mismatch, 0);
        assert_eq!(counters.pending, 0);
    }
}

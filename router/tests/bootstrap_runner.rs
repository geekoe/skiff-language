//! W-bootstrap initial-runner tests: read → project → strict load → publish
//! success chain plus the fail-closed negative matrix (pending / missing /
//! malformed / identity mismatch / loader saturation / shutdown), each with
//! zero epoch publication.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};
use std::time::Duration;

use async_trait::async_trait;
use skiff_artifact_identity::{runtime_assembly_ref, ArtifactRelativePath};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::EnvironmentActivationState;
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, AbortInput, ActivationRepositoryHealth,
    ActivationStateRepository, CommitInput, PrepareInput, RepositoryError,
};
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::{
    BlockingLoader, BlockingLoaderOptions, BootstrapError, BootstrapRunner, BootstrapStrictLoader,
    CanonicalCommittedRefValidator, CommittedActivationBootstrapReader,
};
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-bootstrap-runner-{}-{sequence}",
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

fn state_for(
    environment: &str,
    generation: u64,
    assembly: skiff_artifact_model::RuntimeAssemblyRef,
) -> skiff_deployment::activation_state::EnvironmentActivationState {
    skiff_deployment::activation_state::EnvironmentActivationState::initial(
        environment,
        generation,
        assembly,
        snapshot_ref(),
    )
}

struct RealChain {
    _root: TestRoot,
    strict: Arc<BootstrapStrictLoader>,
    actor_ref: ActorRoutingProjectionRef,
    assembly_ref: skiff_artifact_model::RuntimeAssemblyRef,
}

fn materialize(environment: &str) -> RealChain {
    let root = TestRoot::new();
    let snapshot_store =
        RuntimeConfigSnapshotStore::create(root.path()).expect("create snapshot store");
    let snapshot = RuntimeConfigSnapshot::new(environment, snapshot_ref(), Vec::new())
        .expect("snapshot fixture");
    snapshot_store.publish(&snapshot).expect("publish snapshot");
    let artifact_store =
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    artifact_store
        .write_runtime_assembly(&assembly)
        .expect("write assembly");
    let assembly_ref = runtime_assembly_ref(&assembly).expect("assembly ref");

    let directory = root.path().join("records/actor-routing");
    fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
    fs::write(directory.join("empty.json"), bytes).expect("write projection record");
    let actor_ref = ActorRoutingProjectionRef::new(
        ArtifactRelativePath::new("records/actor-routing/empty.json", "test record")
            .expect("record path"),
    );
    let strict = Arc::new(
        BootstrapStrictLoader::open(root.path(), root.path()).expect("open strict loader"),
    );
    RealChain {
        _root: root,
        strict,
        actor_ref,
        assembly_ref,
    }
}

fn runner(
    repository: Arc<dyn ActivationStateRepository>,
    chain: &RealChain,
    pool: Arc<BlockingLoader>,
) -> BootstrapRunner {
    let validator = Arc::new(
        CanonicalCommittedRefValidator::open(chain.strict.artifact_root()).expect("validator"),
    );
    let reader = CommittedActivationBootstrapReader::new(repository, validator, Arc::clone(&pool));
    BootstrapRunner::new(
        reader,
        Arc::clone(&chain.strict),
        pool,
        Arc::new(skiff_router::bootstrap::ActiveRoutingEpochStore::new()),
    )
}

fn default_pool() -> Arc<BlockingLoader> {
    Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()))
}

struct TestCannedRepository {
    read_result: std::sync::Mutex<Result<EnvironmentActivationState, RepositoryError>>,
}

#[async_trait]
impl ActivationStateRepository for TestCannedRepository {
    async fn read(
        &self,
        _environment: &str,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        self.read_result
            .lock()
            .expect("canned repository lock")
            .clone()
    }

    async fn initialize(
        &self,
        _state: &EnvironmentActivationState,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn prepare(
        &self,
        _input: PrepareInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn commit(
        &self,
        _input: CommitInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn abort(
        &self,
        _input: AbortInput,
    ) -> Result<EnvironmentActivationState, RepositoryError> {
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

    fn health(&self) -> ActivationRepositoryHealth {
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
    async fn committed_only_publishes_complete_epoch() {
        let chain = materialize("prod");
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", 7, chain.assembly_ref.clone()))
            .await
            .expect("initialize");
        let runner = runner(Arc::new(repo), &chain, default_pool());

        let outcome = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect("initial bootstrap must publish");
        let epoch = outcome.epoch;
        assert!(
            outcome.pending.is_none(),
            "committed-only state has no pending"
        );
        assert_eq!(epoch.environment(), "prod");
        assert_eq!(epoch.assembly_generation(), 7);
        assert_eq!(
            epoch.assembly_identity(),
            chain.assembly_ref.assembly_identity.as_str()
        );
        let captured = runner.epoch_store().capture().expect("published epoch");
        assert_eq!(captured.assembly_generation(), 7);
        assert_eq!(runner.epoch_store().publish_count(), 1);
        let health = runner.health();
        assert_eq!(health.epoch_store_publish_count, 1);
        assert_eq!(
            health
                .active_epoch
                .expect("active epoch")
                .assembly_generation,
            7
        );
    }

    #[tokio::test]
    async fn pending_publishes_committed_and_surfaces_recovery() {
        let chain = materialize("prod");
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", 7, chain.assembly_ref.clone()))
            .await
            .expect("initialize");
        repo.prepare(skiff_router::activation::PrepareInput {
            environment: "prod".to_string(),
            activation_id: "act-1".to_string(),
            expected_generation: 7,
            candidate_generation: 8,
            assembly: assembly_ref_hex(&"b".repeat(64)),
            config_snapshot: snapshot_ref(),
            participant_replica_ids: vec!["r1".to_string()],
        })
        .await
        .expect("prepare pending");
        let runner = runner(Arc::new(repo), &chain, default_pool());

        let outcome = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect("pending must publish the committed epoch");
        assert_eq!(outcome.epoch.assembly_generation(), 7);
        let pending = outcome.pending.expect("pending recovery must be surfaced");
        assert_eq!(pending.activation_id, "act-1");
        assert_eq!(pending.expected_generation, 7);
        assert_eq!(pending.candidate_generation, 8);
        assert_eq!(
            runner
                .epoch_store()
                .capture()
                .expect("published epoch")
                .assembly_generation(),
            7
        );
        assert_eq!(runner.epoch_store().publish_count(), 1);
        assert_eq!(runner.reader().fail_closed().pending, 1);
    }

    #[tokio::test]
    async fn missing_fails_closed_without_publishing() {
        let chain = materialize("prod");
        let runner = runner(
            Arc::new(MemoryActivationStateRepository::new()),
            &chain,
            default_pool(),
        );
        let error = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect_err("missing must fail closed");
        assert!(matches!(
            error,
            BootstrapError::Read(skiff_router::bootstrap::BootstrapReadOutcome::FailClosedMissing)
        ));
        assert_eq!(runner.epoch_store().capture(), None);
        assert_eq!(runner.epoch_store().publish_count(), 0);
    }

    #[tokio::test]
    async fn malformed_fails_closed_without_publishing() {
        let chain = materialize("prod");
        let canned = TestCannedRepository {
            read_result: std::sync::Mutex::new(Err(RepositoryError::InvalidRecord {
                environment: "prod".to_string(),
                message: "schemaVersion mismatch".to_string(),
            })),
        };
        let runner = runner(Arc::new(canned), &chain, default_pool());
        let error = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect_err("malformed must fail closed");
        assert!(matches!(
            error,
            BootstrapError::Read(
                skiff_router::bootstrap::BootstrapReadOutcome::FailClosedMalformed { .. }
            )
        ));
        assert_eq!(runner.epoch_store().capture(), None);
    }

    #[tokio::test]
    async fn identity_mismatch_fails_closed_without_publishing() {
        let chain = materialize("prod");
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", 7, assembly_ref_hex(&"a".repeat(64))))
            .await
            .expect("initialize");
        let runner = runner(Arc::new(repo), &chain, default_pool());
        let error = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect_err("identity mismatch must fail closed");
        assert!(matches!(
            error,
            BootstrapError::Read(
                skiff_router::bootstrap::BootstrapReadOutcome::FailClosedIdentityMismatch { .. }
            )
        ));
        assert_eq!(runner.epoch_store().capture(), None);
        assert_eq!(runner.reader().fail_closed().identity_mismatch, 1);
    }

    #[tokio::test]
    async fn loader_saturation_fails_closed_without_publishing() {
        let chain = materialize("prod");
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", 7, chain.assembly_ref.clone()))
            .await
            .expect("initialize");
        let pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions {
            concurrency: 1,
            read_deadline: Duration::from_secs(5),
            drain_deadline: Duration::from_secs(2),
        }));
        let barrier = Arc::new(Barrier::new(2));
        let holder_barrier = Arc::clone(&barrier);
        let holder = tokio::spawn({
            let pool = Arc::clone(&pool);
            async move {
                pool.run(move || {
                    holder_barrier.wait();
                    Ok::<(), String>(())
                })
                .await
            }
        });
        while pool.health().occupancy < 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let runner = runner(Arc::new(repo), &chain, Arc::clone(&pool));
        let error = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect_err("saturation must fail closed");
        assert!(
            matches!(
                error,
                BootstrapError::Read(
                    skiff_router::bootstrap::BootstrapReadOutcome::FailClosedRepository { ref message }
                ) if message.contains("saturated")
            ),
            "{error}"
        );
        assert_eq!(runner.epoch_store().capture(), None);
        barrier.wait();
        let holder_result = tokio::time::timeout(Duration::from_secs(5), holder)
            .await
            .expect("holder must finish after barrier release");
        assert!(holder_result.expect("holder join").is_ok());
    }

    #[tokio::test]
    async fn shutdown_fails_closed_without_publishing() {
        let chain = materialize("prod");
        let repo = MemoryActivationStateRepository::new();
        repo.initialize(&state_for("prod", 7, chain.assembly_ref.clone()))
            .await
            .expect("initialize");
        let pool = default_pool();
        pool.shutdown().await;
        let runner = runner(Arc::new(repo), &chain, pool);
        let error = runner
            .run_initial("prod", &chain.actor_ref)
            .await
            .expect_err("shutdown must fail closed");
        assert!(
            matches!(
                error,
                BootstrapError::Read(
                    skiff_router::bootstrap::BootstrapReadOutcome::FailClosedRepository { ref message }
                ) if message.contains("shut down")
            ),
            "{error}"
        );
        assert_eq!(runner.epoch_store().capture(), None);
    }
}

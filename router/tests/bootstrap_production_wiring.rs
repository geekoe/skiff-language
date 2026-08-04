//! E-bootstrap production wiring tests: `RouterBootstrapAssembly` over a real
//! artifact root with the memory repository fake, plus the fail-closed matrix
//! and the `SessionLayer` epoch-source seam. The real Mongo/process boundary
//! is covered by the ignored `bootstrap_live_probe` harness.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use skiff_artifact_identity::{runtime_assembly_ref, ArtifactRelativePath};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::ProfileActivationState;
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, AbortInput, ActivationRepositoryHealth,
    ActivationStateRepository, CommitInput, PrepareInput, RepositoryError,
};
use skiff_router::bootstrap::{
    BootstrapAssemblyError, RouterBootstrapAssembly, ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};
use skiff_router::config::RouterConfig;
use skiff_router::session::SessionLayer;
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
            "skiff-router-e-bootstrap-wiring-{}-{sequence}",
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

struct RealChain {
    _root: TestRoot,
    assembly_ref: skiff_artifact_model::RuntimeAssemblyRef,
}

fn materialize(profile: &str) -> RealChain {
    let root = TestRoot::new();
    fs::create_dir_all(root.path()).expect("create artifact root");
    let snapshot_store = RuntimeConfigSnapshotStore::create(root.path().join("runtime-config"))
        .expect("create snapshot store");
    let snapshot =
        RuntimeConfigSnapshot::new(profile, snapshot_ref(), Vec::new()).expect("snapshot fixture");
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
    fs::write(
        root.path().join(ACTOR_ROUTING_PROJECTION_RECORD_PATH),
        bytes,
    )
    .expect("write projection record");
    RealChain {
        _root: root,
        assembly_ref,
    }
}

fn config(profile: &str, artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        activation_prepare_timeout_ms: 1_000,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: profile.to_string(),
        release_mode: Some(true),
        request_timeout_ms: 1_000,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff".to_string(),
        },
        telemetry: None,
        websocket_path: "/runtime".to_string(),
    }
}

fn committed_state(chain: &RealChain) -> ProfileActivationState {
    ProfileActivationState::initial("prod", 7, chain.assembly_ref.clone(), snapshot_ref())
}

/// Canned repository for outcomes the memory fake cannot hold (malformed).
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

    async fn commit(&self, _input: CommitInput) -> Result<ProfileActivationState, RepositoryError> {
        unimplemented!("canned repository is read-only")
    }

    async fn abort(&self, _input: AbortInput) -> Result<ProfileActivationState, RepositoryError> {
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
        ActivationRepositoryHealth::default()
    }

    async fn close(&self) -> Result<(), RepositoryError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn assembly_publishes_committed_epoch_and_wires_session_epoch_source() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        repository
            .initialize(&committed_state(&chain))
            .await
            .expect("seed committed state");
        let config = config("prod", chain._root.path());

        let assembly = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect("committed bootstrap must assemble");

        assert_eq!(assembly.profile(), "prod");
        assert_eq!(assembly.epoch().profile(), "prod");
        assert_eq!(assembly.epoch().assembly_generation(), 7);
        assert_eq!(
            assembly.epoch().assembly_identity(),
            chain.assembly_ref.assembly_identity.as_str()
        );
        assert_eq!(
            assembly.epoch().config_snapshot_id(),
            snapshot_ref().snapshot_id.as_str()
        );
        let health = assembly.health();
        assert_eq!(health.epoch_store_publish_count, 1);
        assert!(health.active_epoch.is_some());
        assert_eq!(health.loader.occupancy, 0);
        assert_eq!(health.loader.queued, 0);

        let session = SessionLayer::new(config.clone());
        session.attach_epoch_store(assembly.epoch_store());
        let bootstrap = session.bootstrap_bytes().expect("bootstrap bytes");
        let header = skiff_runtime_transport::protocol::decode_router_bootstrap_frame(&bootstrap)
            .expect("decode router.bootstrap");
        assert_eq!(header.envelope_type, "router.bootstrap");
        assert_eq!(header.activation.profile, "prod");
        assert_eq!(header.activation.generation, 7);
        assert_eq!(
            header.activation.assembly.assembly_identity,
            chain.assembly_ref.assembly_identity
        );
        assert_eq!(header.activation.config_snapshot, snapshot_ref());

        assembly.shutdown().await;
        assert!(assembly.health().loader.shutdown);
        assert_eq!(assembly.health().loader.occupancy, 0);
    }

    #[tokio::test]
    async fn assembly_fails_closed_on_missing_state() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        let config = config("prod", chain._root.path());
        let error = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect_err("missing activation state must fail closed");
        assert!(
            matches!(
                error,
                BootstrapAssemblyError::Bootstrap(skiff_router::bootstrap::BootstrapError::Read(
                    skiff_router::bootstrap::BootstrapReadOutcome::FailClosedMissing
                ))
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn assembly_publishes_committed_epoch_and_exposes_pending_recovery() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        repository
            .initialize(&committed_state(&chain))
            .await
            .expect("seed committed state");
        repository
            .prepare(PrepareInput {
                profile: "prod".to_string(),
                activation_id: "live-activation-1".to_string(),
                expected_generation: 7,
                candidate_generation: 8,
                assembly: chain.assembly_ref.clone(),
                config_snapshot: snapshot_ref(),
                participant_replica_ids: vec!["replica-1".to_string()],
            })
            .await
            .expect("prepare pending state");
        let config = config("prod", chain._root.path());
        let assembly = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect("pending activation must assemble with the committed epoch published");
        // §4.2(1): the committed epoch is published first; the pending record
        // is surfaced for the activation recovery transaction.
        assert_eq!(assembly.epoch().assembly_generation(), 7);
        assert_eq!(assembly.health().epoch_store_publish_count, 1);
        let pending = assembly
            .pending_recovery()
            .expect("pending recovery must be exposed by the assembly");
        assert_eq!(pending.activation_id, "live-activation-1");
        assert_eq!(pending.expected_generation, 7);
        assert_eq!(pending.candidate_generation, 8);
        assert_eq!(
            pending.participant_replica_ids,
            vec!["replica-1".to_string()]
        );
        assembly.shutdown().await;
    }

    #[tokio::test]
    async fn assembly_fails_closed_on_malformed_state() {
        let chain = materialize("prod");
        let repository = Arc::new(CannedRepository {
            read_result: Mutex::new(Err(RepositoryError::InvalidRecord {
                profile: "prod".to_string(),
                message: "state is not canonical".to_string(),
            })),
        });
        let config = config("prod", chain._root.path());
        let error = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            repository as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect_err("malformed activation state must fail closed");
        assert!(
            matches!(
                error,
                BootstrapAssemblyError::Bootstrap(skiff_router::bootstrap::BootstrapError::Read(
                    skiff_router::bootstrap::BootstrapReadOutcome::FailClosedMalformed { .. }
                ))
            ),
            "{error}"
        );
    }

    #[tokio::test]
    async fn assembly_fails_closed_on_identity_mismatch() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        let mut state = committed_state(&chain);
        state.committed.assembly = assembly_ref_hex(&"b".repeat(64));
        repository
            .initialize(&state)
            .await
            .expect("seed mismatched committed state");
        let config = config("prod", chain._root.path());
        let error = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect_err("committed ref mismatch must fail closed");
        assert!(
            matches!(
                error,
                BootstrapAssemblyError::Bootstrap(skiff_router::bootstrap::BootstrapError::Read(
                    skiff_router::bootstrap::BootstrapReadOutcome::FailClosedIdentityMismatch { .. }
                ))
            ),
            "{error}"
        );
    }

    #[test]
    fn actor_projection_record_path_is_escape_safe_and_canonical() {
        let reference = ArtifactRelativePath::new(
            ACTOR_ROUTING_PROJECTION_RECORD_PATH,
            "actor routing projection record",
        )
        .expect("record path");
        assert_eq!(reference.as_str(), ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    }
}

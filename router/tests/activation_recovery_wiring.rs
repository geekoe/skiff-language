//! Cold-recovery production wiring tests (plan §4.2, E-activation gate):
//! the durable pending is no longer a bootstrap fail-closed state, the
//! assembly surfaces it for the recovery transaction, the coordinator's
//! `RegistrationObserver` seam rebinds an expected replica's routable
//! session and commits, candidate-load failure durably aborts, and a
//! restart converges from the durable committed state alone.

use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use skiff_artifact_identity::runtime_assembly_ref;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::{
    ProfileActivationState, PROFILE_ACTIVATION_STATE_SCHEMA_VERSION,
};
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, CommittedActivation};
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, ActivationCoordinator, ActivationCoordinatorOptions,
    ActivationCoordinatorPorts, ActivationParticipantBinding, ActivationPhase,
    ActivationRevalidateOutcome, ActivationStateRepository, BlockingLoaderPort, CandidateEpochRefs,
    CandidateLoadError, CommitInput, EnqueueResult, EpochStorePublishPort, NoopHealthSink,
    PrepareInput, PublishCommittedEpochPort, RuntimeCandidateQueryPort, SessionEnqueuePort,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::{ActiveRoutingEpochStore, RouterBootstrapAssembly, RoutingEpoch};
use skiff_router::config::RouterConfig;
use skiff_router::routing::{DispatchCapabilities, RegisteredSessionLease, SessionCancellation};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::RegistrationObserver;
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};

const ASSEMBLY: &str =
    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT: &str = "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn assembly_ref(identity: &str) -> RuntimeAssemblyRef {
    RuntimeAssemblyRef {
        assembly_identity: AssemblyIdentity::new(identity),
    }
}

fn config_ref(id: &str) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(id).expect("snapshot id"),
    }
}

fn session(replica_id: &str, connection_generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica_id.to_string(),
        connection_generation,
    }
}

fn epoch(
    profile: &str,
    generation: u64,
    assembly_ref: RuntimeAssemblyRef,
    config_snapshot_ref: RuntimeConfigSnapshotRef,
) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_ref.assembly_identity.clone(),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let snapshot = RuntimeConfigSnapshot::new(profile, config_snapshot_ref, Vec::new())
        .expect("snapshot fixture");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            profile,
            generation,
            Arc::new(assembly),
            Arc::new(snapshot),
            catalog,
        )
        .expect("epoch fixture"),
    )
}

fn tuple(profile: &str, generation: u64) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: profile.to_string(),
        generation,
        assembly: assembly_ref(ASSEMBLY),
        config_snapshot: config_ref(SNAPSHOT),
    }
}

fn initial_state(profile: &str, generation: u64) -> ProfileActivationState {
    ProfileActivationState {
        schema_version: PROFILE_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
        profile: profile.to_string(),
        committed: CommittedActivation {
            generation,
            assembly: assembly_ref(ASSEMBLY),
            config_snapshot: config_ref(SNAPSHOT),
        },
        pending: None,
    }
}

fn prepare_input(profile: &str, activation_id: &str, expected: u64) -> PrepareInput {
    PrepareInput {
        profile: profile.to_string(),
        activation_id: activation_id.to_string(),
        expected_generation: expected,
        candidate_generation: expected + 1,
        assembly: assembly_ref(ASSEMBLY),
        config_snapshot: config_ref(SNAPSHOT),
        participant_replica_ids: vec!["runtime-a".to_string()],
    }
}

#[derive(Debug)]
struct OkLoader;

#[async_trait]
impl BlockingLoaderPort for OkLoader {
    async fn load_candidate(
        &self,
        refs: &CandidateEpochRefs,
    ) -> Result<Arc<RoutingEpoch>, CandidateLoadError> {
        Ok(epoch(
            &refs.profile,
            refs.generation,
            refs.assembly.clone(),
            refs.config_snapshot.clone(),
        ))
    }
}

/// Loader that fails only for the recovery candidate (committed loads stay
/// valid), used for the §4.2 candidate-load-failure durable abort case.
#[derive(Debug)]
struct CandidateFailingLoader {
    committed_generation: u64,
}

#[async_trait]
impl BlockingLoaderPort for CandidateFailingLoader {
    async fn load_candidate(
        &self,
        refs: &CandidateEpochRefs,
    ) -> Result<Arc<RoutingEpoch>, CandidateLoadError> {
        if refs.generation == self.committed_generation {
            return Ok(epoch(
                &refs.profile,
                refs.generation,
                refs.assembly.clone(),
                refs.config_snapshot.clone(),
            ));
        }
        Err(CandidateLoadError::Missing)
    }
}

#[derive(Debug)]
struct ScriptedCandidates {
    leases: StdMutex<Vec<RegisteredSessionLease>>,
}

impl ScriptedCandidates {
    fn new(tuple: RegisteredAssemblyTuple, replica_ids: &[(&str, u64)]) -> Self {
        let leases = replica_ids
            .iter()
            .map(|(replica_id, generation)| RegisteredSessionLease {
                session_epoch: session(replica_id, *generation),
                registration_revision: 1,
                exact_registered_tuple: tuple.clone(),
                cancellation: SessionCancellation { cancelled: false },
                capabilities: DispatchCapabilities {
                    unary: true,
                    server_stream: true,
                },
                registered_build_ids: Vec::new(),
                lazy_load: false,
                artifact_root: None,
            })
            .collect();
        Self {
            leases: StdMutex::new(leases),
        }
    }
}

impl RuntimeCandidateQueryPort for ScriptedCandidates {
    fn freeze(
        &self,
        _profile: &str,
    ) -> Result<Vec<RegisteredSessionLease>, skiff_router::activation::ActivationCandidateError>
    {
        Ok(self.leases.lock().expect("leases lock").clone())
    }

    fn revalidate(
        &self,
        _activation_id: &str,
        _frozen: &[ActivationParticipantBinding],
    ) -> ActivationRevalidateOutcome {
        ActivationRevalidateOutcome::Ok
    }
}

#[derive(Debug, Default)]
struct RecordingSessions {
    controls: StdMutex<Vec<(String, AssemblyActivationControl)>>,
}

impl SessionEnqueuePort for RecordingSessions {
    fn enqueue_prepare(
        &self,
        _binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        self.controls
            .lock()
            .expect("controls lock")
            .push(("prepare".to_string(), control.clone()));
        EnqueueResult::Ok
    }

    fn enqueue_commit(
        &self,
        _binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        self.controls
            .lock()
            .expect("controls lock")
            .push(("commit".to_string(), control.clone()));
        EnqueueResult::Ok
    }

    fn enqueue_abort(
        &self,
        _binding: &ActivationParticipantBinding,
        control: &AssemblyActivationControl,
    ) -> EnqueueResult {
        self.controls
            .lock()
            .expect("controls lock")
            .push(("abort".to_string(), control.clone()));
        EnqueueResult::Ok
    }

    fn abort_session(&self, _session: &RuntimeSessionEpoch) {}
}

fn prepared_control(prepare: &AssemblyActivationControl) -> AssemblyActivationControl {
    match prepare {
        AssemblyActivationControl::Prepare {
            profile,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        } => AssemblyActivationControl::Prepared {
            profile: profile.clone(),
            activation_id: activation_id.clone(),
            expected_generation: *expected_generation,
            candidate_generation: *candidate_generation,
            assembly: assembly.clone(),
            config_snapshot: config_snapshot.clone(),
            replica_id: replica_id.clone(),
        },
        other => panic!("expected Prepare control, got {other:?}"),
    }
}

struct Harness {
    repo: Arc<MemoryActivationStateRepository>,
    handle: skiff_router::activation::ActivationCoordinatorHandle,
    sessions: Arc<RecordingSessions>,
}

async fn harness(
    profile: &str,
    committed_generation: u64,
    loader: Arc<dyn BlockingLoaderPort>,
) -> Harness {
    let repo = Arc::new(MemoryActivationStateRepository::new());
    repo.initialize(&initial_state(profile, committed_generation))
        .await
        .expect("initialize committed");
    repo.prepare(prepare_input(profile, "recovery-8", committed_generation))
        .await
        .expect("prepare pending");
    let candidates = Arc::new(ScriptedCandidates::new(
        tuple(profile, committed_generation),
        &[("runtime-a", 1)],
    ));
    let sessions = Arc::new(RecordingSessions::default());
    let ports = ActivationCoordinatorPorts {
        repository: Arc::clone(&repo) as Arc<dyn ActivationStateRepository>,
        loader,
        candidates: Arc::clone(&candidates) as Arc<dyn RuntimeCandidateQueryPort>,
        sessions: Arc::clone(&sessions) as Arc<dyn SessionEnqueuePort>,
        publish: Arc::new(EpochStorePublishPort::new(Arc::new(
            ActiveRoutingEpochStore::new(),
        ))) as Arc<dyn PublishCommittedEpochPort>,
        health: Arc::new(NoopHealthSink),
    };
    let handle = ActivationCoordinator::spawn(
        ports,
        ActivationCoordinatorOptions {
            mailbox_capacity: 64,
            ack_deadline: Duration::from_secs(5),
            service_db_mongo_url: None,
        },
    );
    Harness {
        repo,
        handle,
        sessions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovery_observer_rebinds_expected_replica_and_commits() {
        let harness = harness("prod", 7, Arc::new(OkLoader)).await;
        let Harness {
            repo,
            handle,
            sessions,
        } = harness;

        handle.start_recovery("prod").expect("start recovery");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::WaitingRecovery)
            .await;

        // Production seam: the session layer notifies the coordinator when a
        // Runtime registration becomes routable.
        let observer: Arc<dyn RegistrationObserver> = Arc::new(handle.clone());
        observer.on_session_registered(&session("runtime-a", 1));
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Prepared)
            .await;

        let controls = sessions.controls.lock().expect("controls lock").clone();
        assert_eq!(
            controls.len(),
            1,
            "recovery rebind must enqueue one prepare"
        );
        let prepare = controls[0].1.clone();
        handle
            .deliver_ack(&session("runtime-a", 1), prepared_control(&prepare))
            .expect("deliver prepared ack");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Committed)
            .await;

        let durable = repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 8);
        assert!(
            durable.pending.is_none(),
            "recovery commit must clear pending"
        );
    }

    #[tokio::test]
    async fn recovery_candidate_load_failure_durably_aborts() {
        let harness = harness(
            "prod",
            7,
            Arc::new(CandidateFailingLoader {
                committed_generation: 7,
            }),
        )
        .await;
        let Harness { repo, handle, .. } = harness;

        handle.start_recovery("prod").expect("start recovery");
        handle
            .wait_for_phase(|phase| {
                matches!(phase, ActivationPhase::Aborted | ActivationPhase::Failed)
            })
            .await;

        let durable = repo.read("prod").await.expect("durable state");
        assert_eq!(durable.committed.generation, 7);
        assert!(
            durable.pending.is_none(),
            "candidate load failure must durably abort the pending"
        );
    }

    static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestRoot {
        parent: std::path::PathBuf,
        root: std::path::PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let parent = std::env::temp_dir().join(format!(
                "skiff-router-e-activation-recovery-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&parent).expect("create temp parent");
            Self {
                parent: parent.clone(),
                root: parent.join("root"),
            }
        }

        fn path(&self) -> &std::path::Path {
            &self.root
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.parent);
        }
    }

    fn materialize(profile: &str) -> (TestRoot, RuntimeAssemblyRef) {
        let root = TestRoot::new();
        std::fs::create_dir_all(root.path()).expect("create artifact root");
        let snapshot_store = RuntimeConfigSnapshotStore::create(root.path().join("runtime-config"))
            .expect("create snapshot store");
        let snapshot = RuntimeConfigSnapshot::new(profile, config_ref(SNAPSHOT), Vec::new())
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
        std::fs::create_dir_all(&directory).expect("create actor routing records directory");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            Vec::new(),
        )
        .expect("empty projection");
        let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
        std::fs::write(
            root.path().join("records/actor-routing/current.json"),
            bytes,
        )
        .expect("write projection record");
        (root, assembly_ref)
    }

    fn config(profile: &str, artifact_root: &std::path::Path) -> RouterConfig {
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

    #[tokio::test]
    async fn restart_converges_from_durable_committed_after_exit() {
        let (root, assembly_ref) = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        let committed = ProfileActivationState {
            schema_version: PROFILE_ACTIVATION_STATE_SCHEMA_VERSION.to_string(),
            profile: "prod".to_string(),
            committed: CommittedActivation {
                generation: 7,
                assembly: assembly_ref.clone(),
                config_snapshot: config_ref(SNAPSHOT),
            },
            pending: None,
        };
        repository
            .initialize(&committed)
            .await
            .expect("seed committed");
        repository
            .prepare(PrepareInput {
                profile: "prod".to_string(),
                activation_id: "recovery-8".to_string(),
                expected_generation: 7,
                candidate_generation: 8,
                assembly: assembly_ref.clone(),
                config_snapshot: config_ref(SNAPSHOT),
                participant_replica_ids: vec!["runtime-a".to_string()],
            })
            .await
            .expect("prepare pending");
        let config = config("prod", root.path());

        // First process run: pending is surfaced for recovery, committed is
        // published.
        let first = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect("first run assembles");
        assert_eq!(first.epoch().assembly_generation(), 7);
        assert!(first.pending_recovery().is_some());
        first.shutdown().await;

        // The process exits after the durable commit but before the epoch
        // swap was observable; the next run must converge from durable
        // committed alone (no second eligibility cache).
        let state = repository.read("prod").await.expect("durable state");
        let pending = state.pending.as_ref().expect("pending before commit");
        repository
            .commit(CommitInput {
                profile: "prod".to_string(),
                activation_id: pending.activation_id.clone(),
                expected_generation: pending.expected_generation,
                candidate_generation: pending.candidate_generation,
                assembly: pending.assembly.clone(),
                config_snapshot: pending.config_snapshot.clone(),
                connected_replica_ids: pending.participant_replica_ids.clone(),
                prepared_replica_ids: pending.participant_replica_ids.clone(),
            })
            .await
            .expect("durable commit");

        let second = RouterBootstrapAssembly::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect("second run assembles");
        assert_eq!(second.epoch().assembly_generation(), 8);
        assert!(
            second.pending_recovery().is_none(),
            "committed-only restart must have no recovery pending"
        );
        second.shutdown().await;
    }
}

//! `ActivationCoordinator` unit/sequence tests beyond the shared corpus:
//! production port adapters (routing-query adapter over the real directory,
//! blocking-loader adapter over the real bounded pool), `SessionConsumer`
//! fence integration, mailbox saturation, synchronous rejections,
//! fail-closed negatives (empty candidates, epoch mismatch), service-db wire
//! projection, sequential transactions on one coordinator, and lifecycle
//! terminals.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRequest, AssemblyIdentity,
    CanonicalPackageLinkPlan, DeploymentArtifactIdentity, DeploymentRevision, RuntimeAssembly,
    RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
    ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::activation_state::{
    ProfileActivationState, PROFILE_ACTIVATION_STATE_SCHEMA_VERSION,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, CommittedActivation};
use skiff_router::activation::{
    memory::MemoryActivationStateRepository, ActivationCandidateError, ActivationCoordinator,
    ActivationCoordinatorOptions, ActivationCoordinatorPorts, ActivationParticipantBinding,
    ActivationPhase, ActivationRevalidateOutcome, ActivationStateRepository,
    BlockingLoaderCandidatePort, BlockingLoaderPort, CandidateEpochRefs, CandidateLoadError,
    CoordinatorError, EnqueueResult, EpochStorePublishPort, NoopHealthSink,
    PublishCommittedEpochPort, RoutingCandidateQueryPortAdapter, RuntimeCandidateQueryPort,
    SessionEnqueuePort,
};
use skiff_router::artifact::{ActorRoutingCatalog, ActorRoutingProjectionRef};
use skiff_router::bootstrap::{
    ActiveRoutingEpochStore, BlockingLoader, BlockingLoaderOptions, BootstrapStrictLoader,
    RoutingEpoch,
};
use skiff_router::dispatch::CandidateViewSource;
use skiff_router::routing::{
    CandidateDirectoryView, DispatchCapabilities, RegisteredSessionLease, RuntimeCandidateQuery,
    SessionCancellation,
};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::directory::RuntimeRegistrationDirectory;
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::{ConsumerKind, SessionConsumer};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

const ASSEMBLY: &str =
    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT: &str = "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const FULL_SET: [ConsumerKind; 7] = [
    ConsumerKind::AdmissionPool,
    ConsumerKind::HealthLedger,
    ConsumerKind::RequestDispatcher,
    ConsumerKind::RuntimeGenerationPinLedger,
    ConsumerKind::WebSocketRequestBroker,
    ConsumerKind::ActorSessionOwner,
    ConsumerKind::ActivationCoordinator,
];

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

fn deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/service-1".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    }
}

fn epoch(
    profile: &str,
    generation: u64,
    assembly_ref: RuntimeAssemblyRef,
    config_snapshot_ref: RuntimeConfigSnapshotRef,
    deployments: Vec<ServiceDeploymentRef>,
) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_ref.assembly_identity.clone(),
        roots: deployments.clone(),
        resolved_deployments: deployments,
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

fn full_capabilities() -> DispatchCapabilities {
    DispatchCapabilities {
        unary: true,
        server_stream: true,
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

fn request(
    profile: &str,
    activation_id: &str,
    expected_generation: u64,
) -> AssemblyActivationRequest {
    AssemblyActivationRequest {
        schema_version: ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION.to_string(),
        profile: profile.to_string(),
        activation_id: activation_id.to_string(),
        expected_generation,
        assembly: assembly_ref(ASSEMBLY),
        config_snapshot: config_ref(SNAPSHOT),
    }
}

/// Directory-backed candidate view source for the production adapter.
#[derive(Debug)]
struct DirectoryViewSource {
    directory: Arc<StdMutex<RuntimeRegistrationDirectory>>,
    capabilities: HashMap<RuntimeSessionEpoch, DispatchCapabilities>,
}

impl CandidateViewSource for DirectoryViewSource {
    fn view(&self) -> CandidateDirectoryView {
        RuntimeCandidateQuery::snapshot_directory_view(
            &self.directory.lock().expect("directory lock"),
            &self.capabilities,
        )
    }
}

fn register_and_ack(
    directory: &mut RuntimeRegistrationDirectory,
    session: &RuntimeSessionEpoch,
    tuple: &RegisteredAssemblyTuple,
) {
    directory
        .publish_pending(session, tuple.clone(), &FULL_SET)
        .expect("registration");
    assert!(
        directory.mark_registered(session),
        "registered ACK must publish the session"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn routing_query_adapter_unions_all_exact_sessions_and_revalidates() {
        let current = epoch(
            "prod",
            7,
            assembly_ref(ASSEMBLY),
            config_ref(SNAPSHOT),
            vec![deployment()],
        );
        let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
        epoch_store.publish(Arc::clone(&current));

        let directory = Arc::new(StdMutex::new(RuntimeRegistrationDirectory::new(
            &ConsumerManifest::installed(FULL_SET),
        )));
        let mut capabilities = HashMap::new();
        capabilities.insert(
            session("runtime-a", 1),
            DispatchCapabilities {
                unary: true,
                server_stream: false,
            },
        );
        capabilities.insert(
            session("runtime-b", 2),
            DispatchCapabilities {
                unary: false,
                server_stream: true,
            },
        );
        {
            let mut directory = directory.lock().expect("directory lock");
            register_and_ack(&mut directory, &session("runtime-a", 1), &tuple("prod", 7));
            register_and_ack(&mut directory, &session("runtime-b", 2), &tuple("prod", 7));
        }

        let adapter = RoutingCandidateQueryPortAdapter::new(
            Arc::clone(&epoch_store),
            Arc::new(DirectoryViewSource {
                directory: Arc::clone(&directory),
                capabilities,
            }),
        );
        let leases = adapter.freeze("prod").expect("freeze");
        let mut replica_ids = leases
            .iter()
            .map(|lease| lease.session_epoch.replica_id.clone())
            .collect::<Vec<_>>();
        replica_ids.sort();
        assert_eq!(
            replica_ids,
            vec!["runtime-a".to_string(), "runtime-b".to_string()]
        );

        let frozen = leases
            .into_iter()
            .map(|lease| ActivationParticipantBinding {
                replica_id: lease.session_epoch.replica_id.clone(),
                session_epoch: lease.session_epoch,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            adapter.revalidate("activation-8", &frozen),
            ActivationRevalidateOutcome::Ok
        );

        // Cancel one exact session: the frozen set is stale.
        directory
            .lock()
            .expect("directory lock")
            .begin_close(&session("runtime-a", 1));
        assert_eq!(
            adapter.revalidate("activation-8", &frozen),
            ActivationRevalidateOutcome::Stale
        );
    }

    #[tokio::test]
    async fn routing_query_adapter_projects_exact_sessions_for_empty_epoch() {
        // Managed dev watch seeds a canonical empty generation 0 and then
        // commits the first real assembly through the ordinary CAS path.
        // The empty captured epoch still has its exact registered sessions as
        // activation participants.
        let current = epoch(
            "prod",
            7,
            assembly_ref(ASSEMBLY),
            config_ref(SNAPSHOT),
            Vec::new(),
        );
        let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
        epoch_store.publish(Arc::clone(&current));

        let directory = Arc::new(StdMutex::new(RuntimeRegistrationDirectory::new(
            &ConsumerManifest::installed(FULL_SET),
        )));
        let mut capabilities = HashMap::new();
        capabilities.insert(
            session("runtime-a", 1),
            // An empty bootstrap epoch has no dispatch surfaces, so the
            // runtime's capability binding is empty; the empty-epoch
            // projection must still treat the exact session as a participant.
            DispatchCapabilities::default(),
        );
        {
            let mut directory = directory.lock().expect("directory lock");
            register_and_ack(&mut directory, &session("runtime-a", 1), &tuple("prod", 7));
        }

        let adapter = RoutingCandidateQueryPortAdapter::new(
            Arc::clone(&epoch_store),
            Arc::new(DirectoryViewSource {
                directory: Arc::clone(&directory),
                capabilities,
            }),
        );
        let leases = adapter.freeze("prod").expect("empty epoch freeze");
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].session_epoch.replica_id, "runtime-a");
        assert_eq!(
            leases[0].exact_registered_tuple,
            tuple("prod", 7),
            "empty epoch participants must still carry the exact epoch tuple"
        );

        let frozen = leases
            .into_iter()
            .map(|lease| ActivationParticipantBinding {
                replica_id: lease.session_epoch.replica_id.clone(),
                session_epoch: lease.session_epoch,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            adapter.revalidate("activation-8", &frozen),
            ActivationRevalidateOutcome::Ok
        );
    }

    #[tokio::test]
    async fn routing_query_adapter_fails_closed_without_published_epoch() {
        let adapter = RoutingCandidateQueryPortAdapter::new(
            Arc::new(ActiveRoutingEpochStore::new()),
            Arc::new(DirectoryViewSource {
                directory: Arc::new(StdMutex::new(RuntimeRegistrationDirectory::new(
                    &ConsumerManifest::installed(FULL_SET),
                ))),
                capabilities: HashMap::new(),
            }),
        );
        assert!(matches!(
            adapter.freeze("prod"),
            Err(ActivationCandidateError::NoEpoch { .. })
        ));
        assert_eq!(
            adapter.revalidate("activation-8", &[]),
            ActivationRevalidateOutcome::Stale
        );
    }

    static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestRoot {
        parent: std::path::PathBuf,
        root: std::path::PathBuf,
    }

    impl TestRoot {
        fn new() -> Self {
            let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let parent = std::env::temp_dir().join(format!(
                "skiff-router-w-activation-unit-{}-{sequence}",
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

    fn materialize_strict_loader(
        profile: &str,
    ) -> (
        TestRoot,
        Arc<BootstrapStrictLoader>,
        ActorRoutingProjectionRef,
    ) {
        let root = TestRoot::new();
        let snapshot_store =
            skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(root.path())
                .expect("create snapshot store");
        let snapshot = RuntimeConfigSnapshot::new(profile, config_ref(SNAPSHOT), Vec::new())
            .expect("snapshot fixture");
        snapshot_store.publish(&snapshot).expect("publish snapshot");
        let artifact_store =
            CanonicalArtifactStore::create(root.path()).expect("create artifact store");
        let assembly =
            skiff_deployment::fixtures::empty_runtime_assembly_fixture().expect("assembly fixture");
        artifact_store
            .write_runtime_assembly(&assembly)
            .expect("write assembly");
        let directory = root.path().join("records/actor-routing");
        std::fs::create_dir_all(&directory).expect("create actor routing records directory");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            Vec::new(),
        )
        .expect("empty projection");
        let bytes = skiff_canonical_json::canonical_json_bytes(&projection)
            .expect("canonical projection bytes");
        std::fs::write(directory.join("empty.json"), bytes).expect("write projection record");
        let actor_ref = ActorRoutingProjectionRef::new(
            skiff_artifact_identity::ArtifactRelativePath::new(
                "records/actor-routing/empty.json",
                "test record",
            )
            .expect("record path"),
        );
        let loader = Arc::new(
            BootstrapStrictLoader::open(root.path(), root.path()).expect("open strict loader"),
        );
        (root, loader, actor_ref)
    }

    #[tokio::test]
    async fn blocking_loader_candidate_port_loads_real_epoch() {
        let (_root, strict_loader, actor_ref) = materialize_strict_loader("prod");
        let pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
        let port = BlockingLoaderCandidatePort::new(pool, strict_loader, actor_ref);
        let refs = CandidateEpochRefs {
            profile: "prod".to_string(),
            generation: 8,
            assembly: skiff_artifact_identity::runtime_assembly_ref(
                &skiff_deployment::fixtures::empty_runtime_assembly_fixture()
                    .expect("assembly fixture"),
            )
            .expect("assembly ref"),
            config_snapshot: config_ref(SNAPSHOT),
        };
        let epoch = port.load_candidate(&refs).await.expect("load candidate");
        assert_eq!(epoch.profile(), "prod");
        assert_eq!(epoch.assembly_generation(), 8);
        assert_eq!(epoch.config_snapshot_id(), SNAPSHOT);
    }

    #[tokio::test]
    async fn blocking_loader_candidate_port_saturation_and_shutdown_fail_closed() {
        let (_root, strict_loader, actor_ref) = materialize_strict_loader("prod");
        let pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions {
            concurrency: 1,
            read_deadline: Duration::from_secs(5),
            drain_deadline: Duration::from_secs(1),
        }));
        let port = BlockingLoaderCandidatePort::new(Arc::clone(&pool), strict_loader, actor_ref);
        let refs = CandidateEpochRefs {
            profile: "prod".to_string(),
            generation: 8,
            assembly: skiff_artifact_model::RuntimeAssemblyRef {
                assembly_identity: AssemblyIdentity::new(ASSEMBLY),
            },
            config_snapshot: config_ref(SNAPSHOT),
        };
        let blocking = tokio::spawn({
            let pool = Arc::clone(&pool);
            async move {
                pool.run(|| {
                    std::thread::sleep(Duration::from_millis(300));
                    Ok::<(), ()>(())
                })
                .await
            }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(
            port.load_candidate(&refs).await.expect_err("saturated"),
            CandidateLoadError::Saturated
        );
        blocking.await.expect("blocking op").expect("pool op");

        pool.shutdown().await;
        assert_eq!(
            port.load_candidate(&refs).await.expect_err("shutdown"),
            CandidateLoadError::Shutdown
        );
    }

    /// Minimal fakes for coordinator sequence tests.
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
                Vec::new(),
            ))
        }
    }

    #[derive(Debug)]
    struct ScriptedCandidates {
        leases: StdMutex<Vec<RegisteredSessionLease>>,
    }

    impl ScriptedCandidates {
        fn new(tuple: RegisteredAssemblyTuple, replica_ids: &[(&str, u64)]) -> Self {
            let leases = Self::leases_for(tuple, replica_ids);
            Self {
                leases: StdMutex::new(leases),
            }
        }

        fn set_leases(&self, tuple: RegisteredAssemblyTuple, replica_ids: &[(&str, u64)]) {
            *self.leases.lock().expect("leases lock") = Self::leases_for(tuple, replica_ids);
        }

        fn leases_for(
            tuple: RegisteredAssemblyTuple,
            replica_ids: &[(&str, u64)],
        ) -> Vec<RegisteredSessionLease> {
            replica_ids
                .iter()
                .map(|(replica_id, generation)| RegisteredSessionLease {
                    session_epoch: session(replica_id, *generation),
                    registration_revision: 1,
                    exact_registered_tuple: tuple.clone(),
                    cancellation: SessionCancellation { cancelled: false },
                    capabilities: full_capabilities(),
                })
                .collect()
        }
    }

    impl RuntimeCandidateQueryPort for ScriptedCandidates {
        fn freeze(
            &self,
            _profile: &str,
        ) -> Result<Vec<RegisteredSessionLease>, ActivationCandidateError> {
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
        aborted: StdMutex<Vec<RuntimeSessionEpoch>>,
    }

    impl SessionEnqueuePort for RecordingSessions {
        fn enqueue_prepare(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("prepare".to_string(), control.clone()));
            let _ = binding;
            EnqueueResult::Ok
        }

        fn enqueue_commit(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("commit".to_string(), control.clone()));
            let _ = binding;
            EnqueueResult::Ok
        }

        fn enqueue_abort(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            self.controls
                .lock()
                .expect("controls lock")
                .push(("abort".to_string(), control.clone()));
            let _ = binding;
            EnqueueResult::Ok
        }

        fn abort_session(&self, session: &RuntimeSessionEpoch) {
            self.aborted
                .lock()
                .expect("aborted lock")
                .push(session.clone());
        }
    }

    struct Harness {
        repo: Arc<MemoryActivationStateRepository>,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        sessions: Arc<RecordingSessions>,
        candidates: Arc<ScriptedCandidates>,
        handle: skiff_router::activation::ActivationCoordinatorHandle,
    }

    async fn harness(
        profile: &str,
        committed_generation: u64,
        candidates: Arc<ScriptedCandidates>,
        options: ActivationCoordinatorOptions,
    ) -> Harness {
        let repo = Arc::new(MemoryActivationStateRepository::new());
        repo.initialize(&initial_state(profile, committed_generation))
            .await
            .expect("initialize");
        let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
        let sessions = Arc::new(RecordingSessions::default());
        let ports = ActivationCoordinatorPorts {
            repository: Arc::clone(&repo) as Arc<dyn ActivationStateRepository>,
            loader: Arc::new(OkLoader) as Arc<dyn BlockingLoaderPort>,
            candidates: Arc::clone(&candidates) as Arc<dyn RuntimeCandidateQueryPort>,
            sessions: Arc::clone(&sessions) as Arc<dyn SessionEnqueuePort>,
            publish: Arc::new(EpochStorePublishPort::new(Arc::clone(&epoch_store)))
                as Arc<dyn PublishCommittedEpochPort>,
            health: Arc::new(NoopHealthSink),
        };
        let handle = ActivationCoordinator::spawn(ports, options);
        Harness {
            repo,
            epoch_store,
            sessions,
            candidates,
            handle,
        }
    }

    fn default_options() -> ActivationCoordinatorOptions {
        ActivationCoordinatorOptions {
            mailbox_capacity: 64,
            ack_deadline: Duration::from_secs(3600),
            service_db_mongo_url: None,
        }
    }

    async fn wait_prepared(handle: &skiff_router::activation::ActivationCoordinatorHandle) {
        handle
            .wait_for_phase(|phase| {
                matches!(
                    phase,
                    ActivationPhase::Prepared
                        | ActivationPhase::Failed
                        | ActivationPhase::Aborted
                        | ActivationPhase::Committed
                )
            })
            .await;
    }

    async fn wait_health_for_activation(
        handle: &skiff_router::activation::ActivationCoordinatorHandle,
        activation_id: &str,
        phase: ActivationPhase,
    ) {
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some(activation_id) && health.phase == phase
            })
            .await;
    }

    #[tokio::test]
    async fn session_consumer_fence_aborts_pre_decision_transaction() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1), ("runtime-b", 2)],
        ));
        let harness = harness("test", 7, candidates, default_options()).await;
        let Harness {
            repo,
            sessions,
            handle,
            ..
        } = harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("start live");
        wait_prepared(&handle).await;
        assert_eq!(handle.phase(), ActivationPhase::Prepared);

        let consumer: Arc<dyn SessionConsumer> = Arc::new(handle.clone());
        assert_eq!(consumer.kind(), ConsumerKind::ActivationCoordinator);
        consumer
            .on_session_closed(&session("runtime-a", 1))
            .expect("terminal delivery");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Aborted)
            .await;

        let durable = repo.read("test").await.expect("read");
        assert_eq!(durable.committed.generation, 7);
        assert_eq!(durable.pending, None);
        let kinds = sessions
            .controls
            .lock()
            .expect("controls lock")
            .iter()
            .map(|(kind, _)| kind.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "prepare".to_string(),
                "prepare".to_string(),
                "abort".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn ack_timeout_aborts_with_timeout_reason_and_clears_pending() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let harness = harness("test", 7, candidates, default_options()).await;
        let Harness { repo, handle, .. } = harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("start live");
        wait_prepared(&handle).await;
        assert_eq!(handle.phase(), ActivationPhase::Prepared);

        handle.force_ack_timeout().expect("force ack timeout");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Aborted)
            .await;

        // The control response must be able to surface the exact timeout
        // reason as HTTP 504 (runtime-deployment-topology.md), not a generic
        // "not committed" 409.
        let health = handle.health();
        assert_eq!(
            health.last_failure.as_deref(),
            Some("assembly activation prepare timed out")
        );
        let durable = repo.read("test").await.expect("read");
        assert_eq!(durable.committed.generation, 7);
        assert_eq!(durable.pending, None);
    }

    #[tokio::test]
    async fn mailbox_saturation_counts_and_rejects_overflow() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let options = ActivationCoordinatorOptions {
            mailbox_capacity: 1,
            ..default_options()
        };
        let harness = harness("test", 7, candidates, options).await;
        let Harness { handle, .. } = harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("first start fits the mailbox");
        assert_eq!(
            handle.start_live(request("test", "activation-8", 7)),
            Err(CoordinatorError::MailboxFull)
        );
        wait_prepared(&handle).await;
        assert_eq!(handle.phase(), ActivationPhase::Prepared);
        assert_eq!(handle.health().mailbox_saturation, 1);
        assert_eq!(handle.health().mailbox_capacity, 1);
    }

    #[tokio::test]
    async fn invalid_request_and_in_progress_are_rejected_synchronously() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let harness = harness("test", 7, candidates, default_options()).await;
        let Harness { handle, .. } = harness;
        let mut invalid = request("test", "activation-8", 7);
        invalid.schema_version = "skiff-assembly-activation-request-v9".to_string();
        assert!(matches!(
            handle.start_live(invalid),
            Err(CoordinatorError::InvalidRequest(_))
        ));
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("valid start");
        wait_prepared(&handle).await;
        assert_eq!(
            handle.start_live(request("test", "activation-8", 7)),
            Err(CoordinatorError::TransactionInProgress)
        );
    }

    #[tokio::test]
    async fn empty_candidates_and_epoch_mismatch_fail_closed_without_durable_effect() {
        let empty = Arc::new(ScriptedCandidates::new(tuple("test", 7), &[]));
        let empty_harness = harness("test", 7, empty, default_options()).await;
        let Harness { repo, handle, .. } = empty_harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("start live");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Failed)
            .await;
        let durable = repo.read("test").await.expect("read");
        assert_eq!(durable.committed.generation, 7);
        assert_eq!(durable.pending, None);
        handle.hard_abort().expect("cleanup");

        // A lease tuple whose generation does not match the durable committed
        // generation is the active-epoch/durable mismatch (§4.1 step 1).
        let mismatched = Arc::new(ScriptedCandidates::new(
            tuple("test", 6),
            &[("runtime-a", 1)],
        ));
        let mismatch_harness = harness("test", 7, mismatched, default_options()).await;
        let Harness {
            repo,
            handle,
            sessions,
            ..
        } = mismatch_harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("start live");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Failed)
            .await;
        let durable = repo.read("test").await.expect("read");
        assert_eq!(durable.pending, None);
        assert!(
            sessions.controls.lock().expect("controls lock").is_empty(),
            "no wire frames may be sent on fail-closed paths"
        );
    }

    #[tokio::test]
    async fn service_db_mongo_url_is_carried_on_prepare_and_commit_wire() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let options = ActivationCoordinatorOptions {
            service_db_mongo_url: Some("mongodb://127.0.0.1:27017".to_string()),
            ..default_options()
        };
        let harness = harness("test", 7, candidates, options).await;
        let Harness {
            sessions, handle, ..
        } = harness;
        handle
            .start_live(request("test", "activation-8", 7))
            .expect("start live");
        wait_prepared(&handle).await;
        handle
            .deliver_ack(
                &session("runtime-a", 1),
                AssemblyActivationControl::Prepared {
                    profile: "test".to_string(),
                    activation_id: "activation-8".to_string(),
                    expected_generation: 7,
                    candidate_generation: 8,
                    assembly: assembly_ref(ASSEMBLY),
                    config_snapshot: config_ref(SNAPSHOT),
                    replica_id: "runtime-a".to_string(),
                },
            )
            .expect("deliver prepared");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Committed)
            .await;
        let controls = sessions.controls.lock().expect("controls lock").clone();
        assert_eq!(controls.len(), 2);
        assert!(matches!(
            &controls[0].1,
            AssemblyActivationControl::Prepare { service_db: Some(db), .. }
                if db.mongo_url == "mongodb://127.0.0.1:27017"
        ));
        assert!(matches!(
            &controls[1].1,
            AssemblyActivationControl::Commit { service_db: Some(db), .. }
                if db.mongo_url == "mongodb://127.0.0.1:27017"
        ));
    }

    #[tokio::test]
    async fn sequential_transactions_on_one_coordinator_advance_generations() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let harness = harness("test", 7, candidates, default_options()).await;
        let Harness {
            repo,
            epoch_store,
            sessions,
            handle,
            ..
        } = harness;

        handle
            .start_live(request("test", "activation-8", 7))
            .expect("first start");
        wait_prepared(&handle).await;
        handle
            .deliver_ack(
                &session("runtime-a", 1),
                AssemblyActivationControl::Prepared {
                    profile: "test".to_string(),
                    activation_id: "activation-8".to_string(),
                    expected_generation: 7,
                    candidate_generation: 8,
                    assembly: assembly_ref(ASSEMBLY),
                    config_snapshot: config_ref(SNAPSHOT),
                    replica_id: "runtime-a".to_string(),
                },
            )
            .expect("first prepared");
        wait_health_for_activation(&handle, "activation-8", ActivationPhase::Committed).await;
        assert_eq!(
            repo.read("test").await.expect("read").committed.generation,
            8
        );
        assert_eq!(
            epoch_store.capture().expect("epoch").assembly_generation(),
            8
        );

        // Second transaction on the same coordinator: new expected generation and
        // a new activation id (audit dedup key stays unique).
        // The candidate port must re-freeze against the new current epoch for
        // the second transaction.
        harness
            .candidates
            .set_leases(tuple("test", 8), &[("runtime-a", 1)]);
        handle
            .start_live(request("test", "activation-9", 8))
            .expect("second start");
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some("activation-9")
                    && matches!(
                        health.phase,
                        ActivationPhase::Prepared
                            | ActivationPhase::Failed
                            | ActivationPhase::Aborted
                            | ActivationPhase::Committed
                    )
            })
            .await;
        handle
            .deliver_ack(
                &session("runtime-a", 1),
                AssemblyActivationControl::Prepared {
                    profile: "test".to_string(),
                    activation_id: "activation-9".to_string(),
                    expected_generation: 8,
                    candidate_generation: 9,
                    assembly: assembly_ref(ASSEMBLY),
                    config_snapshot: config_ref(SNAPSHOT),
                    replica_id: "runtime-a".to_string(),
                },
            )
            .expect("second prepared");
        wait_health_for_activation(&handle, "activation-9", ActivationPhase::Committed).await;
        assert_eq!(
            repo.read("test").await.expect("read").committed.generation,
            9
        );
        assert_eq!(
            epoch_store.capture().expect("epoch").assembly_generation(),
            9
        );
        assert_eq!(
            sessions.controls.lock().expect("controls lock").len(),
            4,
            "prepare+commit for each of the two transactions"
        );
    }

    #[tokio::test]
    async fn shutdown_idle_and_hard_abort_terminate_the_coordinator() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let shutdown_harness = harness("test", 7, candidates, default_options()).await;
        let Harness { handle, .. } = shutdown_harness;
        handle.shutdown().expect("shutdown");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Shutdown)
            .await;
        assert!(handle.health().shutdown);
        assert_eq!(
            handle.start_live(request("test", "activation-8", 7)),
            Err(CoordinatorError::Shutdown)
        );

        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let abort_harness = harness("test", 7, candidates, default_options()).await;
        let Harness { handle, .. } = abort_harness;
        handle.hard_abort().expect("hard abort");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Exited)
            .await;
        assert_eq!(
            handle.start_live(request("test", "activation-8", 7)),
            Err(CoordinatorError::Shutdown)
        );
    }

    #[tokio::test]
    async fn recovery_start_without_pending_publishes_committed_only() {
        let candidates = Arc::new(ScriptedCandidates::new(
            tuple("test", 7),
            &[("runtime-a", 1)],
        ));
        let harness = harness("test", 7, candidates, default_options()).await;
        let Harness {
            epoch_store,
            handle,
            ..
        } = harness;
        handle.start_recovery("test").expect("start recovery");
        handle
            .wait_for_phase(|phase| phase == ActivationPhase::Committed)
            .await;
        assert_eq!(
            epoch_store.capture().expect("epoch").assembly_generation(),
            7
        );
        assert!(!handle.health().recovery_active);
    }

    #[test]
    fn phase_and_decision_vocabulary_stay_frozen() {
        use skiff_router::activation::{ActivationPhase, DecisionState};
        assert_eq!(ActivationPhase::Idle.as_str(), "idle");
        assert_eq!(ActivationPhase::Freezing.as_str(), "freezing");
        assert_eq!(ActivationPhase::Prepared.as_str(), "prepared");
        assert_eq!(ActivationPhase::WaitingRecovery.as_str(), "waitingRecovery");
        assert_eq!(ActivationPhase::Committing.as_str(), "committing");
        assert_eq!(ActivationPhase::Committed.as_str(), "committed");
        assert_eq!(ActivationPhase::Aborted.as_str(), "aborted");
        assert_eq!(ActivationPhase::Failed.as_str(), "failed");
        assert_eq!(ActivationPhase::Shutdown.as_str(), "shutdown");
        assert_eq!(ActivationPhase::Exited.as_str(), "exited");
        assert_eq!(DecisionState::Idle.as_str(), "idle");
        assert_eq!(DecisionState::Preparing.as_str(), "preparing");
        assert_eq!(DecisionState::Committing.as_str(), "committing");
        assert_eq!(DecisionState::Committed.as_str(), "committed");
        assert_eq!(DecisionState::Aborted.as_str(), "aborted");
    }
}

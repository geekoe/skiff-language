//! W-activation real-boundary probe (C-activation-coordinator §9 slice):
//! real `MongoActivationStateRepository` + real `ActiveRoutingEpochStore`
//! + real `ActivationCoordinator` with scripted loader/candidates/sessions.
//!
//! Two complete prepare → ACK → durable commit → epoch swap → commit-enqueue
//! cycles run through Mongo (generation 7→8→9), a stale ACK is rejected, and
//! the audit trail contains exactly one event per effective mutation with no
//! duplicates. The full-chain real Runtime re-register roundtrip belongs to
//! E-activation (`router-live:activation-full-chain`).
//!
//! Ignored by default; the runner
//! (`scripts/run-router-activation-coordinator-mongo-probe.mjs`) starts an
//! isolated temporary mongod replica set, injects
//! `SKIFF_ACTIVATION_MONGO_URL`/`SKIFF_ACTIVATION_MONGO_DB`, and cleans up.

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use mongodb::bson::doc;
    use skiff_artifact_model::{
        AssemblyActivationControl, AssemblyActivationRequest, AssemblyIdentity,
        CanonicalPackageLinkPlan, RuntimeAssembly, RuntimeAssemblyRef, RuntimeConfigSnapshotId,
        RuntimeConfigSnapshotRef, ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION,
        RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    };
    use skiff_deployment::activation_state::ProfileActivationState;
    use skiff_deployment::projection::actor_routing::{
        ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
    };
    use skiff_router::activation::{
        ActivationCoordinator, ActivationCoordinatorOptions, ActivationCoordinatorPorts,
        ActivationParticipantBinding, ActivationPhase, ActivationRevalidateOutcome,
        ActivationStateRepository, BlockingLoaderPort, CandidateEpochRefs, CandidateLoadError,
        EnqueueResult, EpochStorePublishPort, MongoActivationStateRepository,
        MongoActivationStateRepositoryOptions, NoopHealthSink, PublishCommittedEpochPort,
        RetryPolicy, RuntimeCandidateQueryPort, SessionEnqueuePort, SystemClock,
    };
    use skiff_router::artifact::ActorRoutingCatalog;
    use skiff_router::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
    use skiff_router::routing::{
        DispatchCapabilities, RegisteredSessionLease, SessionCancellation,
    };
    use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
    use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

    fn assembly(byte: u8) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                char::from(b'a' + byte).to_string().repeat(64)
            )),
        }
    }

    fn config(byte: u8) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(format!(
                "skiff-runtime-config-snapshot-v1:{}",
                char::from(b'a' + byte).to_string().repeat(32)
            ))
            .expect("config snapshot id"),
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

    #[derive(Debug, Default)]
    struct ProbeLoader;

    #[async_trait]
    impl BlockingLoaderPort for ProbeLoader {
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

    #[derive(Debug)]
    struct ProbeCandidates {
        leases: StdMutex<Vec<RegisteredSessionLease>>,
    }

    impl ProbeCandidates {
        fn set(&self, tuple: RegisteredAssemblyTuple) {
            *self.leases.lock().expect("leases lock") = vec![RegisteredSessionLease {
                session_epoch: session("runtime-a", 1),
                registration_revision: 1,
                exact_registered_tuple: tuple,
                cancellation: SessionCancellation { cancelled: false },
                capabilities: DispatchCapabilities {
                    unary: true,
                    server_stream: true,
                },
                registered_build_ids: Vec::new(),
                lazy_load: false,
                artifact_root: None,
            }];
        }
    }

    impl RuntimeCandidateQueryPort for ProbeCandidates {
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
    struct ProbeSessions {
        enqueues: StdMutex<Vec<(String, String)>>,
        aborted: StdMutex<Vec<String>>,
    }

    impl SessionEnqueuePort for ProbeSessions {
        fn enqueue_prepare(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            assert_router_to_runtime(control);
            self.enqueues
                .lock()
                .expect("enqueues")
                .push(("prepare".to_string(), binding.replica_id.clone()));
            EnqueueResult::Ok
        }

        fn enqueue_commit(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            assert_router_to_runtime(control);
            self.enqueues
                .lock()
                .expect("enqueues")
                .push(("commit".to_string(), binding.replica_id.clone()));
            EnqueueResult::Ok
        }

        fn enqueue_abort(
            &self,
            binding: &ActivationParticipantBinding,
            control: &AssemblyActivationControl,
        ) -> EnqueueResult {
            assert_router_to_runtime(control);
            self.enqueues
                .lock()
                .expect("enqueues")
                .push(("abort".to_string(), binding.replica_id.clone()));
            EnqueueResult::Ok
        }

        fn abort_session(&self, session: &RuntimeSessionEpoch) {
            self.aborted
                .lock()
                .expect("aborted")
                .push(session.replica_id.clone());
        }
    }

    fn assert_router_to_runtime(control: &AssemblyActivationControl) {
        let bytes = skiff_runtime_transport::assembly_activation::encode_assembly_activation_frame(
            skiff_runtime_transport::assembly_activation::AssemblyActivationFrameDirection::RouterToRuntime,
            control,
        )
        .expect("router to runtime encode");
        let decoded = skiff_runtime_transport::assembly_activation::decode_assembly_activation_frame(
            skiff_runtime_transport::assembly_activation::AssemblyActivationFrameDirection::RouterToRuntime,
            &bytes,
        )
        .expect("router to runtime decode");
        assert_eq!(decoded, *control);
    }

    fn prepared_control(
        profile: &str,
        activation_id: &str,
        expected_generation: u64,
    ) -> AssemblyActivationControl {
        AssemblyActivationControl::Prepared {
            profile: profile.to_string(),
            activation_id: activation_id.to_string(),
            expected_generation,
            candidate_generation: expected_generation + 1,
            assembly: assembly(1),
            config_snapshot: config(1),
            replica_id: "runtime-a".to_string(),
        }
    }

    async fn connect(database: &str) -> MongoActivationStateRepository {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL")
            .expect("SKIFF_ACTIVATION_MONGO_URL must be set by the probe harness");
        MongoActivationStateRepository::connect(
            &mongo_url,
            MongoActivationStateRepositoryOptions {
                database: database.to_string(),
                retry: RetryPolicy {
                    max_attempts: 6,
                    base_delay: Duration::from_millis(25),
                    max_delay: Duration::from_millis(250),
                    total_deadline: Duration::from_secs(10),
                },
                ..Default::default()
            },
            Arc::new(SystemClock),
        )
        .await
        .expect("connect repository")
    }

    async fn count_audit(database: &str, profile: &str, activation_id: &str) -> usize {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .count_documents(doc! {
                "profile": profile,
                "activationId": activation_id
            })
            .await
            .expect("count audit documents") as usize
    }

    async fn count_audit_total(database: &str, profile: &str) -> usize {
        let mongo_url = std::env::var("SKIFF_ACTIVATION_MONGO_URL").expect("mongo url");
        let client = mongodb::Client::with_uri_str(&mongo_url)
            .await
            .expect("raw client");
        client
            .database(database)
            .collection::<mongodb::bson::Document>("activation_audit")
            .count_documents(doc! { "profile": profile })
            .await
            .expect("count audit documents") as usize
    }

    #[tokio::test]
    #[ignore = "requires SKIFF_ACTIVATION_MONGO_URL temporary replica set managed by the probe harness"]
    async fn activation_coordinator_mongo_probe_two_full_cycles_and_audit_dedup() {
        let database = std::env::var("SKIFF_ACTIVATION_MONGO_DB")
            .unwrap_or_else(|_| "skiff_router_activation_coordinator_probe".to_string());
        let repository = Arc::new(connect(&database).await);
        repository.ensure_indexes().await.expect("ensure indexes");
        repository
            .initialize(&ProfileActivationState::initial(
                "probe",
                7,
                assembly(0),
                config(0),
            ))
            .await
            .expect("initialize");

        let epoch_store = Arc::new(ActiveRoutingEpochStore::new());
        epoch_store.publish(epoch("probe", 7, assembly(0), config(0)));
        let candidates = Arc::new(ProbeCandidates {
            leases: StdMutex::new(Vec::new()),
        });
        candidates.set(RegisteredAssemblyTuple {
            profile: "probe".to_string(),
            generation: 7,
            assembly: assembly(0),
            config_snapshot: config(0),
        });
        let sessions = Arc::new(ProbeSessions::default());
        let ports = ActivationCoordinatorPorts {
            repository: Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
            loader: Arc::new(ProbeLoader),
            candidates: Arc::clone(&candidates) as Arc<dyn RuntimeCandidateQueryPort>,
            sessions: Arc::clone(&sessions) as Arc<dyn SessionEnqueuePort>,
            publish: Arc::new(EpochStorePublishPort::new(Arc::clone(&epoch_store)))
                as Arc<dyn PublishCommittedEpochPort>,
            health: Arc::new(NoopHealthSink),
        };
        let handle = ActivationCoordinator::spawn(
            ports,
            ActivationCoordinatorOptions {
                mailbox_capacity: 128,
                ack_deadline: Duration::from_secs(3600),
                service_db_mongo_url: None,
            },
        );

        // Cycle 1: prepare -> ACK -> durable commit -> swap -> commit enqueue.
        let first = request("probe", "activation-8", 7);
        handle.start_live(first.clone()).expect("start cycle 1");
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some("activation-8")
                    && health.phase == ActivationPhase::Prepared
            })
            .await;
        handle
            .deliver_ack(
                &session("runtime-a", 1),
                prepared_control("probe", "activation-8", 7),
            )
            .expect("cycle 1 prepared");
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some("activation-8")
                    && health.phase == ActivationPhase::Committed
            })
            .await;

        // Cycle 2: stale ACK is rejected (wrong session epoch), the real ACK
        // commits, and the second swap advances to generation 9.
        candidates.set(RegisteredAssemblyTuple {
            profile: "probe".to_string(),
            generation: 8,
            assembly: assembly(0),
            config_snapshot: config(0),
        });
        let second = request("probe", "activation-9", 8);
        handle.start_live(second.clone()).expect("start cycle 2");
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some("activation-9")
                    && health.phase == ActivationPhase::Prepared
            })
            .await;
        handle
            .deliver_ack(
                &session("runtime-a", 99),
                prepared_control("probe", "activation-9", 8),
            )
            .expect("stale ack delivery");
        handle
            .deliver_ack(
                &session("runtime-a", 1),
                prepared_control("probe", "activation-9", 8),
            )
            .expect("cycle 2 prepared");
        handle
            .wait_until_health(|health| {
                health.activation_id.as_deref() == Some("activation-9")
                    && health.phase == ActivationPhase::Committed
            })
            .await;

        assert_eq!(
            epoch_store.capture().expect("epoch").assembly_generation(),
            9,
            "both swaps must land"
        );
        let durable = repository.read("probe").await.expect("durable read");
        assert_eq!(
            durable.committed.generation, 9,
            "durable committed must advance through both Mongo CAS cycles"
        );
        assert_eq!(durable.pending, None);

        let stale_acks = handle.health().stale_acks;
        assert_eq!(stale_acks, 1, "stale/new session ACK must be rejected");
        assert_eq!(
            sessions.enqueues.lock().expect("enqueues").clone(),
            vec![
                ("prepare".to_string(), "runtime-a".to_string()),
                ("commit".to_string(), "runtime-a".to_string()),
                ("prepare".to_string(), "runtime-a".to_string()),
                ("commit".to_string(), "runtime-a".to_string()),
            ]
        );
        assert!(
            sessions.aborted.lock().expect("aborted").is_empty(),
            "no session fence expected on the happy two-cycle probe"
        );
        assert_eq!(
            count_audit(&database, "probe", "activation-8").await,
            2,
            "prepare+commit audit for cycle 1"
        );
        assert_eq!(
            count_audit(&database, "probe", "activation-9").await,
            2,
            "prepare+commit audit for cycle 2"
        );
        assert_eq!(
            count_audit_total(&database, "probe").await,
            4,
            "no duplicate audit events across retries/cycles"
        );
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
            assembly: assembly(1),
            config_snapshot: config(1),
        }
    }
}

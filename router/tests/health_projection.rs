//! Batch 12 health projection: wire-shape and zero/nonzero counter contract
//! tests at the health output layer.

use std::collections::{BTreeMap, HashMap, HashSet};

use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_router::activation::{ActivationCoordinatorHealth, ActivationPhase, DecisionState};
use skiff_router::actor::{
    ActivationHealth, ActorHealthSnapshot, CatalogHealth, ControlHealth, InvocationHealth,
    LeaseHealth, OwnershipHealth, TaskHealth as ActorTaskHealth,
};
use skiff_router::dispatch::{
    AdmissionHealth, DispatcherHealthSnapshot, PendingHealth, TaskHealth, TerminalHealth,
    TerminalSource,
};
use skiff_router::health::{
    project_capability_connections, project_loop_risk_runtimes, project_replicas, render_base,
    ActiveAssemblyProjection, HealthCounters, LoopRiskProjection, LoopRiskRuntimeProjection,
    SessionFacts,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::ws::{BrokerHealthSnapshot, IndexHealthSnapshot, LedgerHealthSnapshot};
use skiff_runtime_transport::protocol::{
    RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn tuple() -> RegisteredAssemblyTuple {
        RegisteredAssemblyTuple {
        environment: "prod".to_string(),
        generation: 7,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        },
        config_snapshot: RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("snapshot"),
        },
    }
    }

    fn session(replica: &str) -> RuntimeSessionEpoch {
        RuntimeSessionEpoch {
            replica_id: replica.to_string(),
            connection_generation: 1,
        }
    }

    fn zero_counters() -> RuntimeHealthCountersFrameHeader {
        RuntimeHealthCountersFrameHeader {
            outbound_requests_pending: 0,
            outbound_stream_leases_active: 0,
            stream_runtime_streams_active: 0,
            flag_backed_cancel_waiters_active: 0,
            task_requests_active: 0,
        }
    }

    fn observation(
        replica: &str,
        observed_at: &str,
    ) -> (RuntimeSessionEpoch, RuntimeHealthFrameHeader) {
        (
            session(replica),
            RuntimeHealthFrameHeader {
                schema_version: "skiff-runtime-frame-v3".to_string(),
                envelope_type: "runtime.health".to_string(),
                runtime_id: replica.to_string(),
                observed_at: observed_at.to_string(),
                counters: zero_counters(),
            },
        )
    }

    fn zero_health_counters() -> HealthCounters {
        let dispatcher = DispatcherHealthSnapshot {
            pending: PendingHealth::default(),
            terminal: TerminalHealth {
                by_source: BTreeMap::new(),
            },
            admission: AdmissionHealth::default(),
            task: TaskHealth::default(),
            stopped: false,
        };
        let coordinator = ActivationCoordinatorHealth {
            phase: ActivationPhase::Idle,
            environment: None,
            activation_id: None,
            expected_generation: None,
            candidate_generation: None,
            participant_bindings: 0,
            prepared_acks: 0,
            reject_acks: 0,
            stale_acks: 0,
            session_aborts: 0,
            decision: DecisionState::Idle,
            recovery_active: false,
            rebound_participants: 0,
            waiting_replicas: Vec::new(),
            readiness: false,
            mailbox_occupancy: 0,
            mailbox_capacity: 64,
            mailbox_saturation: 0,
            shutdown: false,
            last_failure: None,
        };
        let actor = ActorHealthSnapshot {
            catalog: CatalogHealth::default(),
            ownership: OwnershipHealth::default(),
            activation: ActivationHealth::default(),
            invocation: InvocationHealth::default(),
            control: ControlHealth::default(),
            lease: LeaseHealth::default(),
            task: ActorTaskHealth::default(),
        };
        build_counters(
            &dispatcher,
            &coordinator,
            &actor,
            &IndexHealthSnapshot {
                connection_count: 0,
                open_connections: Vec::new(),
                finalizer_pending: 0,
                finalizer_count: 0,
                finalizer_failures: Vec::new(),
                terminals_by_id: HashMap::new(),
                slow_client_count: 0,
                observed_write_bytes: HashMap::new(),
            },
            &LedgerHealthSnapshot::default(),
            &BrokerHealthSnapshot::default(),
        )
    }

    fn build_counters(
        dispatcher: &DispatcherHealthSnapshot,
        coordinator: &ActivationCoordinatorHealth,
        actor: &ActorHealthSnapshot,
        index: &IndexHealthSnapshot,
        ledger: &LedgerHealthSnapshot,
        broker: &BrokerHealthSnapshot,
    ) -> HealthCounters {
        let mut activation = skiff_router::health::counters::ActivationCounters::from(coordinator);
        activation.repository = skiff_router::health::counters::RepositoryCounters::default();
        HealthCounters {
            active_routing_epoch: skiff_router::health::counters::ActiveRoutingEpochCounters {
                publish_count: 1,
                active: Some(skiff_router::health::counters::ActiveEpochTuple {
                    environment: "prod".to_string(),
                    generation: 7,
                    assembly_identity: tuple().assembly_identity().to_string(),
                    config_snapshot_id: tuple().snapshot_id().to_string(),
                }),
            },
            bootstrap: skiff_router::health::counters::BootstrapCounters {
                reader: skiff_router::health::counters::ReaderFailClosedCountersDto {
                    missing: 0,
                    malformed: 0,
                    identity_mismatch: 0,
                    pending: 0,
                    repository: 0,
                },
            },
            blocking_loader: skiff_router::health::counters::BlockingLoaderCounters {
                concurrency: 8,
                occupancy: 0,
                queued: 0,
                saturated: 0,
                deadline_aborts: 0,
                shutdown_refusals: 0,
                shutdown: false,
            },
            sessions: skiff_router::health::counters::SessionCounters {
                pre_auth_connections: 0,
                pre_auth_refused: 0,
                registered_sessions: 0,
                pending_sessions: 0,
                cancelled_sessions: 0,
                barrier_pending: 0,
                consumer_permits_held: 0,
                live_session_tasks: 0,
            },
            capabilities: skiff_router::health::counters::CapabilityCounters { connections: 0 },
            health: skiff_router::health::counters::HealthObservationCounters {
                observations: 0,
                observed_total: 0,
                health_before_ack: 0,
            },
            barrier: skiff_router::health::counters::BarrierCounters {
                pending: 0,
                permits_held: 0,
                fail_stop: None,
            },
            admission: skiff_router::health::counters::AdmissionCounters::from(dispatcher),
            request_pending: skiff_router::health::counters::RequestPendingCounters {
                unary: dispatcher.pending.unary,
                stream: dispatcher.pending.stream,
                derived_task: dispatcher.pending.derived_task,
                http_pending: 0,
                http_overflow_terminals: 0,
                stopped: dispatcher.stopped,
            },
            terminal: skiff_router::health::counters::TerminalCounters::from(dispatcher),
            client_connections: skiff_router::health::counters::ClientConnectionCounters::from(
                index,
            ),
            generation_leases: skiff_router::health::counters::GenerationLeaseCounters::from(
                ledger,
            ),
            broker: skiff_router::health::counters::BrokerCounters::from(broker),
            actor: skiff_router::health::counters::ActorCounters::from(actor),
            activation,
            http: skiff_router::health::counters::HttpCounters::default(),
            mailboxes: skiff_router::health::counters::MailboxCounters {
                coordinator: skiff_router::health::counters::CoordinatorMailboxCounters {
                    occupancy: 0,
                    capacity: 64,
                    saturation: 0,
                },
            },
            writer_queues: skiff_router::health::counters::WriterQueueCounters {
                ws_slow_client_count: 0,
                ws_observed_write_bytes_total: 0,
            },
            tasks: skiff_router::health::counters::SpawnedTaskCounters {
                live_session_tasks: 0,
                actor_task_capacity_in_use: 0,
                actor_task_accepted: 0,
                actor_task_rejected: 0,
            },
            shutdown: skiff_router::health::counters::ShutdownResidueCounters {
                session_fail_stop: None,
                coordinator_shutdown: false,
                repository_driver_closed: false,
                repository_driver_shutdown_residue: 0,
                dispatcher_stopped: false,
                ws_fail_stop_reason: None,
            },
        }
    }

    #[test]
    fn base_render_keeps_ts_shape_and_exposes_all_counter_sections() {
        let active = ActiveAssemblyProjection {
            environment: "prod".to_string(),
            generation: 7,
            assembly_identity: tuple().assembly_identity().to_string(),
            config_snapshot_id: tuple().snapshot_id().to_string(),
            ingress_count: 0,
        };
        let value = render_base(true, Some(&active), None, &[], &[], &zero_health_counters());
        let object = value.as_object().expect("health must be an object");
        assert_eq!(object.len(), 6, "TS base shape plus counters");
        assert_eq!(value["ok"], true);
        assert!(value["pendingActivation"].is_null());
        assert_eq!(value["replicas"].as_array().map(Vec::len), Some(0));
        for section in [
            "activeRoutingEpoch",
            "bootstrap",
            "blockingLoader",
            "sessions",
            "capabilities",
            "health",
            "barrier",
            "admission",
            "requestPending",
            "terminal",
            "clientConnections",
            "generationLeases",
            "broker",
            "actor",
            "activation",
            "http",
            "mailboxes",
            "writerQueues",
            "tasks",
            "shutdown",
        ] {
            assert!(
                value["counters"][section].is_object(),
                "counters.{section} must be an object"
            );
        }
    }

    #[test]
    fn nonzero_owner_counts_render_into_the_output_layer() {
        let dispatcher = DispatcherHealthSnapshot {
            pending: PendingHealth {
                unary: 1,
                stream: 2,
                derived_task: 3,
            },
            terminal: TerminalHealth {
                by_source: BTreeMap::from([(TerminalSource::Timeout, 4)]),
            },
            admission: AdmissionHealth {
                permits_held: 5,
                ..AdmissionHealth::default()
            },
            task: TaskHealth::default(),
            stopped: false,
        };
        let coordinator = ActivationCoordinatorHealth {
            phase: ActivationPhase::Prepared,
            environment: Some("prod".to_string()),
            activation_id: Some("activation-x".to_string()),
            expected_generation: Some(6),
            candidate_generation: Some(7),
            participant_bindings: 2,
            prepared_acks: 1,
            ..ActivationCoordinatorHealth::default()
        };
        let mut actor = ActorHealthSnapshot::default();
        actor.ownership.current_fences = 3;
        actor.invocation.pending = 4;
        actor.task.actor_invocation_accepted = 6;
        let counters = build_counters(
            &dispatcher,
            &coordinator,
            &actor,
            &IndexHealthSnapshot {
                connection_count: 1,
                open_connections: vec!["conn-1".to_string()],
                ..IndexHealthSnapshot {
                    connection_count: 0,
                    open_connections: Vec::new(),
                    finalizer_pending: 0,
                    finalizer_count: 0,
                    finalizer_failures: Vec::new(),
                    terminals_by_id: HashMap::new(),
                    slow_client_count: 0,
                    observed_write_bytes: HashMap::new(),
                }
            },
            &LedgerHealthSnapshot {
                pins_acquired: 2,
                ..LedgerHealthSnapshot::default()
            },
            &BrokerHealthSnapshot {
                outbound_pending: 1,
                ..BrokerHealthSnapshot::default()
            },
        );
        let value = serde_json::to_value(&counters).expect("counters serialize");
        assert_eq!(value["requestPending"]["unary"], 1);
        assert_eq!(value["requestPending"]["stream"], 2);
        assert_eq!(value["requestPending"]["derivedTask"], 3);
        assert_eq!(value["admission"]["permitsHeld"], 5);
        assert_eq!(value["terminal"]["bySource"]["timeout"], 4);
        assert_eq!(value["actor"]["task"]["actorInvocationAccepted"], 6);
        assert_eq!(value["activation"]["phase"], "prepared");
        assert_eq!(value["activation"]["activationId"], "activation-x");
        assert_eq!(value["actor"]["ownership"]["currentFences"], 3);
        assert_eq!(value["actor"]["invocation"]["pending"], 4);
        assert_eq!(value["clientConnections"]["connectionCount"], 1);
        assert_eq!(value["generationLeases"]["pinsAcquired"], 2);
        assert_eq!(value["broker"]["outboundPending"], 1);
    }

    #[test]
    fn replica_and_capability_projections_match_ts_semantics() {
        let facts = vec![
            SessionFacts {
                session: session("runtime-a"),
                tuple: tuple(),
                registered: true,
                cancelled: false,
            },
            SessionFacts {
                session: session("runtime-b"),
                tuple: tuple(),
                registered: false,
                cancelled: true,
            },
            SessionFacts {
                session: session("runtime-c"),
                tuple: tuple(),
                registered: false,
                cancelled: false,
            },
        ];
        let observations = HashMap::from([
            observation("runtime-a", "2026-08-02T00:00:00.000Z"),
            observation("runtime-b", "2026-08-02T00:00:00.000Z"),
        ]);
        let connected = HashSet::from([session("runtime-a")]);
        let in_flight = HashMap::from([
            ("runtime-a".to_string(), 2_u64),
            ("runtime-c".to_string(), 1_u64),
        ]);
        let replicas = project_replicas(&facts, &observations, &connected, &in_flight);
        assert_eq!(replicas.len(), 2, "pending sessions are excluded");
        let healthy = &replicas[0];
        assert_eq!(healthy.replica_id, "runtime-a");
        assert_eq!(healthy.state, "healthy");
        assert!(healthy.connected);
        assert_eq!(healthy.in_flight_count, 2);
        assert!(healthy.last_health_at.is_some());
        assert!(healthy.health_counters.is_some());
        assert!(!healthy.last_health_at.as_deref().unwrap().is_empty());
        let disconnected = &replicas[1];
        assert_eq!(disconnected.replica_id, "runtime-b");
        assert_eq!(disconnected.state, "disconnected");
        assert!(!disconnected.connected);

        let capabilities = HashMap::from([(
            session("runtime-a"),
            skiff_router::routing::DispatchCapabilities {
                unary: true,
                server_stream: false,
            },
        )]);
        let connections = project_capability_connections(&facts, &capabilities, &connected);
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].runtime_id, "runtime-a");
        assert!(connections[0].connected);
        assert_eq!(connections[0].capabilities.dispatch_modes, ["unary"]);
    }

    #[test]
    fn loop_risk_runtimes_only_include_observed_registered_sessions() {
        let facts = vec![SessionFacts {
            session: session("runtime-a"),
            tuple: tuple(),
            registered: true,
            cancelled: false,
        }];
        let connected = HashSet::from([session("runtime-a")]);
        let runtimes = project_loop_risk_runtimes(
            &facts,
            &HashMap::from([observation("runtime-a", "2026-08-02T00:00:00.000Z")]),
            &connected,
            1_785_628_800_000,
        );
        assert_eq!(runtimes.len(), 1);
        assert!(runtimes[0].fresh);
        let stale = project_loop_risk_runtimes(
            &facts,
            &HashMap::from([observation("runtime-a", "2026-08-02T00:00:00.000Z")]),
            &connected,
            1_785_628_805_001,
        );
        assert!(!stale[0].fresh);

        let value = serde_json::to_value(&LoopRiskProjection {
            observed_at: "2026-08-02T00:00:00.000Z".to_string(),
            router: skiff_router::health::LoopRiskRouterProjection {
                dispatcher: skiff_router::health::LoopRiskDispatcherProjection {
                    pending_unary: 0,
                    pending_stream: 0,
                },
                http_stream: skiff_router::health::LoopRiskHttpStreamProjection {
                    backpressure_waiters: 0,
                    backpressure_cancels: 0,
                },
            },
            runtimes: vec![LoopRiskRuntimeProjection {
                runtime_id: "runtime-a".to_string(),
                connected: true,
                fresh: true,
                counters: zero_counters(),
            }],
        })
        .expect("loopRisk serializes");
        assert_eq!(value["observedAt"], "2026-08-02T00:00:00.000Z");
        assert_eq!(
            value["runtimes"][0]["counters"]["outboundRequestsPending"],
            0
        );
        assert_eq!(value["runtimes"][0]["counters"]["taskRequestsActive"], 0);
    }

    #[test]
    fn replica_projection_omits_registered_at_and_pin_counts_are_zero() {
        let facts = vec![SessionFacts {
            session: session("runtime-a"),
            tuple: tuple(),
            registered: true,
            cancelled: false,
        }];
        let value = serde_json::to_value(project_replicas(
            &facts,
            &HashMap::new(),
            &HashSet::new(),
            &HashMap::new(),
        ))
        .expect("replica serializes");
        let replica = &value[0];
        assert!(replica.get("registeredAt").is_none());
        assert_eq!(replica["connectionPinCount"], 0);
        assert_eq!(replica["connectionReleaseAckCount"], 0);
        assert!(replica.get("lastHealthAt").is_none());
        assert!(replica.get("healthCounters").is_none());
    }
}

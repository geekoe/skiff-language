//! Batch 12 health projection: wire-shape and zero/nonzero counter contract
//! tests at the health output layer. M4: `activeAssembly` is the release
//! pointer table projection; the coordinator/activation/repository counter
//! sections are retired.

use std::collections::{BTreeMap, HashMap, HashSet};

use skiff_router::actor::{
    ActivationHealth, ActorHealthSnapshot, CatalogHealth, ControlHealth, InvocationHealth,
    LeaseHealth, OwnershipHealth,
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
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::ws::{BrokerHealthSnapshot, IndexHealthSnapshot, LedgerHealthSnapshot};
use skiff_runtime_transport::protocol::{
    RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
};

#[cfg(test)]
mod tests {
    use super::*;

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
                schema_version: "skiff-runtime-frame-v4".to_string(),
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
        let actor = ActorHealthSnapshot {
            catalog: CatalogHealth::default(),
            ownership: OwnershipHealth::default(),
            activation: ActivationHealth::default(),
            invocation: InvocationHealth::default(),
            control: ControlHealth::default(),
            lease: LeaseHealth::default(),
        };
        build_counters(
            &dispatcher,
            &actor,
            &skiff_router::health::counters::DurableTaskCounters::default(),
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
        actor: &ActorHealthSnapshot,
        tasks: &skiff_router::health::counters::DurableTaskCounters,
        index: &IndexHealthSnapshot,
        ledger: &LedgerHealthSnapshot,
        broker: &BrokerHealthSnapshot,
    ) -> HealthCounters {
        HealthCounters {
            bootstrap: skiff_router::health::counters::BootstrapCounters {
                profile: "prod".to_string(),
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
                task_attempt: dispatcher.pending.task_attempt,
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
            http: skiff_router::health::counters::HttpCounters::default(),
            mailboxes: skiff_router::health::counters::MailboxCounters::default(),
            writer_queues: skiff_router::health::counters::WriterQueueCounters {
                ws_slow_client_count: 0,
                ws_observed_write_bytes_total: 0,
            },
            tasks: tasks.clone(),
            shutdown: skiff_router::health::counters::ShutdownResidueCounters {
                session_fail_stop: None,
                dispatcher_stopped: false,
                ws_fail_stop_reason: None,
            },
        }
    }

    #[test]
    fn base_render_keeps_ts_shape_and_exposes_all_counter_sections() {
        let active = ActiveAssemblyProjection {
            profile: "prod".to_string(),
            release_count: 1,
            build_ids: vec!["skiff-deployment-artifact-v4:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()],
            loaded_build_ids: Vec::new(),
            router_artifact_root: None,
        };
        let value = render_base(true, Some(&active), &[], &[], &zero_health_counters());
        let object = value.as_object().expect("health must be an object");
        assert_eq!(object.len(), 5, "TS base shape plus counters");
        assert_eq!(value["ok"], true);
        assert_eq!(value["activeAssembly"]["profile"], "prod");
        assert_eq!(value["activeAssembly"]["releaseCount"], 1);
        assert_eq!(value["activeAssembly"]["buildIds"].as_array().map(Vec::len), Some(1));
        assert!(
            value.get("pendingActivation").is_none(),
            "M4 retires pendingActivation"
        );
        assert!(
            value.get("activeRoutingEpoch").is_none(),
            "M4 retires activeRoutingEpoch"
        );
        assert_eq!(value["replicas"].as_array().map(Vec::len), Some(0));
        for section in [
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
                task_attempt: 3,
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
        let mut actor = ActorHealthSnapshot::default();
        actor.ownership.current_fences = 3;
        actor.invocation.pending = 4;
        let tasks = skiff_router::health::counters::DurableTaskCounters {
            live_session_tasks: 1,
            renewing_attempts: 2,
            pending_attempts: 3,
            backlog_ready: 4,
            submissions_accepted: 5,
            settlements_failed: 6,
            ..skiff_router::health::counters::DurableTaskCounters::default()
        };
        let counters = build_counters(
            &dispatcher,
            &actor,
            &tasks,
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
        assert_eq!(value["requestPending"]["taskAttempt"], 3);
        assert_eq!(value["admission"]["permitsHeld"], 5);
        assert_eq!(value["terminal"]["bySource"]["timeout"], 4);
        assert_eq!(value["tasks"]["liveSessionTasks"], 1);
        assert_eq!(value["tasks"]["renewingAttempts"], 2);
        assert_eq!(value["tasks"]["pendingAttempts"], 3);
        assert_eq!(value["tasks"]["backlogReady"], 4);
        assert_eq!(value["tasks"]["submissionsAccepted"], 5);
        assert_eq!(value["tasks"]["settlementsFailed"], 6);
        assert_eq!(value["actor"]["ownership"]["currentFences"], 3);
        assert_eq!(value["actor"]["invocation"]["pending"], 4);
        assert_eq!(value["clientConnections"]["connectionCount"], 1);
        assert_eq!(value["generationLeases"]["pinsAcquired"], 2);
        assert_eq!(value["broker"]["outboundPending"], 1);
        assert!(
            value.get("activation").is_none(),
            "M4 retires the coordinator activation counter section"
        );
    }

    #[test]
    fn replica_and_capability_projections_match_ts_semantics() {
        let facts = vec![
            SessionFacts {
                session: session("runtime-a"),
                registered: true,
                cancelled: false,
                registered_build_ids: Vec::new(),
                lazy_load: false,
                artifact_root: None,
            },
            SessionFacts {
                session: session("runtime-b"),
                registered: false,
                cancelled: true,
                registered_build_ids: Vec::new(),
                lazy_load: false,
                artifact_root: None,
            },
            SessionFacts {
                session: session("runtime-c"),
                registered: false,
                cancelled: false,
                registered_build_ids: Vec::new(),
                lazy_load: false,
                artifact_root: None,
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
            registered: true,
            cancelled: false,
            registered_build_ids: Vec::new(),
            lazy_load: false,
            artifact_root: None,
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
            registered: true,
            cancelled: false,
            registered_build_ids: Vec::new(),
            lazy_load: false,
            artifact_root: None,
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

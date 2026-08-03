//! `HealthAggregator` (authority design §3.2): aggregates read-only snapshots
//! published by every owner and renders the `/__router/health` projection.
//!
//! The aggregator never mutates any owner; it owns no business state besides
//! the optional HTTP gateway health source (the only counter owner created
//! after the aggregator). Per-session replica/capability/loopRisk projections
//! are pure functions over directory facts (plan §3.3: health active/draining
//! is a derived projection, never a second source of truth).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{to_value, Value};

use crate::actor::ActorHealthSnapshot;
use crate::http::HttpGatewayHealth;
use crate::supervisor::RouterComponents;
use crate::telemetry::backlog_metric_event;

use super::counters::{
    ActivationCounters, ActiveRoutingEpochCounters, ActorCounters, AdmissionCounters,
    BarrierCounters, BlockingLoaderCounters, BootstrapCounters, BrokerCounters, CapabilityCounters,
    ClientConnectionCounters, CoordinatorMailboxCounters, GenerationLeaseCounters, HealthCounters,
    HealthObservationCounters, HttpCounters, MailboxCounters, ReaderFailClosedCountersDto,
    RepositoryCounters, RequestPendingCounters, SessionCounters, ShutdownResidueCounters,
    TerminalCounters, WriterQueueCounters, DurableTaskCounters,
};
use super::time::format_iso_millis;
use super::wire::{
    project_capability_connections, project_loop_risk_runtimes, project_replicas, render_base,
    session_facts, ActiveAssemblyProjection, LoopRiskDispatcherProjection,
    LoopRiskHttpStreamProjection, LoopRiskProjection, LoopRiskRouterProjection,
};

type HttpHealthSource = dyn Fn() -> HttpGatewayHealth + Send + Sync;

/// Read-only health projection owner (plan §3.2).
pub struct HealthAggregator {
    components: Arc<RouterComponents>,
    http_health: Mutex<Option<Arc<HttpHealthSource>>>,
}

impl std::fmt::Debug for HealthAggregator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HealthAggregator")
            .field("components", &"RouterComponents")
            .field("http_health", &self.http_health_source_installed())
            .finish()
    }
}

impl HealthAggregator {
    pub fn new(components: Arc<RouterComponents>) -> Arc<Self> {
        Arc::new(Self {
            components,
            http_health: Mutex::new(None),
        })
    }

    /// Installs the live HTTP gateway counter source (the gateway is created
    /// after the aggregator; absent source yields zero counters).
    pub fn set_http_health_source(&self, source: Arc<HttpHealthSource>) {
        *self
            .http_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(source);
    }

    fn http_health_source_installed(&self) -> bool {
        self.http_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    fn http_health(&self) -> HttpGatewayHealth {
        self.http_health
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .map(|source| source())
            .unwrap_or_default()
    }

    /// Renders one health response. `pendingActivation` is the durable
    /// repository pending (read-only; `null` when none or the read fails).
    pub async fn render(&self, with_loop_risk: bool) -> Value {
        let components = &self.components;

        let captured_epoch = components.epoch_store.capture();
        let epoch_store_health = components.epoch_store.health();
        let bootstrap_health = components.assembly.health();
        let session_health = components.session.health_snapshot();
        let dispatcher_health = components.dispatcher.health();
        let index_health = components.ws_lane.index.snapshot();
        let ledger_health = components.ws_lane.ledger.snapshot();
        let broker_health = components.ws_lane.broker.snapshot();
        let coordinator_health = components.coordinator.health();
        let repository_health = components.assembly.repository().health();
        let http_health = self.http_health();

        let actor_health = ActorHealthSnapshot {
            catalog: components.actor.catalog_view.health(),
            ownership: components.actor.registry.health(),
            activation: components.actor.activation_broker.health(),
            invocation: components.actor.relay.health(),
            control: components.actor.control_broker.health(),
            lease: components.actor.lease_scheduler.health(),
        };

        let observations = components.session.health().observations_snapshot();
        let capabilities = components.session.dispatch_capabilities_snapshot();
        // The directory guard is never held across `.await`: the facts,
        // capability and writer sets are owned projections.
        let (facts, connected, in_flight_by_replica) = {
            let directory = components.session.directory_lock();
            let facts = session_facts(&directory);
            let mut connected = HashSet::new();
            for fact in &facts {
                if !fact.cancelled && components.session.has_frame_writer(&fact.session) {
                    connected.insert(fact.session.clone());
                }
            }
            let permit_ledger = components.dispatcher.permit_ledger();
            let mut in_flight_by_replica: HashMap<String, u64> = HashMap::new();
            for (session, count) in &permit_ledger.per_session {
                *in_flight_by_replica
                    .entry(session.replica_id.clone())
                    .or_insert(0) += u64::try_from(*count).unwrap_or(u64::MAX);
            }
            (facts, connected, in_flight_by_replica)
        };

        let replicas = project_replicas(&facts, &observations, &connected, &in_flight_by_replica);
        let capability_connections =
            project_capability_connections(&facts, &capabilities, &connected);

        let now = SystemTime::now();
        let now_millis = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let loop_risk = LoopRiskProjection {
            observed_at: format_iso_millis(now),
            router: LoopRiskRouterProjection {
                dispatcher: LoopRiskDispatcherProjection {
                    // TS parity: task attempts are unary request pending and
                    // count toward the loop-risk unary projection.
                    pending_unary: dispatcher_health.pending.unary
                        + dispatcher_health.pending.task_attempt,
                    pending_stream: dispatcher_health.pending.stream,
                },
                http_stream: LoopRiskHttpStreamProjection {
                    backpressure_waiters: 0,
                    backpressure_cancels: http_health.backpressure_cancels,
                },
            },
            runtimes: project_loop_risk_runtimes(&facts, &observations, &connected, now_millis),
        };

        let active = captured_epoch
            .as_ref()
            .map(|epoch| ActiveAssemblyProjection {
                environment: epoch.environment().to_string(),
                generation: epoch.assembly_generation(),
                assembly_identity: epoch.assembly_identity().to_string(),
                config_snapshot_id: epoch.config_snapshot_id().to_string(),
                ingress_count: epoch.ingress_projection().len(),
            });

        let environment = active
            .as_ref()
            .map(|active| active.environment.clone())
            .or_else(|| {
                epoch_store_health
                    .current
                    .as_ref()
                    .map(|current| current.environment.clone())
            });
        let pending = match environment.as_deref() {
            Some(environment) => components
                .assembly
                .repository()
                .read(environment)
                .await
                .ok()
                .and_then(|state| state.pending),
            None => None,
        };
        let task_backlog = components.task_control.backlog().await;
        components
            .task_telemetry
            .emit(backlog_metric_event(&task_backlog));

        let counters = HealthCounters {
            active_routing_epoch: ActiveRoutingEpochCounters::from(&epoch_store_health),
            bootstrap: BootstrapCounters {
                reader: ReaderFailClosedCountersDto::from(&bootstrap_health.reader_fail_closed),
            },
            blocking_loader: BlockingLoaderCounters::from(&bootstrap_health.loader),
            sessions: SessionCounters::from(&session_health),
            capabilities: CapabilityCounters {
                connections: capabilities.len(),
            },
            health: HealthObservationCounters {
                observations: observations.len(),
                observed_total: session_health.observed_health,
                health_before_ack: session_health.health_before_ack,
            },
            barrier: BarrierCounters {
                pending: session_health.barrier_pending,
                permits_held: session_health.consumer_permits_held,
                fail_stop: session_health.fail_stop.clone(),
            },
            admission: AdmissionCounters::from(&dispatcher_health),
            request_pending: RequestPendingCounters {
                unary: dispatcher_health.pending.unary,
                stream: dispatcher_health.pending.stream,
                task_attempt: dispatcher_health.pending.task_attempt,
                http_pending: components.pending_http.pending_count(),
                http_overflow_terminals: components.pending_http.overflow_terminal_count(),
                stopped: dispatcher_health.stopped,
            },
            terminal: TerminalCounters::from(&dispatcher_health),
            client_connections: ClientConnectionCounters::from(&index_health),
            generation_leases: GenerationLeaseCounters::from(&ledger_health),
            broker: BrokerCounters::from(&broker_health),
            actor: ActorCounters::from(&actor_health),
            activation: {
                let mut activation = ActivationCounters::from(&coordinator_health);
                activation.repository = RepositoryCounters::from(&repository_health);
                activation
            },
            http: HttpCounters::from(&http_health),
            mailboxes: MailboxCounters {
                coordinator: CoordinatorMailboxCounters {
                    occupancy: coordinator_health.mailbox_occupancy,
                    capacity: coordinator_health.mailbox_capacity,
                    saturation: coordinator_health.mailbox_saturation,
                },
            },
            writer_queues: WriterQueueCounters {
                ws_slow_client_count: index_health.slow_client_count,
                ws_observed_write_bytes_total: index_health.observed_write_bytes.values().sum(),
            },
            tasks: DurableTaskCounters {
                live_session_tasks: session_health.live_session_tasks,
                renewing_attempts: components.scheduler.active_lease_count(),
                pending_attempts: components.task_control.pending_attempt_count(),
                backlog_scheduled: task_backlog.scheduled,
                backlog_ready: task_backlog.ready,
                backlog_leased: task_backlog.leased,
                oldest_due_at_ms: task_backlog.oldest_due_at.map(|timestamp| timestamp.millis()),
                submissions_accepted: components
                    .task_control
                    .counters()
                    .submissions_accepted
                    .load(std::sync::atomic::Ordering::Relaxed),
                submissions_rejected: components
                    .task_control
                    .counters()
                    .submissions_rejected
                    .load(std::sync::atomic::Ordering::Relaxed),
                submissions_transient: components
                    .task_control
                    .counters()
                    .submissions_transient
                    .load(std::sync::atomic::Ordering::Relaxed),
                status_queries: components
                    .task_control
                    .counters()
                    .status_queries
                    .load(std::sync::atomic::Ordering::Relaxed),
                status_expired: components
                    .task_control
                    .counters()
                    .status_expired
                    .load(std::sync::atomic::Ordering::Relaxed),
                status_not_found: components
                    .task_control
                    .counters()
                    .status_not_found
                    .load(std::sync::atomic::Ordering::Relaxed),
                status_unavailable: components
                    .task_control
                    .counters()
                    .status_unavailable
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_canceled: components
                    .task_control
                    .counters()
                    .cancel_canceled
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_already_started: components
                    .task_control
                    .counters()
                    .cancel_already_started
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_already_terminal: components
                    .task_control
                    .counters()
                    .cancel_already_terminal
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_expired: components
                    .task_control
                    .counters()
                    .cancel_expired
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_not_found: components
                    .task_control
                    .counters()
                    .cancel_not_found
                    .load(std::sync::atomic::Ordering::Relaxed),
                cancel_unavailable: components
                    .task_control
                    .counters()
                    .cancel_unavailable
                    .load(std::sync::atomic::Ordering::Relaxed),
                settlements_succeeded: components
                    .task_control
                    .counters()
                    .settlements_succeeded
                    .load(std::sync::atomic::Ordering::Relaxed),
                settlements_failed: components
                    .task_control
                    .counters()
                    .settlements_failed
                    .load(std::sync::atomic::Ordering::Relaxed),
                settlements_uncertain: components
                    .task_control
                    .counters()
                    .settlements_uncertain
                    .load(std::sync::atomic::Ordering::Relaxed),
                admissions_accepted: components
                    .task_control
                    .counters()
                    .admissions_accepted
                    .load(std::sync::atomic::Ordering::Relaxed),
                admissions_rejected: components
                    .task_control
                    .counters()
                    .admissions_rejected
                    .load(std::sync::atomic::Ordering::Relaxed),
                admissions_uncertain: components
                    .task_control
                    .counters()
                    .admissions_uncertain
                    .load(std::sync::atomic::Ordering::Relaxed),
                admissions_permanent_failure: components
                    .task_control
                    .counters()
                    .admissions_permanent_failure
                    .load(std::sync::atomic::Ordering::Relaxed),
            },
            shutdown: ShutdownResidueCounters {
                session_fail_stop: session_health.fail_stop,
                coordinator_shutdown: coordinator_health.shutdown,
                repository_driver_closed: repository_health.driver.closed,
                repository_driver_shutdown_residue: repository_health.driver.shutdown_residue,
                dispatcher_stopped: dispatcher_health.stopped,
                ws_fail_stop_reason: ledger_health.fail_stop_reason,
            },
        };

        let mut value = render_base(
            true,
            active.as_ref(),
            pending.as_ref(),
            &capability_connections,
            &replicas,
            &counters,
        );
        if with_loop_risk {
            value["loopRisk"] = to_value(&loop_risk).expect("loopRisk projection serializes");
        }
        value
    }
}

//! §10 counting-surface DTOs (plan §10, batch 12 health leaf).
//!
//! Each section is a read-only projection of exactly one owner's published
//! snapshot; `HealthAggregator` never mutates any owner. Fields that an owner
//! does not publish are omitted from the section (documented in the leaf)
//! rather than fabricated.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::activation::{ActivationCoordinatorHealth, ActivationRepositoryHealth, DecisionState};
use crate::actor::{
    ActivationHealth, ActorHealthSnapshot, CatalogHealth, ControlHealth, InvocationHealth,
    LeaseHealth, OwnershipHealth, SpawnHealth as ActorSpawnHealth,
};
use crate::bootstrap::{BlockingLoaderHealth, ReaderFailClosedCounters};
use crate::dispatch::DispatcherHealthSnapshot;
use crate::session::layer::SessionHealthSnapshot;
use crate::session::RuntimeSessionEpoch;
use crate::ws::{BrokerHealthSnapshot, IndexHealthSnapshot, LedgerHealthSnapshot};

/// Complete §10 counters object (`counters`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCounters {
    pub active_routing_epoch: ActiveRoutingEpochCounters,
    pub bootstrap: BootstrapCounters,
    pub blocking_loader: BlockingLoaderCounters,
    pub sessions: SessionCounters,
    pub capabilities: CapabilityCounters,
    pub health: HealthObservationCounters,
    pub barrier: BarrierCounters,
    pub admission: AdmissionCounters,
    pub request_pending: RequestPendingCounters,
    pub terminal: TerminalCounters,
    pub client_connections: ClientConnectionCounters,
    pub generation_leases: GenerationLeaseCounters,
    pub broker: BrokerCounters,
    pub actor: ActorCounters,
    pub activation: ActivationCounters,
    pub http: HttpCounters,
    pub mailboxes: MailboxCounters,
    pub writer_queues: WriterQueueCounters,
    pub spawned_tasks: SpawnedTaskCounters,
    pub shutdown: ShutdownResidueCounters,
}

/// `counters.activeRoutingEpoch` (owner: `ActiveRoutingEpochStore`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRoutingEpochCounters {
    pub publish_count: u64,
    pub active: Option<ActiveEpochTuple>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveEpochTuple {
    pub environment: String,
    pub generation: u64,
    pub assembly_identity: String,
    pub config_snapshot_id: String,
}

/// `counters.bootstrap` (owner: `RouterBootstrapAssembly`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapCounters {
    pub reader: ReaderFailClosedCountersDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFailClosedCountersDto {
    pub missing: u64,
    pub malformed: u64,
    pub identity_mismatch: u64,
    pub pending: u64,
    pub repository: u64,
}

/// `counters.blockingLoader` (owner: `BlockingLoader`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockingLoaderCounters {
    pub concurrency: usize,
    pub occupancy: usize,
    pub queued: u64,
    pub saturated: u64,
    pub deadline_aborts: u64,
    pub shutdown_refusals: u64,
    pub shutdown: bool,
}

/// `counters.sessions` (owner: `SessionLayer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCounters {
    pub pre_auth_connections: usize,
    pub pre_auth_refused: u64,
    pub registered_sessions: usize,
    pub pending_sessions: usize,
    pub cancelled_sessions: usize,
    pub barrier_pending: usize,
    pub consumer_permits_held: usize,
    pub live_session_tasks: usize,
}

/// `counters.capabilities` (derived from session capability bindings).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCounters {
    pub connections: usize,
}

/// `counters.health` (owner: `RuntimeHealthLedger`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthObservationCounters {
    pub observations: usize,
    pub observed_total: u64,
    pub health_before_ack: u64,
}

/// `counters.barrier` (owner: session close barrier).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BarrierCounters {
    pub pending: usize,
    pub permits_held: usize,
    pub fail_stop: Option<String>,
}

/// `counters.admission` (owner: `RuntimeAdmissionPool` via dispatcher).
///
/// The selection cursor is not published by the dispatcher health snapshot;
/// permits and counters are the owner's real values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionCounters {
    pub permits_held: usize,
    pub releases: u64,
    pub queue_full_rejects: u64,
    pub revalidate_failures: u64,
    pub reselects: u64,
    pub no_candidate_rejects: u64,
    pub duplicate_request_id_rejects: u64,
}

/// `counters.requestPending` (owners: dispatcher + composition HTTP router).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPendingCounters {
    pub unary: u64,
    pub stream: u64,
    pub derived_spawn: u64,
    pub http_pending: usize,
    pub http_overflow_terminals: u64,
    pub stopped: bool,
}

/// `counters.terminal` (owner: dispatcher terminal ledger).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalCounters {
    pub by_source: BTreeMap<String, u64>,
}

/// `counters.clientConnections` (owner: `ClientConnectionIndex`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientConnectionCounters {
    pub connection_count: usize,
    pub open_connections: Vec<String>,
    pub finalizer_pending: usize,
    pub finalizer_count: u64,
    pub finalizer_failures: Vec<String>,
    pub slow_client_count: u64,
}

/// `counters.generationLeases` (owner: `RuntimeGenerationPinLedger`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationLeaseCounters {
    pub pins_acquired: usize,
    pub pins_pending_release: usize,
    pub cached_acquire_count: usize,
    pub release_acks: u64,
    pub release_failures: Vec<String>,
    pub runtime_closed: Vec<String>,
}

/// `counters.broker` (owner: `WebSocketRequestBroker`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokerCounters {
    pub generation_count: usize,
    pub outbound_pending: usize,
    pub inbound_pending: usize,
    pub outbound_tombstones: usize,
    pub inbound_tombstones: usize,
    pub timer_count: usize,
    pub protocol_violations: usize,
    pub runtime_disconnect_detached: u64,
}

/// `counters.actor` (owners: the six actor lane owners).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorCounters {
    pub catalog: CatalogHealthDto,
    pub ownership: OwnershipHealthDto,
    pub activation: ActivationHealthDto,
    pub invocation: InvocationHealthDto,
    pub control: ControlHealthDto,
    pub lease: LeaseHealthDto,
    pub spawn: ActorSpawnHealthDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogHealthDto {
    pub captures: u64,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipHealthDto {
    pub current_fences: usize,
    pub in_flight_reservations: usize,
    pub commits: u64,
    pub aborts: u64,
    pub conflicts: u64,
    pub renewals: u64,
    pub releases: u64,
    pub expired: u64,
    pub epoch_mismatches: u64,
    pub rejected_commits: u64,
    pub rejected_aborts: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationHealthDto {
    pub pending_claims: usize,
    pub pending_waiters: usize,
    pub dedup_joins: u64,
    pub lineage_conflicts: u64,
    pub commits: u64,
    pub aborts: u64,
    pub timeouts: u64,
    pub late_acks: u64,
    pub wrong_correlation: u64,
    pub saturated: u64,
    pub tombstones: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvocationHealthDto {
    pub pending: usize,
    pub capacity: usize,
    pub settled: u64,
    pub rejected: u64,
    pub terminals: u64,
    pub deadline_cancels: u64,
    pub saturated: u64,
    pub tombstones: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlHealthDto {
    pub pending: usize,
    pub accepted: u64,
    pub rejected: u64,
    pub late_acks: u64,
    pub timeouts: u64,
    pub wrong_correlation: u64,
    pub disconnects: u64,
    pub saturated: u64,
    pub tombstones: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseHealthDto {
    pub sweep_count: u64,
    pub expired: u64,
    pub idle_candidates: u64,
    pub eviction_pending: usize,
    pub eviction_acked: u64,
    pub eviction_retries: u64,
    pub eviction_exhausted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorSpawnHealthDto {
    pub capacity_in_use: usize,
    pub accepted: u64,
    pub rejected: u64,
    pub legacy_rejected: u64,
    pub request_accepted: u64,
    pub actor_invocation_accepted: u64,
    pub by_error: BTreeMap<String, u64>,
}

/// `counters.activation` (owners: coordinator + repository).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationCounters {
    pub phase: String,
    pub environment: Option<String>,
    pub activation_id: Option<String>,
    pub expected_generation: Option<u64>,
    pub candidate_generation: Option<u64>,
    pub participant_bindings: usize,
    pub prepared_acks: usize,
    pub reject_acks: usize,
    pub stale_acks: u64,
    pub session_aborts: u64,
    pub decision: String,
    pub recovery_active: bool,
    pub rebound_participants: usize,
    pub waiting_replicas: Vec<String>,
    pub readiness: bool,
    pub mailbox_occupancy: usize,
    pub mailbox_capacity: usize,
    pub mailbox_saturation: u64,
    pub shutdown: bool,
    pub last_failure: Option<String>,
    pub repository: RepositoryCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryCounters {
    pub environment: Option<String>,
    pub committed_generation: Option<u64>,
    pub pending_activation_id: Option<String>,
    pub last_outcome: Option<String>,
    pub last_outcome_operation: Option<String>,
    pub retry: RetryCounters,
    pub audit: AuditCounters,
    pub driver: DriverCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryCounters {
    pub attempts: u32,
    pub retried: u32,
    pub next_backoff_ms: u64,
    pub deadline_remaining_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditCounters {
    pub last_event_id: Option<String>,
    pub last_event_operation: Option<String>,
    pub last_event_timestamp: Option<i64>,
    pub failed_writes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverCounters {
    pub connected: bool,
    pub reconnecting: bool,
    pub closed: bool,
    pub shutdown_residue: u32,
}

/// `counters.http` (owner: `HttpGatewayServer`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpCounters {
    pub requests: u64,
    pub unary_dispatches: u64,
    pub stream_dispatches: u64,
    pub cors_preflights: u64,
    pub service_managed_cors: u64,
    pub selector_rejects: u64,
    pub ingress_misses: u64,
    pub request_too_large: u64,
    pub response_too_large: u64,
    pub backpressure_cancels: u64,
    pub client_disconnect_cancels: u64,
    pub timeouts: u64,
    pub platform_errors: u64,
}

/// `counters.mailboxes` (only the coordinator publishes mailbox occupancy).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxCounters {
    pub coordinator: CoordinatorMailboxCounters,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinatorMailboxCounters {
    pub occupancy: usize,
    pub capacity: usize,
    pub saturation: u64,
}

/// `counters.writerQueues` (WS index publishes observed write bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterQueueCounters {
    pub ws_slow_client_count: u64,
    pub ws_observed_write_bytes_total: u64,
}

/// `counters.spawnedTasks` (session tasks + actor spawn capacity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpawnedTaskCounters {
    pub live_session_tasks: usize,
    pub actor_spawn_capacity_in_use: usize,
    pub actor_spawn_accepted: u64,
    pub actor_spawn_rejected: u64,
}

/// `counters.shutdown` (shutdown residue across owners).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownResidueCounters {
    pub session_fail_stop: Option<String>,
    pub coordinator_shutdown: bool,
    pub repository_driver_closed: bool,
    pub repository_driver_shutdown_residue: u32,
    pub dispatcher_stopped: bool,
    pub ws_fail_stop_reason: Option<String>,
}

impl From<&crate::bootstrap::EpochStoreHealth> for ActiveRoutingEpochCounters {
    fn from(health: &crate::bootstrap::EpochStoreHealth) -> Self {
        Self {
            publish_count: health.publish_count,
            active: health.current.as_ref().map(|current| ActiveEpochTuple {
                environment: current.environment.clone(),
                generation: current.assembly_generation,
                assembly_identity: current.assembly_identity.clone(),
                config_snapshot_id: current.config_snapshot_id.clone(),
            }),
        }
    }
}

impl From<&ReaderFailClosedCounters> for ReaderFailClosedCountersDto {
    fn from(health: &ReaderFailClosedCounters) -> Self {
        Self {
            missing: health.missing,
            malformed: health.malformed,
            identity_mismatch: health.identity_mismatch,
            pending: health.pending,
            repository: health.repository,
        }
    }
}

impl From<&BlockingLoaderHealth> for BlockingLoaderCounters {
    fn from(health: &BlockingLoaderHealth) -> Self {
        Self {
            concurrency: health.concurrency,
            occupancy: health.occupancy,
            queued: health.queued,
            saturated: health.saturated,
            deadline_aborts: health.deadline_aborts,
            shutdown_refusals: health.shutdown_refusals,
            shutdown: health.shutdown,
        }
    }
}

impl From<&SessionHealthSnapshot> for SessionCounters {
    fn from(health: &SessionHealthSnapshot) -> Self {
        Self {
            pre_auth_connections: health.pre_auth_connections,
            pre_auth_refused: health.pre_auth_refused,
            registered_sessions: health.registered_sessions,
            pending_sessions: health.pending_sessions,
            cancelled_sessions: health.cancelled_sessions,
            barrier_pending: health.barrier_pending,
            consumer_permits_held: health.consumer_permits_held,
            live_session_tasks: health.live_session_tasks,
        }
    }
}

impl From<&DispatcherHealthSnapshot> for AdmissionCounters {
    fn from(health: &DispatcherHealthSnapshot) -> Self {
        Self {
            permits_held: health.admission.permits_held,
            releases: health.admission.releases,
            queue_full_rejects: health.admission.queue_full_rejects,
            revalidate_failures: health.admission.revalidate_failures,
            reselects: health.admission.reselects,
            no_candidate_rejects: health.admission.no_candidate_rejects,
            duplicate_request_id_rejects: health.admission.duplicate_request_id_rejects,
        }
    }
}

impl From<&DispatcherHealthSnapshot> for RequestPendingCounters {
    fn from(health: &DispatcherHealthSnapshot) -> Self {
        Self {
            unary: health.pending.unary,
            stream: health.pending.stream,
            derived_spawn: health.pending.derived_spawn,
            http_pending: 0,
            http_overflow_terminals: 0,
            stopped: health.stopped,
        }
    }
}

impl From<&DispatcherHealthSnapshot> for TerminalCounters {
    fn from(health: &DispatcherHealthSnapshot) -> Self {
        Self {
            by_source: health
                .terminal
                .by_source
                .iter()
                .map(|(source, count)| (source.as_str().to_string(), *count))
                .collect(),
        }
    }
}

impl From<&IndexHealthSnapshot> for ClientConnectionCounters {
    fn from(health: &IndexHealthSnapshot) -> Self {
        Self {
            connection_count: health.connection_count,
            open_connections: health.open_connections.clone(),
            finalizer_pending: health.finalizer_pending,
            finalizer_count: health.finalizer_count,
            finalizer_failures: health.finalizer_failures.clone(),
            slow_client_count: health.slow_client_count,
        }
    }
}

impl From<&LedgerHealthSnapshot> for GenerationLeaseCounters {
    fn from(health: &LedgerHealthSnapshot) -> Self {
        Self {
            pins_acquired: health.pins_acquired,
            pins_pending_release: health.pins_pending_release,
            cached_acquire_count: health.cached_acquire_count,
            release_acks: health.release_acks,
            release_failures: health.release_failures.clone(),
            runtime_closed: health.runtime_closed.iter().map(session_label).collect(),
        }
    }
}

impl From<&BrokerHealthSnapshot> for BrokerCounters {
    fn from(health: &BrokerHealthSnapshot) -> Self {
        Self {
            generation_count: health.generation_count,
            outbound_pending: health.outbound_pending,
            inbound_pending: health.inbound_pending,
            outbound_tombstones: health.outbound_tombstones,
            inbound_tombstones: health.inbound_tombstones,
            timer_count: health.timer_count,
            protocol_violations: health.protocol_violations,
            runtime_disconnect_detached: health.runtime_disconnect_detached,
        }
    }
}

impl From<&ActorHealthSnapshot> for ActorCounters {
    fn from(health: &ActorHealthSnapshot) -> Self {
        Self {
            catalog: CatalogHealthDto::from(&health.catalog),
            ownership: OwnershipHealthDto::from(&health.ownership),
            activation: ActivationHealthDto::from(&health.activation),
            invocation: InvocationHealthDto::from(&health.invocation),
            control: ControlHealthDto::from(&health.control),
            lease: LeaseHealthDto::from(&health.lease),
            spawn: ActorSpawnHealthDto::from(&health.spawn),
        }
    }
}

impl From<&CatalogHealth> for CatalogHealthDto {
    fn from(health: &CatalogHealth) -> Self {
        Self {
            captures: health.captures,
            hits: health.hits,
            misses: health.misses,
        }
    }
}

impl From<&OwnershipHealth> for OwnershipHealthDto {
    fn from(health: &OwnershipHealth) -> Self {
        Self {
            current_fences: health.current_fences,
            in_flight_reservations: health.in_flight_reservations,
            commits: health.commits,
            aborts: health.aborts,
            conflicts: health.conflicts,
            renewals: health.renewals,
            releases: health.releases,
            expired: health.expired,
            epoch_mismatches: health.epoch_mismatches,
            rejected_commits: health.rejected_commits,
            rejected_aborts: health.rejected_aborts,
        }
    }
}

impl From<&ActivationHealth> for ActivationHealthDto {
    fn from(health: &ActivationHealth) -> Self {
        Self {
            pending_claims: health.pending_claims,
            pending_waiters: health.pending_waiters,
            dedup_joins: health.dedup_joins,
            lineage_conflicts: health.lineage_conflicts,
            commits: health.commits,
            aborts: health.aborts,
            timeouts: health.timeouts,
            late_acks: health.late_acks,
            wrong_correlation: health.wrong_correlation,
            saturated: health.saturated,
            tombstones: health.tombstones,
        }
    }
}

impl From<&InvocationHealth> for InvocationHealthDto {
    fn from(health: &InvocationHealth) -> Self {
        Self {
            pending: health.pending,
            capacity: health.capacity,
            settled: health.settled,
            rejected: health.rejected,
            terminals: health.terminals,
            deadline_cancels: health.deadline_cancels,
            saturated: health.saturated,
            tombstones: health.tombstones,
        }
    }
}

impl From<&ControlHealth> for ControlHealthDto {
    fn from(health: &ControlHealth) -> Self {
        Self {
            pending: health.pending,
            accepted: health.accepted,
            rejected: health.rejected,
            late_acks: health.late_acks,
            timeouts: health.timeouts,
            wrong_correlation: health.wrong_correlation,
            disconnects: health.disconnects,
            saturated: health.saturated,
            tombstones: health.tombstones,
        }
    }
}

impl From<&LeaseHealth> for LeaseHealthDto {
    fn from(health: &LeaseHealth) -> Self {
        Self {
            sweep_count: health.sweep_count,
            expired: health.expired,
            idle_candidates: health.idle_candidates,
            eviction_pending: health.eviction_pending,
            eviction_acked: health.eviction_acked,
            eviction_retries: health.eviction_retries,
            eviction_exhausted: health.eviction_exhausted,
        }
    }
}

impl From<&ActorSpawnHealth> for ActorSpawnHealthDto {
    fn from(health: &ActorSpawnHealth) -> Self {
        Self {
            capacity_in_use: health.capacity_in_use,
            accepted: health.accepted,
            rejected: health.rejected,
            legacy_rejected: health.legacy_rejected,
            request_accepted: health.request_accepted,
            actor_invocation_accepted: health.actor_invocation_accepted,
            by_error: health.by_error.clone(),
        }
    }
}

impl From<&ActivationCoordinatorHealth> for ActivationCounters {
    fn from(health: &ActivationCoordinatorHealth) -> Self {
        Self {
            phase: health.phase.as_str().to_string(),
            environment: health.environment.clone(),
            activation_id: health.activation_id.clone(),
            expected_generation: health.expected_generation,
            candidate_generation: health.candidate_generation,
            participant_bindings: health.participant_bindings,
            prepared_acks: health.prepared_acks,
            reject_acks: health.reject_acks,
            stale_acks: health.stale_acks,
            session_aborts: health.session_aborts,
            decision: decision_str(health.decision),
            recovery_active: health.recovery_active,
            rebound_participants: health.rebound_participants,
            waiting_replicas: health.waiting_replicas.clone(),
            readiness: health.readiness,
            mailbox_occupancy: health.mailbox_occupancy,
            mailbox_capacity: health.mailbox_capacity,
            mailbox_saturation: health.mailbox_saturation,
            shutdown: health.shutdown,
            last_failure: health.last_failure.clone(),
            repository: RepositoryCounters::default(),
        }
    }
}

impl Default for RepositoryCounters {
    fn default() -> Self {
        Self {
            environment: None,
            committed_generation: None,
            pending_activation_id: None,
            last_outcome: None,
            last_outcome_operation: None,
            retry: RetryCounters {
                attempts: 0,
                retried: 0,
                next_backoff_ms: 0,
                deadline_remaining_ms: None,
            },
            audit: AuditCounters {
                last_event_id: None,
                last_event_operation: None,
                last_event_timestamp: None,
                failed_writes: 0,
            },
            driver: DriverCounters {
                connected: false,
                reconnecting: false,
                closed: false,
                shutdown_residue: 0,
            },
        }
    }
}

impl From<&ActivationRepositoryHealth> for RepositoryCounters {
    fn from(health: &ActivationRepositoryHealth) -> Self {
        Self {
            environment: health.environment.clone(),
            committed_generation: health.committed_generation,
            pending_activation_id: health.pending_activation_id.clone(),
            last_outcome: health
                .last_outcome
                .map(|outcome| outcome.as_str().to_string()),
            last_outcome_operation: health.last_outcome_operation.clone(),
            retry: RetryCounters {
                attempts: health.retry.attempts,
                retried: health.retry.retried,
                next_backoff_ms: health.retry.next_backoff_ms,
                deadline_remaining_ms: health.retry.deadline_remaining_ms,
            },
            audit: AuditCounters {
                last_event_id: health.audit.last_event_id.clone(),
                last_event_operation: health.audit.last_event_operation.clone(),
                last_event_timestamp: health.audit.last_event_timestamp,
                failed_writes: health.audit.failed_writes,
            },
            driver: DriverCounters {
                connected: health.driver.connected,
                reconnecting: health.driver.reconnecting,
                closed: health.driver.closed,
                shutdown_residue: health.driver.shutdown_residue,
            },
        }
    }
}

impl From<&crate::http::HttpGatewayHealth> for HttpCounters {
    fn from(health: &crate::http::HttpGatewayHealth) -> Self {
        Self {
            requests: health.requests,
            unary_dispatches: health.unary_dispatches,
            stream_dispatches: health.stream_dispatches,
            cors_preflights: health.cors_preflights,
            service_managed_cors: health.service_managed_cors,
            selector_rejects: health.selector_rejects,
            ingress_misses: health.ingress_misses,
            request_too_large: health.request_too_large,
            response_too_large: health.response_too_large,
            backpressure_cancels: health.backpressure_cancels,
            client_disconnect_cancels: health.client_disconnect_cancels,
            timeouts: health.timeouts,
            platform_errors: health.platform_errors,
        }
    }
}

fn decision_str(decision: DecisionState) -> String {
    decision.as_str().to_string()
}

fn session_label(session: &RuntimeSessionEpoch) -> String {
    format!("{}#{}", session.replica_id, session.connection_generation)
}

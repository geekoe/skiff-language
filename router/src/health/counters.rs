//! §10 counting-surface DTOs (plan §10, batch 12 health leaf).
//!
//! Each section is a read-only projection of exactly one owner's published
//! snapshot; `HealthAggregator` never mutates any owner. Fields that an owner
//! does not publish are omitted from the section (documented in the leaf)
//! rather than fabricated.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::actor::{
    ActivationHealth, ActorHealthSnapshot, CatalogHealth, ControlHealth, InvocationHealth,
    LeaseHealth, OwnershipHealth,
};
use crate::bootstrap::BlockingLoaderHealth;
use crate::dispatch::DispatcherHealthSnapshot;
use crate::session::layer::SessionHealthSnapshot;
use crate::ws::{BrokerHealthSnapshot, IndexHealthSnapshot};

/// Complete §10 counters object (`counters`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthCounters {
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
    pub http: HttpCounters,
    pub mailboxes: MailboxCounters,
    pub writer_queues: WriterQueueCounters,
    pub tasks: DurableTaskCounters,
    pub shutdown: ShutdownResidueCounters,
}

/// `counters.bootstrap` (owner: `RouterBootstrapAssembly`; M4: artifact
/// store + profile only, no reader).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapCounters {
    pub profile: String,
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
    pub task_attempt: u64,
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

/// `counters.generationLeases` (owner: retired `RuntimeGenerationPinLedger`;
/// kept zeroed for wire-shape stability — the broker connection registry is
/// the router-side accounting authority).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxCounters {}

/// `counters.writerQueues` (WS index publishes observed write bytes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterQueueCounters {
    pub ws_slow_client_count: u64,
    pub ws_observed_write_bytes_total: u64,
}

/// `counters.tasks`: session tasks + durable task control plane projection
/// (authoritative design "Observability And Retention": leased count, backlog
/// depth, oldest eligible age, submission/status/cancel/settlement counters).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableTaskCounters {
    pub live_session_tasks: usize,
    pub renewing_attempts: usize,
    pub pending_attempts: usize,
    pub backlog_scheduled: usize,
    pub backlog_ready: usize,
    pub backlog_leased: usize,
    pub oldest_due_at_ms: Option<i64>,
    pub submissions_accepted: u64,
    pub submissions_rejected: u64,
    pub submissions_transient: u64,
    pub status_queries: u64,
    pub status_expired: u64,
    pub status_not_found: u64,
    pub status_unavailable: u64,
    pub cancel_canceled: u64,
    pub cancel_already_started: u64,
    pub cancel_already_terminal: u64,
    pub cancel_expired: u64,
    pub cancel_not_found: u64,
    pub cancel_unavailable: u64,
    pub settlements_succeeded: u64,
    pub settlements_failed: u64,
    pub settlements_uncertain: u64,
    pub admissions_accepted: u64,
    pub admissions_rejected: u64,
    pub admissions_uncertain: u64,
    pub admissions_permanent_failure: u64,
}

/// `counters.shutdown` (shutdown residue across owners).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownResidueCounters {
    pub session_fail_stop: Option<String>,
    pub dispatcher_stopped: bool,
    pub ws_fail_stop_reason: Option<String>,
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
            task_attempt: health.pending.task_attempt,
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

//! Aggregate health snapshot of the W-actor lane (C-actor §7).
//!
//! Health never exposes payload bytes, actor key bytes, Mongo URLs or
//! secrets; it only carries occupancy/counter projections.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogHealth {
    pub captures: u64,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OwnershipHealth {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActivationHealth {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InvocationHealth {
    pub pending: usize,
    pub capacity: usize,
    pub settled: u64,
    pub rejected: u64,
    pub terminals: u64,
    pub deadline_cancels: u64,
    pub saturated: u64,
    pub tombstones: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlHealth {
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LeaseHealth {
    pub sweep_count: u64,
    pub expired: u64,
    pub idle_candidates: u64,
    pub eviction_pending: usize,
    pub eviction_acked: u64,
    pub eviction_retries: u64,
    pub eviction_exhausted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskHealth {
    pub capacity_in_use: usize,
    pub accepted: u64,
    pub rejected: u64,
    pub legacy_rejected: u64,
    pub request_accepted: u64,
    pub actor_invocation_accepted: u64,
    pub by_error: BTreeMap<String, u64>,
}

/// Combined health projection of the W-actor lane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActorHealthSnapshot {
    pub catalog: CatalogHealth,
    pub ownership: OwnershipHealth,
    pub activation: ActivationHealth,
    pub invocation: InvocationHealth,
    pub control: ControlHealth,
    pub lease: LeaseHealth,
    pub task: TaskHealth,
}

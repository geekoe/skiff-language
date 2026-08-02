//! `ActorOwnerControlBroker`: claim / renew / evict owner-control
//! correlation (plan §3.2, C-actor §3.5/§7, C-model-actor §7).
//!
//! The broker owns only request/ACK correlation: pending keyed by request id,
//! exact operation + runtime id + connection, deadline timeout and bounded
//! late-ACK tombstones. It does not interpret idle timing and never mutates
//! registry truth.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::actor_owner::ActorOwnerControlOperation;

use super::health::ControlHealth;
use super::types::{
    ActorOwnerFence, ActorOwnerRouteAuthority, DEFAULT_ACTOR_PENDING_BUDGET,
    DEFAULT_ACTOR_TOMBSTONE_BUDGET, DEFAULT_CONTROL_ACK_DEADLINE_MS,
};

/// Broker construction options (C-actor §4).
#[derive(Debug, Clone)]
pub struct ControlBrokerOptions {
    pub ack_deadline_ms: u64,
    pub max_pending: usize,
    pub max_tombstones: usize,
}

impl Default for ControlBrokerOptions {
    fn default() -> Self {
        Self {
            ack_deadline_ms: DEFAULT_CONTROL_ACK_DEADLINE_MS,
            max_pending: DEFAULT_ACTOR_PENDING_BUDGET,
            max_tombstones: DEFAULT_ACTOR_TOMBSTONE_BUDGET,
        }
    }
}

/// One owner-control request entering the broker (C-actor §3.5).
#[derive(Debug, Clone)]
pub struct OwnerControlRequest {
    pub request_id: String,
    pub operation: ActorOwnerControlOperation,
    pub runtime_id: String,
    pub connection: String,
    pub fence: ActorOwnerFence,
    pub route_authority: ActorOwnerRouteAuthority,
    pub deadline_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlError {
    Duplicate,
    Saturated,
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate => write!(formatter, "control request already pending"),
            Self::Saturated => write!(formatter, "control broker saturated"),
        }
    }
}

impl std::error::Error for ControlError {}

/// Result of correlating one owner-control ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAckOutcome {
    Accepted,
    Rejected,
    LateAck,
    WrongCorrelation,
    Unknown,
}

/// Result of an ACK deadline timeout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlTimeoutOutcome {
    TimedOut { request_id: String },
    Unknown,
}

#[derive(Debug, Clone)]
struct PendingControl {
    operation: ActorOwnerControlOperation,
    runtime_id: String,
    connection: String,
    fence: ActorOwnerFence,
    route_authority: ActorOwnerRouteAuthority,
    deadline_at: u64,
}

#[derive(Debug, Default)]
struct ControlCounters {
    accepted: u64,
    rejected: u64,
    late_acks: u64,
    timeouts: u64,
    wrong_correlation: u64,
    disconnects: u64,
    saturated: u64,
}

#[derive(Debug, Default)]
struct ControlInner {
    pending: BTreeMap<String, PendingControl>,
    tombstones: BTreeSet<String>,
    outcomes: BTreeMap<String, bool>,
    counters: ControlCounters,
}

/// Owner-control correlation owner (C-actor §2).
#[derive(Debug)]
pub struct ActorOwnerControlBroker {
    inner: Arc<Mutex<ControlInner>>,
    options: ControlBrokerOptions,
}

impl ActorOwnerControlBroker {
    pub fn new(options: ControlBrokerOptions) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ControlInner::default())),
            options,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ControlInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Registers one owner-control pending (non-blocking; bounded).
    pub fn send_control(&self, request: &OwnerControlRequest) -> Result<(), ControlError> {
        let mut inner = self.lock();
        if inner.pending.contains_key(&request.request_id) {
            return Err(ControlError::Duplicate);
        }
        if inner.pending.len() >= self.options.max_pending {
            inner.counters.saturated += 1;
            return Err(ControlError::Saturated);
        }
        inner.pending.insert(
            request.request_id.clone(),
            PendingControl {
                operation: request.operation,
                runtime_id: request.runtime_id.clone(),
                connection: request.connection.clone(),
                fence: request.fence.clone(),
                route_authority: request.route_authority.clone(),
                deadline_at: request.deadline_at,
            },
        );
        Ok(())
    }

    /// ACK correlation (C-actor §7): request id + runtime id + operation +
    /// connection must all match; settled ids only enter the tombstone.
    pub fn on_ack(
        &self,
        runtime_id: &str,
        request_id: &str,
        operation: ActorOwnerControlOperation,
        connection: &str,
        accepted: bool,
    ) -> ControlAckOutcome {
        let mut inner = self.lock();
        let Some(pending) = inner.pending.get(request_id) else {
            if inner.tombstones.contains(request_id) {
                inner.counters.late_acks += 1;
                inner.counters.rejected += 1;
                return ControlAckOutcome::LateAck;
            }
            inner.counters.wrong_correlation += 1;
            inner.counters.rejected += 1;
            return ControlAckOutcome::Unknown;
        };
        if pending.runtime_id != runtime_id
            || pending.operation != operation
            || pending.connection != connection
        {
            inner.counters.wrong_correlation += 1;
            inner.counters.rejected += 1;
            return ControlAckOutcome::WrongCorrelation;
        }
        inner.pending.remove(request_id);
        inner.tombstones.insert(request_id.to_string());
        inner.outcomes.insert(request_id.to_string(), accepted);
        if accepted {
            inner.counters.accepted += 1;
            ControlAckOutcome::Accepted
        } else {
            inner.counters.rejected += 1;
            ControlAckOutcome::Rejected
        }
    }

    /// ACK deadline timeout: resolve false (C-actor §6).
    pub fn timeout(&self, request_id: &str) -> ControlTimeoutOutcome {
        let mut inner = self.lock();
        if inner.pending.remove(request_id).is_some() {
            inner.tombstones.insert(request_id.to_string());
            inner.counters.timeouts += 1;
            inner.outcomes.insert(request_id.to_string(), false);
            ControlTimeoutOutcome::TimedOut {
                request_id: request_id.to_string(),
            }
        } else {
            ControlTimeoutOutcome::Unknown
        }
    }

    /// Deadline sweep: expires every pending whose ACK deadline elapsed.
    pub fn expire_deadlines(&self, now: u64) -> Vec<ControlTimeoutOutcome> {
        let request_ids = {
            let inner = self.lock();
            inner
                .pending
                .iter()
                .filter(|(_, pending)| pending.deadline_at <= now)
                .map(|(request_id, _)| request_id.clone())
                .collect::<Vec<_>>()
        };
        request_ids
            .into_iter()
            .map(|request_id| self.timeout(&request_id))
            .collect()
    }

    /// Owner runtime disconnect: every pending on the exact connection
    /// resolves false (C-actor §6).
    pub fn on_owner_disconnect(
        &self,
        runtime_id: &str,
        connection: &str,
    ) -> Vec<ControlTimeoutOutcome> {
        let mut inner = self.lock();
        let ids = inner
            .pending
            .iter()
            .filter(|(_, pending)| {
                pending.runtime_id == runtime_id && pending.connection == connection
            })
            .map(|(request_id, _)| request_id.clone())
            .collect::<Vec<_>>();
        let mut outcomes = Vec::new();
        for request_id in ids {
            inner.pending.remove(&request_id);
            inner.tombstones.insert(request_id.clone());
            inner.outcomes.insert(request_id.clone(), false);
            inner.counters.rejected += 1;
            inner.counters.disconnects += 1;
            outcomes.push(ControlTimeoutOutcome::TimedOut { request_id });
        }
        outcomes
    }

    /// Shutdown: resolve every pending false and clear tombstones.
    pub fn shutdown(&self) -> Vec<ControlTimeoutOutcome> {
        let mut inner = self.lock();
        let ids = inner.pending.keys().cloned().collect::<Vec<_>>();
        let mut outcomes = Vec::new();
        for request_id in ids {
            inner.pending.remove(&request_id);
            inner.outcomes.insert(request_id.clone(), false);
            inner.counters.rejected += 1;
            outcomes.push(ControlTimeoutOutcome::TimedOut { request_id });
        }
        inner.tombstones.clear();
        outcomes
    }

    /// Final outcome of one request id (`true` = accepted; corpus/health).
    pub fn outcome_for(&self, request_id: &str) -> Option<bool> {
        self.lock().outcomes.get(request_id).copied()
    }

    /// Exact fence + route authority captured for one pending control
    /// (used by the composition when correlating ACK outcomes back to the
    /// actor lane; C-actor §7 pending record).
    pub fn pending_snapshot(
        &self,
        request_id: &str,
    ) -> Option<(ActorOwnerFence, ActorOwnerRouteAuthority)> {
        self.lock()
            .pending
            .get(request_id)
            .map(|pending| (pending.fence.clone(), pending.route_authority.clone()))
    }

    pub fn health(&self) -> ControlHealth {
        let inner = self.lock();
        ControlHealth {
            pending: inner.pending.len(),
            accepted: inner.counters.accepted,
            rejected: inner.counters.rejected,
            late_acks: inner.counters.late_acks,
            timeouts: inner.counters.timeouts,
            wrong_correlation: inner.counters.wrong_correlation,
            disconnects: inner.counters.disconnects,
            saturated: inner.counters.saturated,
            tombstones: inner.tombstones.len(),
        }
    }
}

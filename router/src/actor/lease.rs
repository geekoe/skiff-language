//! `ActorLeaseExpiryScheduler`: lease/idle deadline scheduling and eviction
//! trigger (plan §3.2, C-actor §3.6/§8, C-model-actor §8).
//!
//! The scheduler owns deadlines and eviction triggers only: lease expiry
//! mutations go through `ActorOwnershipRegistry`, `IdleEvict` frames go
//! through the control seam, and a bounded retry (3) ends in a fail-closed
//! exhausted report without silently keeping the owner.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::health::LeaseHealth;
use super::ownership::ActorOwnershipRegistry;
use super::types::{
    ActorLogicalKey, ActorOwnerFence, DEFAULT_EVICTION_RETRY_BOUND, DEFAULT_IDLE_TTL_MS,
};

/// Scheduler construction options (C-actor §4 defaults).
#[derive(Debug, Clone)]
pub struct LeaseSchedulerOptions {
    pub idle_ttl_ms: u64,
    pub max_eviction_retries: usize,
}

impl Default for LeaseSchedulerOptions {
    fn default() -> Self {
        Self {
            idle_ttl_ms: DEFAULT_IDLE_TTL_MS,
            max_eviction_retries: DEFAULT_EVICTION_RETRY_BOUND,
        }
    }
}

/// Non-blocking `IdleEvict` send port (control seam).
pub trait IdleEvictControlPort: Send + Sync + fmt::Debug {
    fn send_idle_evict(
        &self,
        key: &ActorLogicalKey,
        fence: &ActorOwnerFence,
        eviction_request_id: &str,
        connection: &str,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseError {
    EvictionMismatch,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EvictionMismatch => write!(formatter, "eviction request id mismatch"),
        }
    }
}

impl std::error::Error for LeaseError {}

#[derive(Debug, Clone)]
struct EvictionState {
    eviction_request_id: String,
    retries: usize,
    exhausted: bool,
}

#[derive(Debug, Default)]
struct LeaseCounters {
    sweep_count: u64,
    expired: u64,
    eviction_requests: u64,
    eviction_acked: u64,
    eviction_retries: u64,
    eviction_exhausted: u64,
}

#[derive(Debug, Default)]
struct LeaseInner {
    idle_since: BTreeMap<ActorLogicalKey, u64>,
    connections: BTreeMap<ActorLogicalKey, String>,
    evictions: BTreeMap<ActorLogicalKey, EvictionState>,
    eviction_seq: u64,
    counters: LeaseCounters,
}

/// Lease/idle deadline scheduling owner (C-actor §2).
#[derive(Debug)]
pub struct ActorLeaseExpiryScheduler {
    registry: Arc<ActorOwnershipRegistry>,
    control: Arc<dyn IdleEvictControlPort>,
    options: LeaseSchedulerOptions,
    inner: Arc<Mutex<LeaseInner>>,
}

impl ActorLeaseExpiryScheduler {
    pub fn new(
        registry: Arc<ActorOwnershipRegistry>,
        control: Arc<dyn IdleEvictControlPort>,
        options: LeaseSchedulerOptions,
    ) -> Self {
        Self {
            registry,
            control,
            options,
            inner: Arc::new(Mutex::new(LeaseInner::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LeaseInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Marks an owner live/idle at `now` on the exact connection.
    pub fn mark_live(&self, key: &ActorLogicalKey, now: u64, connection: &str) {
        let mut inner = self.lock();
        inner.idle_since.insert(key.clone(), now);
        inner
            .connections
            .insert(key.clone(), connection.to_string());
    }

    /// Marks the actor active (idle clock reset).
    pub fn mark_active(&self, key: &ActorLogicalKey, now: u64) {
        self.lock().idle_since.insert(key.clone(), now);
    }

    /// One sweep tick: expire leases, mint/retry idle evictions.
    pub fn sweep(&self, now: u64) {
        let mut inner = self.lock();
        inner.counters.sweep_count += 1;

        let owned_keys = self.registry.owned_keys();
        for key in owned_keys {
            let Some(fence) = self.registry.current_owner(&key) else {
                continue;
            };
            let idle_since = inner.idle_since.get(&key).copied().unwrap_or(now);
            if now.saturating_sub(idle_since) < self.options.idle_ttl_ms {
                continue;
            }
            if let Some(mut state) = inner.evictions.get(&key).cloned() {
                if state.exhausted {
                    continue;
                }
                if state.retries >= self.options.max_eviction_retries {
                    state.exhausted = true;
                    inner.evictions.insert(key.clone(), state);
                    inner.counters.eviction_exhausted += 1;
                    continue;
                }
                state.retries += 1;
                inner.counters.eviction_retries += 1;
                let connection = inner.connections.get(&key).cloned().unwrap_or_default();
                let _ = self.control.send_idle_evict(
                    &key,
                    &fence,
                    &state.eviction_request_id,
                    &connection,
                );
                inner.evictions.insert(key.clone(), state);
                continue;
            }
            inner.eviction_seq += 1;
            let seq = inner.eviction_seq;
            let eviction_request_id = format!("evict:{seq}");
            let _ = self.registry.request_eviction(&key, &eviction_request_id);
            let connection = inner.connections.get(&key).cloned().unwrap_or_default();
            let _ = self
                .control
                .send_idle_evict(&key, &fence, &eviction_request_id, &connection);
            inner.evictions.insert(
                key.clone(),
                EvictionState {
                    eviction_request_id,
                    retries: 0,
                    exhausted: false,
                },
            );
            inner.counters.eviction_requests += 1;
        }

        let expired = self.registry.expire(now);
        inner.counters.expired += expired.len() as u64;
        for expired_owner in expired {
            inner.idle_since.remove(&expired_owner.actor_key);
            inner.connections.remove(&expired_owner.actor_key);
            inner.evictions.remove(&expired_owner.actor_key);
        }
    }

    /// Eviction ACK: clears the exact eviction request and (through the
    /// registry) the owner fence.
    pub fn on_eviction_ack(
        &self,
        key: &ActorLogicalKey,
        eviction_request_id: &str,
    ) -> Result<(), LeaseError> {
        let mut inner = self.lock();
        let Some(state) = inner.evictions.get(key) else {
            return Err(LeaseError::EvictionMismatch);
        };
        if state.eviction_request_id != eviction_request_id {
            return Err(LeaseError::EvictionMismatch);
        }
        self.registry
            .acknowledge_eviction(key, eviction_request_id)
            .map_err(|_| LeaseError::EvictionMismatch)?;
        inner.evictions.remove(key);
        inner.idle_since.remove(key);
        inner.connections.remove(key);
        inner.counters.eviction_acked += 1;
        Ok(())
    }

    /// Forgets one actor's idle/eviction bookkeeping after its owner fence was
    /// released outside the sweep path (disconnect/replace/shutdown). The
    /// registry fence removal is owned by the caller; this only drops the
    /// scheduler-local clock so a later re-activation starts a fresh idle
    /// window and no stale eviction is retried against an unowned actor.
    pub fn forget(&self, key: &ActorLogicalKey) {
        let mut inner = self.lock();
        inner.idle_since.remove(key);
        inner.connections.remove(key);
        inner.evictions.remove(key);
    }

    /// Shutdown: cancel every scheduler timer/bookkeeping.
    pub fn shutdown(&self) {
        let mut inner = self.lock();
        inner.idle_since.clear();
        inner.connections.clear();
        inner.evictions.clear();
    }

    pub fn health(&self) -> LeaseHealth {
        let inner = self.lock();
        LeaseHealth {
            sweep_count: inner.counters.sweep_count,
            expired: inner.counters.expired,
            idle_candidates: inner.counters.eviction_requests,
            eviction_pending: inner
                .evictions
                .values()
                .filter(|state| !state.exhausted)
                .count(),
            eviction_acked: inner.counters.eviction_acked,
            eviction_retries: inner.counters.eviction_retries,
            eviction_exhausted: inner.counters.eviction_exhausted,
        }
    }
}

//! Spawn inbound lane support for E-actor-rust (plan §5.5, C-spawn §3-§6,
//! C-dispatch §5).
//!
//! The production `spawn.submit.request` sink (installed into
//! `InboundSinkSet.spawn` by the composition) decodes the canonical wire
//! request and routes it through the two typed parent namespaces:
//! `request` resolves exclusively through the `RequestDispatcher` pending and
//! `actorInvocation` exclusively through the `ActorInvocationRelay` parent
//! store. Function targets are dispatched as dispatcher-owned derived spawns;
//! actor-method targets are admitted through the stateless
//! [`SpawnSubmitRouter`] and handed to the real execution owner together
//! with the raw wire request (M-spawn-repair `SpawnSubmitAcceptance` data
//! surface). The router stores no parent-child mapping: an accepted spawn is
//! separated from the parent lifecycle (C-spawn §2 invariant 4).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use skiff_runtime_transport::protocol::SpawnSubmitRequestFrame;

use super::spawn::{SpawnErrorCode, SpawnSubmitError};

/// One raw wire request awaiting its actor-method execution outcome.
#[derive(Debug, Clone)]
pub struct PendingSpawnWire {
    pub frame: SpawnSubmitRequestFrame,
    pub outcome: Option<Result<(), SpawnSubmitError>>,
}

/// Router-local correlation between the decoded `spawn.submit.request` wire
/// (raw header + opaque args payload) and the accepted actor-method spawn
/// handed to the execution owner. Keyed by the router-minted `spawnId`
/// (the dispatcher's `spawn_request_id`); entries live only for the
/// synchronous accept → execute → respond window and are removed after the
/// response is written (C-spawn §2 invariant 5: no pending residue).
#[derive(Debug, Default)]
pub struct SpawnWireStore {
    entries: Mutex<HashMap<String, PendingSpawnWire>>,
    registered: AtomicU64,
    consumed: AtomicU64,
    orphan_accepts: AtomicU64,
}

impl SpawnWireStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, spawn_request_id: &str, frame: SpawnSubmitRequestFrame) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries
            .insert(
                spawn_request_id.to_string(),
                PendingSpawnWire {
                    frame,
                    outcome: None,
                },
            )
            .is_some()
        {
            // A duplicate registration is a fail-closed signal; the previous
            // entry is replaced and the count reflects the anomaly.
            self.registered.fetch_add(1, Ordering::Relaxed);
        }
        self.registered.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get(&self, spawn_request_id: &str) -> Option<PendingSpawnWire> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(spawn_request_id)
            .cloned()
    }

    /// Records the synchronous execution outcome so the spawn sink can pick
    /// `spawn.submit.response` vs `spawn.submit.error`.
    pub fn set_outcome(&self, spawn_request_id: &str, outcome: Result<(), SpawnSubmitError>) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.get_mut(spawn_request_id) {
            entry.outcome = Some(outcome);
        }
    }

    /// Removes the wire once the response has been written (or the submit was
    /// rejected before execution).
    pub fn remove(&self, spawn_request_id: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.remove(spawn_request_id).is_some() {
            self.consumed.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Fail-closed accounting for an acceptance whose wire is missing.
    pub fn record_orphan_accept(&self) {
        self.orphan_accepts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn pending_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn health(&self) -> SpawnWireHealth {
        SpawnWireHealth {
            pending: self.pending_count(),
            registered: self.registered.load(Ordering::Relaxed),
            consumed: self.consumed.load(Ordering::Relaxed),
            orphan_accepts: self.orphan_accepts.load(Ordering::Relaxed),
        }
    }
}

/// Health projection for the spawn wire correlation (C-spawn §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnWireHealth {
    pub pending: usize,
    pub registered: u64,
    pub consumed: u64,
    pub orphan_accepts: u64,
}

impl SpawnWireHealth {
    pub fn all_zero(&self) -> bool {
        self.pending == 0
    }
}

/// Closed mapping from a wire request to the router-side spawn error code.
/// `SpawnSubmitError` is the typed lane error; callers translate dispatcher
/// rejections separately.
pub fn spawn_error_code(error: &SpawnSubmitError) -> SpawnErrorCode {
    error.code()
}

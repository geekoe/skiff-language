//! Task inbound lane support for E-actor-rust (plan §5.5, C-task §3-§6,
//! C-dispatch §5).
//!
//! The production `task.submit.request` sink (installed into
//! `InboundSinkSet.task` by the composition) decodes the canonical wire
//! request and routes it through the two typed parent namespaces:
//! `request` resolves exclusively through the `RequestDispatcher` pending and
//! `actorInvocation` exclusively through the `ActorInvocationRelay` parent
//! store. Function targets are dispatched as dispatcher-owned derived tasks;
//! actor-method targets are admitted through the stateless
//! [`TaskSubmitRouter`] and handed to the real execution owner together
//! with the raw wire request (M-task-repair `TaskSubmitAcceptance` data
//! surface). The router stores no parent-child mapping: an accepted task is
//! separated from the parent lifecycle (C-task §2 invariant 4).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use skiff_runtime_transport::protocol::TaskSubmitRequestFrame;

use super::task::{TaskErrorCode, TaskSubmitError};

/// One raw wire request awaiting its actor-method execution outcome.
#[derive(Debug, Clone)]
pub struct PendingTaskWire {
    pub frame: TaskSubmitRequestFrame,
    pub outcome: Option<Result<(), TaskSubmitError>>,
}

/// Router-local correlation between the decoded `task.submit.request` wire
/// (raw header + opaque args payload) and the accepted actor-method task
/// handed to the execution owner. Keyed by the router-minted `taskId`
/// (the dispatcher's `task_request_id`); entries live only for the
/// synchronous accept → execute → respond window and are removed after the
/// response is written (C-task §2 invariant 5: no pending residue).
#[derive(Debug, Default)]
pub struct TaskWireStore {
    entries: Mutex<HashMap<String, PendingTaskWire>>,
    registered: AtomicU64,
    consumed: AtomicU64,
    orphan_accepts: AtomicU64,
}

impl TaskWireStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, task_request_id: &str, frame: TaskSubmitRequestFrame) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries
            .insert(
                task_request_id.to_string(),
                PendingTaskWire {
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

    pub fn get(&self, task_request_id: &str) -> Option<PendingTaskWire> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(task_request_id)
            .cloned()
    }

    /// Records the synchronous execution outcome so the task sink can pick
    /// `task.submit.response` vs `task.submit.error`.
    pub fn set_outcome(&self, task_request_id: &str, outcome: Result<(), TaskSubmitError>) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.get_mut(task_request_id) {
            entry.outcome = Some(outcome);
        }
    }

    /// Removes the wire once the response has been written (or the submit was
    /// rejected before execution).
    pub fn remove(&self, task_request_id: &str) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.remove(task_request_id).is_some() {
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

    pub fn health(&self) -> TaskWireHealth {
        TaskWireHealth {
            pending: self.pending_count(),
            registered: self.registered.load(Ordering::Relaxed),
            consumed: self.consumed.load(Ordering::Relaxed),
            orphan_accepts: self.orphan_accepts.load(Ordering::Relaxed),
        }
    }
}

/// Health projection for the task wire correlation (C-task §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskWireHealth {
    pub pending: usize,
    pub registered: u64,
    pub consumed: u64,
    pub orphan_accepts: u64,
}

impl TaskWireHealth {
    pub fn all_zero(&self) -> bool {
        self.pending == 0
    }
}

/// Closed mapping from a wire request to the router-side task error code.
/// `TaskSubmitError` is the typed lane error; callers translate dispatcher
/// rejections separately.
pub fn task_error_code(error: &TaskSubmitError) -> TaskErrorCode {
    error.code()
}

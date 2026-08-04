//! `ActorInvocationRelay`: actor method invocation / return / error / cancel
//! correlation (plan §3.2, C-actor §3.4/§6, C-model-actor §6).
//!
//! The relay never mutates the ownership registry and never handles
//! owner-control ACKs. Every settle is exact-fence: owner return/error must
//! come from the admitted owner fence on the exact connection, caller cancel
//! must carry the exact caller connection and cancellation correlation, and
//! one invocation id settles at most once (bounded tombstone).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::actor_method::{
    ActorMethodCancelReason, ActorMethodDeadlineFrameHeader,
};

use super::health::InvocationHealth;
use super::types::{ActorOwnerFence, ActorOwnerRouteAuthority};

/// Relay construction options.
#[derive(Debug, Clone)]
pub struct ActorInvocationRelayOptions {
    pub max_concurrency: usize,
    pub max_tombstones: usize,
}

impl Default for ActorInvocationRelayOptions {
    fn default() -> Self {
        Self {
            max_concurrency: 64,
            max_tombstones: 1024,
        }
    }
}

/// One admitted actor method invocation (C-actor §3.4).
#[derive(Debug, Clone)]
pub struct ActorInvokeInput {
    pub invocation_id: String,
    pub caller_connection: String,
    pub caller_runtime_id: String,
    pub owner_fence: ActorOwnerFence,
    pub owner_connection: String,
    pub route_authority: ActorOwnerRouteAuthority,
    pub correlation: String,
    pub deadline: Option<ActorMethodDeadlineFrameHeader>,
    pub test_case_capability: Option<String>,
    pub now: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvocationError {
    Duplicate,
    Saturated,
}

impl fmt::Display for InvocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate => write!(formatter, "invocation already pending"),
            Self::Saturated => write!(formatter, "invocation relay saturated"),
        }
    }
}

impl std::error::Error for InvocationError {}

/// Owner settle kind (return / error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerSettleKind {
    Return,
    Error,
}

/// A settled invocation forwarded to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationSettled {
    pub invocation_id: String,
    pub kind: OwnerSettleKind,
}

/// Terminal kinds produced by relay-owned cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationTerminalKind {
    OwnerUnavailable,
    Deadline,
    CallerDisconnected,
    RouterShutdown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationTerminal {
    pub invocation_id: String,
    pub kind: InvocationTerminalKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerCancel {
    pub invocation_id: String,
    pub correlation: String,
    pub reason: ActorMethodCancelReason,
}

#[derive(Debug, Clone)]
struct PendingInvocation {
    caller_connection: String,
    owner_fence: ActorOwnerFence,
    owner_connection: String,
    correlation: String,
    deadline_at: Option<u64>,
    /// Opaque test case capability of the admitted invocation (F2a). Kept
    /// while the invocation is pending so a `task.submit.request` from the
    /// executing Actor method can derive the same capability for its durable
    /// child on the exact owner connection.
    test_case_capability: Option<String>,
}

#[derive(Debug, Default)]
struct InvocationCounters {
    settled: u64,
    rejected: u64,
    terminals: u64,
    deadline_cancels: u64,
    saturated: u64,
}

#[derive(Debug, Default)]
struct InvocationInner {
    pending: BTreeMap<String, PendingInvocation>,
    tombstones: BTreeSet<String>,
    counters: InvocationCounters,
}

/// Actor method invocation correlation owner (C-actor §2).
#[derive(Debug)]
pub struct ActorInvocationRelay {
    inner: Arc<Mutex<InvocationInner>>,
    options: ActorInvocationRelayOptions,
}

impl ActorInvocationRelay {
    pub fn new(options: ActorInvocationRelayOptions) -> Self {
        Self {
            inner: Arc::new(Mutex::new(InvocationInner::default())),
            options,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, InvocationInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Admits one invocation pending (exact owner fence + caller correlation
    /// captured at admission; C-actor §6).
    pub fn invoke(&self, input: &ActorInvokeInput) -> Result<(), InvocationError> {
        let mut inner = self.lock();
        if inner.pending.contains_key(&input.invocation_id) {
            return Err(InvocationError::Duplicate);
        }
        if inner.pending.len() >= self.options.max_concurrency {
            inner.counters.saturated += 1;
            return Err(InvocationError::Saturated);
        }
        let deadline_at = input
            .deadline
            .as_ref()
            .map(|deadline| input.now.saturating_add(deadline.timeout_ms));
        inner.pending.insert(
            input.invocation_id.clone(),
            PendingInvocation {
                caller_connection: input.caller_connection.clone(),
                owner_fence: input.owner_fence.clone(),
                owner_connection: input.owner_connection.clone(),
                correlation: input.correlation.clone(),
                deadline_at,
                test_case_capability: input.test_case_capability.clone(),
            },
        );
        Ok(())
    }

    /// Opaque test case capability retained by one pending Actor invocation
    /// executing on the exact owner connection (F2a task-submit parent
    /// derivation). Returns `None` for ordinary invocations and when the
    /// invocation is not active on the exact connection.
    pub fn parent_test_capability(
        &self,
        owner_connection: &str,
        invocation_id: &str,
    ) -> Option<String> {
        let inner = self.lock();
        inner.pending.get(invocation_id).and_then(|pending| {
            (pending.owner_connection == owner_connection)
                .then(|| pending.test_case_capability.clone())
                .flatten()
        })
    }

    /// Settles a pending invocation from the exact admitted owner
    /// (return/error); duplicate/wrong-fence settles fail closed.
    pub fn on_owner_settle(
        &self,
        invocation_id: &str,
        fence: &ActorOwnerFence,
        connection: &str,
        kind: OwnerSettleKind,
    ) -> Result<InvocationSettled, String> {
        let mut inner = self.lock();
        let Some(pending) = inner.pending.get(invocation_id) else {
            inner.counters.rejected += 1;
            return Err("already settled (duplicate settle or unknown invocation)".to_string());
        };
        if pending.owner_fence != *fence || pending.owner_connection != connection {
            inner.counters.rejected += 1;
            return Err("settle did not come from the exact admitted owner".to_string());
        }
        inner.pending.remove(invocation_id);
        inner.tombstones.insert(invocation_id.to_string());
        inner.counters.settled += 1;
        Ok(InvocationSettled {
            invocation_id: invocation_id.to_string(),
            kind,
        })
    }

    /// Settles a pending invocation from the exact correlated caller cancel.
    pub fn on_caller_cancel(
        &self,
        caller_connection: &str,
        invocation_id: &str,
        correlation: &str,
    ) -> Result<OwnerCancel, String> {
        let mut inner = self.lock();
        let Some(pending) = inner.pending.get(invocation_id) else {
            inner.counters.rejected += 1;
            return Err("already settled (duplicate settle or unknown invocation)".to_string());
        };
        if pending.caller_connection != caller_connection || pending.correlation != correlation {
            inner.counters.rejected += 1;
            return Err("cancel did not come from the correlated caller".to_string());
        }
        let correlation = pending.correlation.clone();
        inner.pending.remove(invocation_id);
        inner.tombstones.insert(invocation_id.to_string());
        inner.counters.settled += 1;
        Ok(OwnerCancel {
            invocation_id: invocation_id.to_string(),
            correlation,
            reason: ActorMethodCancelReason::Cancelled,
        })
    }

    /// Owner runtime disconnect: all pending of the exact owner fence
    /// connection terminal; caller notified; tombstones installed.
    pub fn on_owner_disconnect(
        &self,
        runtime_id: &str,
        connection: &str,
    ) -> Vec<InvocationTerminal> {
        let mut inner = self.lock();
        let ids = inner
            .pending
            .iter()
            .filter(|(_, pending)| {
                pending.owner_fence.owner_runtime_id == runtime_id
                    && pending.owner_connection == connection
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        self.terminal_locked(&mut inner, &ids, InvocationTerminalKind::OwnerUnavailable)
    }

    /// Caller disconnect: cancel the exact owner and terminal the pending.
    pub fn on_caller_disconnect(
        &self,
        caller_connection: &str,
    ) -> (Vec<OwnerCancel>, Vec<InvocationTerminal>) {
        let mut inner = self.lock();
        let ids = inner
            .pending
            .iter()
            .filter(|(_, pending)| pending.caller_connection == caller_connection)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        let cancels = ids
            .iter()
            .filter_map(|id| {
                let pending = inner.pending.get(id)?;
                Some(OwnerCancel {
                    invocation_id: id.clone(),
                    correlation: pending.correlation.clone(),
                    reason: ActorMethodCancelReason::Cancelled,
                })
            })
            .collect();
        let terminals =
            self.terminal_locked(&mut inner, &ids, InvocationTerminalKind::CallerDisconnected);
        (cancels, terminals)
    }

    /// One invocation deadline expiry: cancel owner + terminal caller.
    pub fn on_deadline(&self, invocation_id: &str) -> Option<(OwnerCancel, InvocationTerminal)> {
        let mut inner = self.lock();
        let pending = inner.pending.remove(invocation_id)?;
        inner.tombstones.insert(invocation_id.to_string());
        inner.counters.terminals += 1;
        inner.counters.deadline_cancels += 1;
        Some((
            OwnerCancel {
                invocation_id: invocation_id.to_string(),
                correlation: pending.correlation.clone(),
                reason: ActorMethodCancelReason::DeadlineExceeded,
            },
            InvocationTerminal {
                invocation_id: invocation_id.to_string(),
                kind: InvocationTerminalKind::Deadline,
            },
        ))
    }

    /// Deadline sweep (timer owner): expires every invocation whose captured
    /// deadline elapsed.
    pub fn expire_deadlines(&self, now: u64) -> Vec<(OwnerCancel, InvocationTerminal)> {
        let ids = {
            let inner = self.lock();
            inner
                .pending
                .iter()
                .filter(|(_, pending)| {
                    pending
                        .deadline_at
                        .is_some_and(|deadline_at| deadline_at <= now)
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        ids.into_iter()
            .filter_map(|id| self.on_deadline(&id))
            .collect()
    }

    /// Shutdown: terminal every pending and clear tombstones.
    pub fn shutdown(&self) -> Vec<InvocationTerminal> {
        let mut inner = self.lock();
        let ids = inner.pending.keys().cloned().collect::<Vec<_>>();
        let terminals =
            self.terminal_locked(&mut inner, &ids, InvocationTerminalKind::RouterShutdown);
        inner.tombstones.clear();
        terminals
    }

    fn terminal_locked(
        &self,
        inner: &mut InvocationInner,
        ids: &[String],
        kind: InvocationTerminalKind,
    ) -> Vec<InvocationTerminal> {
        ids.iter()
            .filter_map(|id| {
                if inner.pending.remove(id).is_some() {
                    inner.tombstones.insert(id.clone());
                    inner.counters.terminals += 1;
                    Some(InvocationTerminal {
                        invocation_id: id.clone(),
                        kind,
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn health(&self) -> InvocationHealth {
        let inner = self.lock();
        InvocationHealth {
            pending: inner.pending.len(),
            capacity: self.options.max_concurrency,
            settled: inner.counters.settled,
            rejected: inner.counters.rejected,
            terminals: inner.counters.terminals,
            deadline_cancels: inner.counters.deadline_cancels,
            saturated: inner.counters.saturated,
            tombstones: inner.tombstones.len(),
        }
    }
}

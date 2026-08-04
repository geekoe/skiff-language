//! F2a Router-side derivation of a task submission's test-case authority.
//!
//! The `task.submit.request` wire deliberately carries only `callerRequestId`
//! (`doc/architecture/test-runner-runtime-isolation.md`: it must not repeat
//! `testCaseCapability` or a test parent id). The Router derives the parent
//! test case capability from the still-active parent request / Actor
//! invocation on the exact same Runtime connection, persists it on the
//! durable `TaskRecord`, and admission re-uses it for the attempt. Ordinary
//! production submissions resolve to `None` and keep the D2
//! location-transparent semantics.

use std::fmt;
use std::sync::Arc;

use skiff_runtime_transport::protocol::TaskCallerKind;

use crate::dispatch::RequestDispatcher;
use crate::session::identity::RuntimeSessionEpoch;
use crate::supervisor::actor::ActorComponents;

/// Port used by the durable task sink to derive test-case authority from the
/// active parent on the exact session.
pub trait TaskSubmitParentResolver: Send + Sync + fmt::Debug {
    /// Resolves the parent's opaque test case capability. `None` means the
    /// parent is ordinary (or no longer active on the exact connection) and
    /// the task is an ordinary production submission with no test authority.
    fn resolve(
        &self,
        session: &RuntimeSessionEpoch,
        caller_kind: TaskCallerKind,
        caller_request_id: &str,
    ) -> Option<String>;
}

/// Production resolver over the request-dispatcher pending (Request parent)
/// and the actor invocation relay (ActorInvocation parent). Both lookups are
/// keyed by the exact Runtime connection so a capability can never be
/// borrowed across sessions.
#[derive(Debug)]
pub struct RouterTaskSubmitParentResolver {
    dispatcher: Arc<RequestDispatcher>,
    actor: Arc<ActorComponents>,
}

impl RouterTaskSubmitParentResolver {
    pub fn new(dispatcher: Arc<RequestDispatcher>, actor: Arc<ActorComponents>) -> Self {
        Self { dispatcher, actor }
    }
}

impl TaskSubmitParentResolver for RouterTaskSubmitParentResolver {
    fn resolve(
        &self,
        session: &RuntimeSessionEpoch,
        caller_kind: TaskCallerKind,
        caller_request_id: &str,
    ) -> Option<String> {
        match caller_kind {
            TaskCallerKind::Request => self
                .dispatcher
                .parent_test_capability(session, caller_request_id),
            TaskCallerKind::ActorInvocation => {
                let connection = format!(
                    "{}#{}",
                    session.replica_id, session.connection_generation
                );
                self.actor
                    .relay
                    .parent_test_capability(&connection, caller_request_id)
            }
        }
    }
}

/// Deterministic no-op resolver for tests and non-production compositions.
/// Every task is treated as an ordinary submission (`None`).
#[derive(Debug, Default)]
pub struct NoopTaskSubmitParentResolver;

impl TaskSubmitParentResolver for NoopTaskSubmitParentResolver {
    fn resolve(
        &self,
        _session: &RuntimeSessionEpoch,
        _caller_kind: TaskCallerKind,
        _caller_request_id: &str,
    ) -> Option<String> {
        None
    }
}

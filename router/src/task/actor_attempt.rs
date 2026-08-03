//! Actor-method task attempt terminal vocabulary (E2b execution side).
//!
//! Actor-method attempts are ordinary Actor invocations admitted through the
//! actor lane; their terminals arrive as `actor.method.return` /
//! `actor.method.error` / `actor.owner.failure` / relay deadline / owner
//! disconnect. The task control plane maps these to TaskStore settlement
//! (succeeded / failed / platform-failed) or to lease-loss recovery /
//! provable-rejection backoff.

use std::fmt;

/// Definite terminal of one actor-method task attempt, as proven by the
/// actor lane correlation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorAttemptTerminal {
    /// `actor.method.return` converged: the target returned normally.
    Succeeded,
    /// The target explicitly threw / rejected, or the owner runtime
    /// definitely refused the attempt (owner failure / invocation deadline).
    TargetFailed { message: String },
    /// The task implementation was taken over by a different implementation
    /// (ActorVersionRejectedError / ActorIncarnationReplacedError):
    /// platform-failed terminal, never retried and never handed to new code.
    VersionRejected { message: String },
    /// The actor is upgrading (ActorUpgradingError): recoverable platform
    /// condition. The attempt is released with the runtime-provided backoff
    /// and a later attempt re-runs get-or-activate.
    Upgrading { retry_after_ms: u64 },
    /// Disconnect / shutdown / protocol loss: the outcome cannot be proven.
    /// No settlement; lease expiry drives recovery.
    Uncertain { reason: String },
}

/// Terminal sink consumed by the actor frame sink for task-attempt
/// invocations (implementation: [`super::control::DurableTaskControl`]).
pub trait ActorAttemptTerminalSink: Send + Sync + fmt::Debug {
    fn on_actor_terminal(
        &self,
        request_id: &str,
        task_id: &str,
        attempt_id: &str,
        lease_id: &str,
        terminal: ActorAttemptTerminal,
    );
}

/// Default no-op sink used by tests that do not exercise the task control
/// plane's actor terminal mapping.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoopActorAttemptTerminalSink;

impl ActorAttemptTerminalSink for NoopActorAttemptTerminalSink {
    fn on_actor_terminal(
        &self,
        _request_id: &str,
        _task_id: &str,
        _attempt_id: &str,
        _lease_id: &str,
        _terminal: ActorAttemptTerminal,
    ) {
    }
}

/// Correlation facts for one task-attempt actor invocation registered in the
/// actor frame sink. The caller side is the task control plane, not a Runtime
/// session; terminals route back through [`ActorAttemptTerminalSink`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskAttemptInvocationCorrelation {
    pub request_id: String,
    pub task_id: String,
    pub attempt_id: String,
    pub lease_id: String,
}

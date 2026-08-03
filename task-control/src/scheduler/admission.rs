//! Pluggable Runtime admission seam for the task scheduler.
//!
//! The scheduler never interprets the business payload. It hands one leased
//! attempt to this seam, and the seam returns what the platform can prove
//! about the Runtime admission. The stage-C2 fake lives in the integration
//! tests; the real Router transport admission is a later node (D2).

use async_trait::async_trait;

use crate::model::TaskRecord;

/// Admission decision for one leased attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// Runtime accepted the request; the attempt is executing. The scheduler
    /// renews this lease while it is pending; the seam settles through the
    /// TaskStore with the current lease id when the attempt ends.
    Accepted,
    /// The platform proved the Runtime did not accept the request. The
    /// scheduler releases the claim back to `ready` with platform backoff so
    /// the task can be claimed again (transient, non-terminal).
    RejectedProvable { reason: String },
    /// The admission outcome cannot be proven either way. The scheduler makes
    /// no settlement and does not release; lease expiry then drives store
    /// recovery, which produces a new attempt with platform backoff.
    Uncertain { reason: String },
    /// The platform proved the task can never form a legal execution attempt
    /// (permanent error). The scheduler settles `platform-failed` with the
    /// current lease id and does not retry.
    PermanentFailure { reason: String },
}

/// Admittance port consumed by [`crate::scheduler::Scheduler`]. The seam owns
/// Runtime transport semantics; the scheduler owns claim / lease / backoff.
#[async_trait]
pub trait AttemptAdmission: Send + Sync {
    /// Admit one freshly claimed attempt. The record is the post-claim store
    /// authority fact: `state == leased`, `active_lease` carries the fresh
    /// AttemptId / lease id and the frozen execution image witness.
    async fn admit(&self, record: &TaskRecord) -> AdmissionDecision;
}

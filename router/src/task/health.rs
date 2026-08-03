//! Task control plane health projection (authoritative design "Observability
//! And Retention"). Health never exposes TaskIds, payload bytes, Mongo URLs
//! or secrets; it only carries occupancy/counter projections.

/// Read-only snapshot of the durable task control plane.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TaskControlHealth {
    /// Leases this router replica accepted and is renewing.
    pub renewing_attempts: usize,
    /// Store-authority backlog (observe_backlog).
    pub backlog_scheduled: usize,
    pub backlog_ready: usize,
    pub backlog_leased: usize,
    /// Oldest `due_at` across scheduled + ready records (epoch millis).
    pub oldest_due_at_ms: Option<i64>,
    /// Submission outcomes.
    pub submissions_accepted: u64,
    pub submissions_rejected: u64,
    pub submissions_transient: u64,
    /// Status queries and their expired/unavailable projections.
    pub status_queries: u64,
    pub status_expired: u64,
    pub status_not_found: u64,
    pub status_unavailable: u64,
    /// Cancel outcomes by reference kind.
    pub cancel_canceled: u64,
    pub cancel_already_started: u64,
    pub cancel_already_terminal: u64,
    pub cancel_expired: u64,
    pub cancel_not_found: u64,
    pub cancel_unavailable: u64,
    /// Attempt settlements by outcome class.
    pub settlements_succeeded: u64,
    pub settlements_failed: u64,
    pub settlements_uncertain: u64,
    /// Platform-proven terminal (ActorVersionRejectedError etc.).
    pub settlements_platform_failed: u64,
    /// Recoverable actor upgrading releases (backoff, not a settlement).
    pub settlements_upgrading: u64,
    /// Admissions observed by this replica's scheduler seam.
    pub admissions_accepted: u64,
    pub admissions_rejected: u64,
    pub admissions_uncertain: u64,
    pub admissions_permanent_failure: u64,
}

/// Atomic counter bank shared by the task sink, control plane and admission
/// seam. Each field maps one-to-one onto [`TaskControlHealth`].
#[derive(Debug, Default)]
pub struct TaskControlCounters {
    pub submissions_accepted: std::sync::atomic::AtomicU64,
    pub submissions_rejected: std::sync::atomic::AtomicU64,
    pub submissions_transient: std::sync::atomic::AtomicU64,
    pub status_queries: std::sync::atomic::AtomicU64,
    pub status_expired: std::sync::atomic::AtomicU64,
    pub status_not_found: std::sync::atomic::AtomicU64,
    pub status_unavailable: std::sync::atomic::AtomicU64,
    pub cancel_canceled: std::sync::atomic::AtomicU64,
    pub cancel_already_started: std::sync::atomic::AtomicU64,
    pub cancel_already_terminal: std::sync::atomic::AtomicU64,
    pub cancel_expired: std::sync::atomic::AtomicU64,
    pub cancel_not_found: std::sync::atomic::AtomicU64,
    pub cancel_unavailable: std::sync::atomic::AtomicU64,
    pub settlements_succeeded: std::sync::atomic::AtomicU64,
    pub settlements_failed: std::sync::atomic::AtomicU64,
    pub settlements_uncertain: std::sync::atomic::AtomicU64,
    pub settlements_platform_failed: std::sync::atomic::AtomicU64,
    pub settlements_upgrading: std::sync::atomic::AtomicU64,
    pub admissions_accepted: std::sync::atomic::AtomicU64,
    pub admissions_rejected: std::sync::atomic::AtomicU64,
    pub admissions_uncertain: std::sync::atomic::AtomicU64,
    pub admissions_permanent_failure: std::sync::atomic::AtomicU64,
}

impl TaskControlCounters {
    pub fn snapshot(&self) -> TaskControlHealth {
        TaskControlHealth {
            renewing_attempts: 0,
            backlog_scheduled: 0,
            backlog_ready: 0,
            backlog_leased: 0,
            oldest_due_at_ms: None,
            submissions_accepted: self.submissions_accepted.load(std::sync::atomic::Ordering::Relaxed),
            submissions_rejected: self.submissions_rejected.load(std::sync::atomic::Ordering::Relaxed),
            submissions_transient: self
                .submissions_transient
                .load(std::sync::atomic::Ordering::Relaxed),
            status_queries: self.status_queries.load(std::sync::atomic::Ordering::Relaxed),
            status_expired: self.status_expired.load(std::sync::atomic::Ordering::Relaxed),
            status_not_found: self
                .status_not_found
                .load(std::sync::atomic::Ordering::Relaxed),
            status_unavailable: self
                .status_unavailable
                .load(std::sync::atomic::Ordering::Relaxed),
            cancel_canceled: self.cancel_canceled.load(std::sync::atomic::Ordering::Relaxed),
            cancel_already_started: self
                .cancel_already_started
                .load(std::sync::atomic::Ordering::Relaxed),
            cancel_already_terminal: self
                .cancel_already_terminal
                .load(std::sync::atomic::Ordering::Relaxed),
            cancel_expired: self.cancel_expired.load(std::sync::atomic::Ordering::Relaxed),
            cancel_not_found: self
                .cancel_not_found
                .load(std::sync::atomic::Ordering::Relaxed),
            cancel_unavailable: self
                .cancel_unavailable
                .load(std::sync::atomic::Ordering::Relaxed),
            settlements_succeeded: self
                .settlements_succeeded
                .load(std::sync::atomic::Ordering::Relaxed),
            settlements_failed: self
                .settlements_failed
                .load(std::sync::atomic::Ordering::Relaxed),
            settlements_uncertain: self
                .settlements_uncertain
                .load(std::sync::atomic::Ordering::Relaxed),
            settlements_platform_failed: self
                .settlements_platform_failed
                .load(std::sync::atomic::Ordering::Relaxed),
            settlements_upgrading: self
                .settlements_upgrading
                .load(std::sync::atomic::Ordering::Relaxed),
            admissions_accepted: self
                .admissions_accepted
                .load(std::sync::atomic::Ordering::Relaxed),
            admissions_rejected: self
                .admissions_rejected
                .load(std::sync::atomic::Ordering::Relaxed),
            admissions_uncertain: self
                .admissions_uncertain
                .load(std::sync::atomic::Ordering::Relaxed),
            admissions_permanent_failure: self
                .admissions_permanent_failure
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }
}

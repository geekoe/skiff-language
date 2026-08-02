//! Activation state repository health snapshot (C-router-activation-state §9).
//!
//! The snapshot is read-only to consumers; the repository owns the mutable
//! state and publishes a clone through its port. Health never contains Mongo
//! URLs, secrets, or business payloads.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryMutationOutcome {
    Ok,
    CasMismatch,
    InvalidRecord,
    Transient,
}

impl RepositoryMutationOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::CasMismatch => "cas_mismatch",
            Self::InvalidRecord => "invalid",
            Self::Transient => "transient",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RetryHealth {
    pub attempts: u32,
    pub retried: u32,
    pub next_backoff_ms: u64,
    pub deadline_remaining_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuditHealth {
    pub last_event_id: Option<String>,
    pub last_event_operation: Option<String>,
    pub last_event_timestamp: Option<i64>,
    pub failed_writes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriverHealth {
    pub connected: bool,
    pub reconnecting: bool,
    pub closed: bool,
    pub shutdown_residue: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivationRepositoryHealth {
    pub environment: Option<String>,
    pub committed_generation: Option<u64>,
    pub pending_activation_id: Option<String>,
    pub last_outcome: Option<RepositoryMutationOutcome>,
    pub last_outcome_operation: Option<String>,
    pub retry: RetryHealth,
    pub audit: AuditHealth,
    pub driver: DriverHealth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_health_is_empty_and_deterministic() {
        let health = ActivationRepositoryHealth::default();
        assert_eq!(health.environment, None);
        assert_eq!(health.committed_generation, None);
        assert_eq!(health.pending_activation_id, None);
        assert_eq!(health.last_outcome, None);
        assert_eq!(health.retry.attempts, 0);
        assert_eq!(health.audit.failed_writes, 0);
        assert!(!health.driver.connected);
        assert!(!health.driver.closed);
    }

    #[test]
    fn outcome_names_match_frozen_audit_vocabulary() {
        assert_eq!(RepositoryMutationOutcome::Ok.as_str(), "ok");
        assert_eq!(
            RepositoryMutationOutcome::CasMismatch.as_str(),
            "cas_mismatch"
        );
        assert_eq!(RepositoryMutationOutcome::InvalidRecord.as_str(), "invalid");
        assert_eq!(RepositoryMutationOutcome::Transient.as_str(), "transient");
    }
}

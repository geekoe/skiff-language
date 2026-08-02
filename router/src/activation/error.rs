//! Router-owned activation state repository error classification.
//!
//! Classification follows C-router-activation-state §5: `CasMismatch` and
//! `InvalidRecord` are deterministic outcomes that must not be retried;
//! `Transient` covers driver/connection/backoff/write-conflict infrastructure
//! failures that the bounded retry policy may retry; `Closed` is the terminal
//! state after `close()`.

use thiserror::Error;

use skiff_deployment::activation_state::ActivationStateError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RepositoryError {
    #[error("activation state CAS mismatch for environment {environment}: {message}")]
    CasMismatch {
        environment: String,
        message: String,
    },
    #[error("invalid activation state for environment {environment}: {message}")]
    InvalidRecord {
        environment: String,
        message: String,
    },
    #[error("transient activation state repository failure: {message}")]
    Transient { message: String },
    #[error("activation state repository is closed")]
    Closed,
}

impl RepositoryError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Transient { .. })
    }

    pub fn class(&self) -> RepositoryErrorClass {
        match self {
            Self::CasMismatch { .. } => RepositoryErrorClass::CasMismatch,
            Self::InvalidRecord { .. } => RepositoryErrorClass::InvalidRecord,
            Self::Transient { .. } => RepositoryErrorClass::Transient,
            Self::Closed => RepositoryErrorClass::Closed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepositoryErrorClass {
    CasMismatch,
    InvalidRecord,
    Transient,
    Closed,
}

pub(crate) fn map_reducer_error(error: ActivationStateError) -> RepositoryError {
    match error {
        ActivationStateError::CasMismatch {
            environment,
            message,
        } => RepositoryError::CasMismatch {
            environment,
            message,
        },
        ActivationStateError::InvalidRecord {
            environment,
            message,
        } => RepositoryError::InvalidRecord {
            environment,
            message,
        },
    }
}

pub(crate) fn cas_mismatch(environment: &str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::CasMismatch {
        environment: environment.to_string(),
        message: message.into(),
    }
}

pub(crate) fn invalid_record(environment: &str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::InvalidRecord {
        environment: environment.to_string(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_transient_errors_are_retryable() {
        let transient = RepositoryError::Transient {
            message: "connection reset".to_string(),
        };
        let cas = cas_mismatch("test", "stale generation");
        let invalid = invalid_record("test", "bad schema");
        assert!(transient.is_retryable());
        assert!(!cas.is_retryable());
        assert!(!invalid.is_retryable());
        assert!(!RepositoryError::Closed.is_retryable());
        assert_eq!(transient.class(), RepositoryErrorClass::Transient);
        assert_eq!(cas.class(), RepositoryErrorClass::CasMismatch);
        assert_eq!(invalid.class(), RepositoryErrorClass::InvalidRecord);
    }

    #[test]
    fn reducer_errors_map_without_class_change() {
        let cas = map_reducer_error(ActivationStateError::CasMismatch {
            environment: "test".to_string(),
            message: "conflict".to_string(),
        });
        assert!(matches!(
            cas,
            RepositoryError::CasMismatch {
                environment,
                ..
            } if environment == "test"
        ));
        let invalid = map_reducer_error(ActivationStateError::InvalidRecord {
            environment: "test".to_string(),
            message: "corrupt".to_string(),
        });
        assert!(matches!(
            invalid,
            RepositoryError::InvalidRecord { environment, .. } if environment == "test"
        ));
    }
}

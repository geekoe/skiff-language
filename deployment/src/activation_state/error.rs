//! Adapter-independent error types for durable activation state reduction.
//!
//! These map one-to-one onto the CAS/validation classification frozen by
//! C-router-activation-state (§5): `CasMismatch` is a recoverable concurrent
//! conflict that must not be retried blindly, `InvalidRecord` is
//! non-recoverable input/persistence corruption. The Router-owned Mongo
//! adapter adds its own transient/shutdown classification on top.

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ActivationStateError {
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
}

pub type ActivationStateResult<T> = Result<T, ActivationStateError>;

pub(crate) fn cas_error(environment: &str, message: impl Into<String>) -> ActivationStateError {
    ActivationStateError::CasMismatch {
        environment: environment.to_string(),
        message: message.into(),
    }
}

pub(crate) fn invalid_error(environment: &str, message: impl Into<String>) -> ActivationStateError {
    ActivationStateError::InvalidRecord {
        environment: environment.to_string(),
        message: message.into(),
    }
}

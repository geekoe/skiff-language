use std::{fmt, sync::Arc};

use skiff_artifact_model::DeploymentArtifactIdentity;

use crate::DeploymentOwnerIdentity;

/// Opaque identity for one deployment-image load attempt.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LoadAttemptId(u64);

impl LoadAttemptId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Why one shared load attempt failed.
#[derive(Debug)]
pub enum DeploymentLoadFailureReason<E> {
    Provider {
        error: Arc<E>,
    },
    OutputOwnerMismatch {
        expected: DeploymentOwnerIdentity,
        actual: DeploymentOwnerIdentity,
    },
    LoaderTaskPanicked,
    LoaderTaskCancelled,
    RuntimeUnavailable,
    AttemptStateUnavailable,
}

/// Failure value shared by every waiter that joined one load attempt.
#[derive(Debug)]
pub struct DeploymentLoadFailure<E> {
    attempt_id: LoadAttemptId,
    reason: DeploymentLoadFailureReason<E>,
}

impl<E> DeploymentLoadFailure<E> {
    pub fn attempt_id(&self) -> LoadAttemptId {
        self.attempt_id
    }

    pub fn reason(&self) -> &DeploymentLoadFailureReason<E> {
        &self.reason
    }

    pub(crate) fn provider(attempt_id: LoadAttemptId, error: E) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::Provider {
                error: Arc::new(error),
            },
        })
    }

    pub(crate) fn output_owner_mismatch(
        attempt_id: LoadAttemptId,
        expected: DeploymentOwnerIdentity,
        actual: DeploymentOwnerIdentity,
    ) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::OutputOwnerMismatch { expected, actual },
        })
    }

    pub(crate) fn loader_task_panicked(attempt_id: LoadAttemptId) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::LoaderTaskPanicked,
        })
    }

    pub(crate) fn loader_task_cancelled(attempt_id: LoadAttemptId) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::LoaderTaskCancelled,
        })
    }

    pub(crate) fn runtime_unavailable(attempt_id: LoadAttemptId) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::RuntimeUnavailable,
        })
    }

    pub(crate) fn attempt_state_unavailable(attempt_id: LoadAttemptId) -> Arc<Self> {
        Arc::new(Self {
            attempt_id,
            reason: DeploymentLoadFailureReason::AttemptStateUnavailable,
        })
    }
}

impl<E: fmt::Display> fmt::Display for DeploymentLoadFailure<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "deployment load attempt {} failed: ",
            self.attempt_id.get()
        )?;
        match &self.reason {
            DeploymentLoadFailureReason::Provider { error } => fmt::Display::fmt(error, formatter),
            DeploymentLoadFailureReason::OutputOwnerMismatch { expected, actual } => write!(
                formatter,
                "loader returned owner {actual:?}, expected {expected:?}"
            ),
            DeploymentLoadFailureReason::LoaderTaskPanicked => {
                formatter.write_str("loader task panicked")
            }
            DeploymentLoadFailureReason::LoaderTaskCancelled => {
                formatter.write_str("loader task was cancelled")
            }
            DeploymentLoadFailureReason::RuntimeUnavailable => {
                formatter.write_str("deployment loading requires a Tokio runtime")
            }
            DeploymentLoadFailureReason::AttemptStateUnavailable => {
                formatter.write_str("attempt state became unavailable")
            }
        }
    }
}

impl<E> std::error::Error for DeploymentLoadFailure<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.reason {
            DeploymentLoadFailureReason::Provider { error } => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// A build id was presented with a different full deployment identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentOwnerConflict {
    build_id: DeploymentArtifactIdentity,
    existing: Box<DeploymentOwnerIdentity>,
    requested: Box<DeploymentOwnerIdentity>,
}

impl DeploymentOwnerConflict {
    pub fn build_id(&self) -> &DeploymentArtifactIdentity {
        &self.build_id
    }

    pub fn existing(&self) -> &DeploymentOwnerIdentity {
        self.existing.as_ref()
    }

    pub fn requested(&self) -> &DeploymentOwnerIdentity {
        self.requested.as_ref()
    }

    pub(crate) fn new(
        build_id: DeploymentArtifactIdentity,
        existing: DeploymentOwnerIdentity,
        requested: DeploymentOwnerIdentity,
    ) -> Self {
        Self {
            build_id,
            existing: Box::new(existing),
            requested: Box::new(requested),
        }
    }
}

impl fmt::Display for DeploymentOwnerConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "deployment build {} is bound to {:?}, not {:?}",
            self.build_id, self.existing, self.requested
        )
    }
}

impl std::error::Error for DeploymentOwnerConflict {}

/// Failure returned before or after joining a deployment load attempt.
#[derive(Debug)]
pub enum DeploymentLoadError<E> {
    OwnerConflict(DeploymentOwnerConflict),
    Attempt(Arc<DeploymentLoadFailure<E>>),
    AttemptIdExhausted,
}

impl<E> Clone for DeploymentLoadError<E> {
    fn clone(&self) -> Self {
        match self {
            Self::OwnerConflict(conflict) => Self::OwnerConflict(conflict.clone()),
            Self::Attempt(failure) => Self::Attempt(Arc::clone(failure)),
            Self::AttemptIdExhausted => Self::AttemptIdExhausted,
        }
    }
}

impl<E: fmt::Display> fmt::Display for DeploymentLoadError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerConflict(conflict) => fmt::Display::fmt(conflict, formatter),
            Self::Attempt(failure) => fmt::Display::fmt(failure, formatter),
            Self::AttemptIdExhausted => {
                formatter.write_str("deployment load attempt ids exhausted")
            }
        }
    }
}

impl<E> std::error::Error for DeploymentLoadError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OwnerConflict(conflict) => Some(conflict),
            Self::Attempt(failure) => Some(failure.as_ref()),
            Self::AttemptIdExhausted => None,
        }
    }
}

pub type DeploymentLoadResult<P, E> = Result<Arc<P>, DeploymentLoadError<E>>;

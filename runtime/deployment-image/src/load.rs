use std::sync::Arc;

use crate::DeploymentImage;

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

/// Failure value shared by every waiter that observed one load attempt.
#[derive(Debug)]
pub struct DeploymentLoadFailure<E> {
    attempt_id: LoadAttemptId,
    error: Arc<E>,
}

impl<E> DeploymentLoadFailure<E> {
    pub fn new(attempt_id: LoadAttemptId, error: Arc<E>) -> Self {
        Self { attempt_id, error }
    }

    pub fn attempt_id(&self) -> LoadAttemptId {
        self.attempt_id
    }

    pub fn error(&self) -> &Arc<E> {
        &self.error
    }
}

pub type DeploymentLoadResult<P, E> =
    Result<Arc<DeploymentImage<P>>, Arc<DeploymentLoadFailure<E>>>;

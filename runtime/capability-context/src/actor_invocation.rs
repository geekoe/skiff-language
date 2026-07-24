use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};

/// Identity facts which must survive the Router/Runtime boundary unchanged.
/// Payload bytes deliberately remain outside this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInvocationIdentity {
    pub invocation_id: String,
    pub expected_epoch: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub requested_implementation_identity: ActorImplementationIdentity,
    pub method_identity: ActorMethodIdentity,
    pub cancellation_correlation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInvocationDeadline {
    pub timeout_ms: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorInvocationCancellation {
    Cancelled,
    DeadlineExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorInvocationError {
    ActorUpgrading {
        retry_after_ms: u64,
    },
    ActorVersionRejected {
        requested: ActorImplementationIdentity,
        accepted: ActorImplementationIdentity,
    },
    ActorIncarnationReplaced {
        requested_epoch: u64,
        current_epoch: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorInvocationOutcome {
    Returned(Vec<u8>),
    ActorError(ActorInvocationError),
    Cancelled(ActorInvocationCancellation),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_errors_are_not_generic_transport_errors() {
        let outcome =
            ActorInvocationOutcome::ActorError(ActorInvocationError::ActorIncarnationReplaced {
                requested_epoch: 1,
                current_epoch: 2,
            });
        assert!(matches!(
            outcome,
            ActorInvocationOutcome::ActorError(
                ActorInvocationError::ActorIncarnationReplaced { .. }
            )
        ));
    }
}

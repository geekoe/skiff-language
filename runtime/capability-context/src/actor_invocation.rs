use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_runtime_model::runtime_value::ActorRef;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorInvocationOwnerUnit {
    Service,
    Package(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorInvocationOwnerFile {
    LoadedFileIndex(u64),
    FileIrIdentity(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInvocationDeclarationOwner {
    pub unit: ActorInvocationOwnerUnit,
    pub file: ActorInvocationOwnerFile,
    pub actor_symbol: String,
}

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInvocationRequest {
    pub actor_ref: ActorRef,
    pub declaration_owner: ActorInvocationDeclarationOwner,
    pub identity: ActorInvocationIdentity,
    pub deadline: ActorInvocationDeadline,
    pub arguments_payload: Vec<u8>,
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
mod tests;

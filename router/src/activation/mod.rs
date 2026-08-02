//! Router-owned durable activation state (authoritative design §2.2 third
//! model; C-router-activation-state W-lane).
//!
//! The Mongo adapter, retry policy, health snapshot, and index contract live
//! here. Runtime/transport never consume this module; the coordinator and
//! bootstrap reader consume the repository port from the Router process.
//! W-activation adds the `ActivationCoordinator` (live transaction + cold
//! recovery) on top of the frozen repository.

pub mod coordinator;
pub mod error;
pub mod health;
pub mod http;
pub mod index;
pub mod memory;
pub mod recovery;
pub mod repository;
pub mod retry;

pub use coordinator::{
    ActivationCandidateError, ActivationCoordinator, ActivationCoordinatorHandle,
    ActivationCoordinatorHealth, ActivationCoordinatorOptions, ActivationCoordinatorPorts,
    ActivationParticipantBinding, ActivationPhase, ActivationRevalidateOutcome,
    BlockingLoaderCandidatePort, BlockingLoaderPort, CandidateLoadError, CoordinatorError,
    DecisionState, EnqueueResult, EpochStorePublishPort, HealthSinkPort, NoopHealthSink,
    PublishCommittedEpochPort, RoutingCandidateQueryPortAdapter, RuntimeCandidateQueryPort,
    SessionEnqueuePort,
};
pub use error::{RepositoryError, RepositoryErrorClass};
pub use health::ActivationRepositoryHealth;
pub use http::{
    ActivationHttpHandler, ACTIVATION_REQUEST_BODY_CAP, ASSEMBLY_ACTIVATION_CONTROL_PATH,
};
pub use recovery::{CandidateEpochRefs, RecoveryTransaction};
pub use repository::{
    AbortInput, ActivationStateRepository, CommitInput, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, PrepareInput,
};
pub use retry::{ActivationClock, RetryOutcome, RetryPolicy, SystemClock};

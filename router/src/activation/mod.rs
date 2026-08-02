//! Router-owned durable activation state (authoritative design §2.2 third
//! model; C-router-activation-state W-lane).
//!
//! The Mongo adapter, retry policy, health snapshot, and index contract live
//! here. Runtime/transport never consume this module; the coordinator and
//! bootstrap reader consume the repository port from the Router process.

pub mod error;
pub mod health;
pub mod index;
pub mod memory;
pub mod repository;
pub mod retry;

pub use error::{RepositoryError, RepositoryErrorClass};
pub use health::ActivationRepositoryHealth;
pub use repository::{
    AbortInput, ActivationStateRepository, CommitInput, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, PrepareInput,
};
pub use retry::{ActivationClock, RetryOutcome, RetryPolicy, SystemClock};

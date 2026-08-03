//! Durable task dispatch core: canonical task model and the TaskStore
//! authority contract (shared checkpoint C1).
//!
//! This crate owns task identity, state, `due_at`, attempt generation, lease,
//! cancellation results and terminal outcomes. The [`TaskStore`] port is
//! storage-agnostic; the in-memory fake shares the pure state-machine reducer
//! with the Mongo adapter, and the Mongo adapter executes every transition as
//! a conditional write / CAS on the server clock (`$$NOW`), never on a client
//! wall clock.
//!
//! The crate deliberately has no dependency on runtime execution or router
//! business code: it is consumed by the router and hosts the scheduler core
//! (stage C2) behind a pluggable admission seam.

pub mod clock;
pub mod error;
pub mod memory;
pub mod model;
pub mod mongo;
pub mod reducer;
pub mod retry;
pub mod scheduler;
pub mod store;

pub use clock::{SystemClock, TaskClock};
pub use error::{TaskStoreError, TaskStoreErrorClass};
pub use memory::MemoryTaskStore;
pub use mongo::{
    task_state_due_at_index, MongoTaskStore, MongoTaskStoreOptions, DEFAULT_TASK_COLLECTION,
    DEFAULT_TASK_DATABASE, TASK_STATE_DUE_AT_INDEX,
};
pub use retry::{TaskRetryOutcome, TaskRetryPolicy};
pub use store::{
    CancelInput, ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput, LeaseRecoveryInput,
    LeaseRecoveryOutcome, ReleaseInput, ReleaseOutcome, RenewInput, RenewOutcome, RenewRejection,
    ScanExpiredLeasesInput, SettleInput, SettleOutcome, SettleTransition, StatusInput, TaskStore,
};

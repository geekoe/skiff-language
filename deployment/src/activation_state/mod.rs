//! Durable activation state model: adapter-independent reducer, audit event,
//! and error classification (Router/platform durable activation model,
//! authoritative design §2.2 third model).
//!
//! The DTO itself stays in the frozen `storage::activation` module
//! (C-router-activation-state §2); this module adds the pure transition
//! functions shared by every persistence adapter and the audit event shape
//! frozen by C-router-activation-state §6. Runtime/transport never consume
//! this module.

pub mod audit;
pub mod error;
pub mod reducer;

pub use crate::storage::{
    ActivationRecoveryAction, CommittedActivation, PendingActivation, ProfileActivationState,
    PROFILE_ACTIVATION_STATE_SCHEMA_VERSION,
};
pub use audit::{
    activation_audit_event_id, ActivationAuditEvent, ActivationAuditOperation,
    ActivationAuditOutcome,
};
pub use error::{ActivationStateError, ActivationStateResult};
pub use reducer::{abort, commit, prepare, AbortInput, CommitInput, PrepareInput};

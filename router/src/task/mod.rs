//! Durable task dispatch control plane (stage D2): the router-side owner of
//! TaskStore composition, submission/status/cancel wire handlers, the real
//! AttemptAdmission seam and attempt settlement/deadline tracking.
//!
//! Owner boundary (authoritative design "Layer Ownership"): TaskStore owns
//! durable facts, the scheduler owns claim/lease/fairness, this module owns
//! Runtime transport admission and settlement correlation. It is isolated
//! from activation/session/actor owners; it consumes them only through the
//! narrow ports defined here and in the dispatcher.

pub(crate) mod activation;
pub mod actor_attempt;
pub mod actor_plan;
pub mod actor_ports;
pub mod actor_target;
pub mod admission;
pub mod control;
pub mod health;
pub mod observation;
pub mod parent;
pub mod sink;

use std::time::{Duration, SystemTime};

pub use actor_attempt::{
    ActorAttemptTerminal, ActorAttemptTerminalSink, NoopActorAttemptTerminalSink,
    TaskAttemptInvocationCorrelation,
};
pub use actor_plan::project_runtime_expected_type_plan;
pub use actor_ports::{SessionTaskActorOwnerPort, TaskActorOwnerPort};
pub use actor_target::{snapshot_actor_key, store_declaration_owner_to_frame};
pub use admission::RouterTaskAttemptAdmission;
pub use control::{DurableTaskControl, FirstAdmissionOutcome};
pub use health::{TaskControlCounters, TaskControlHealth};
pub use observation::RouterTaskSchedulerObservation;
pub use parent::{
    NoopTaskSubmitParentResolver, RouterTaskSubmitParentResolver, TaskSubmitParentResolver,
};
pub use sink::{DurableTaskFrameSink, ReleaseTaskExecutionImageSource, TaskExecutionImageSource};

/// Formats epoch millis as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC) for the ordinary
/// request deadline wire.
pub(crate) fn iso_timestamp(epoch_ms: u64) -> String {
    crate::health::time::format_iso_millis(SystemTime::UNIX_EPOCH + Duration::from_millis(epoch_ms))
}

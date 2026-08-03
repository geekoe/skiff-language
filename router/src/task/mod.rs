//! Durable task dispatch control plane (stage D2): the router-side owner of
//! TaskStore composition, submission/status/cancel wire handlers, the real
//! AttemptAdmission seam and attempt settlement/deadline tracking.
//!
//! Owner boundary (authoritative design "Layer Ownership"): TaskStore owns
//! durable facts, the scheduler owns claim/lease/fairness, this module owns
//! Runtime transport admission and settlement correlation. It is isolated
//! from activation/session/actor owners; it consumes them only through the
//! narrow ports defined here and in the dispatcher.

pub mod admission;
pub mod control;
pub mod health;
pub mod sink;

use std::time::{Duration, SystemTime};

pub use admission::RouterTaskAttemptAdmission;
pub use control::DurableTaskControl;
pub use health::{TaskControlCounters, TaskControlHealth};
pub use sink::{DurableTaskFrameSink, EpochTaskExecutionImageSource, TaskExecutionImageSource};

/// Formats epoch millis as `YYYY-MM-DDTHH:MM:SS.mmmZ` (UTC) for the ordinary
/// request deadline wire.
pub(crate) fn iso_timestamp(epoch_ms: u64) -> String {
    crate::health::time::format_iso_millis(
        SystemTime::UNIX_EPOCH + Duration::from_millis(epoch_ms),
    )
}

//! Injectable wall clock for deterministic store tests.
//!
//! The Mongo adapter uses the server clock (`$$NOW`) for due / expiry
//! authority and only uses this clock for bounded retry deadlines. The
//! in-memory fake uses this clock as its store authority time so contract
//! tests can drive due visibility and lease expiry deterministically.

use std::time::{SystemTime, UNIX_EPOCH};

pub trait TaskClock: Send + Sync {
    fn now_millis(&self) -> i64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl TaskClock for SystemClock {
    fn now_millis(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_positive_epoch_millis() {
        assert!(SystemClock.now_millis() > 1_600_000_000_000);
    }
}

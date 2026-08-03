//! Shared test harness: deterministic clock, record fixtures and the
//! TaskStore contract runner (reference test matrix items 5-14) exercised by
//! both the in-memory fake and the ignored Mongo live probe.
#![allow(dead_code)]

pub mod contract;
pub mod fixtures;
pub mod scheduler;

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use skiff_task_control::TaskClock;

/// Controllable store authority clock for the in-memory fake.
#[derive(Debug)]
pub struct FakeClock {
    now: AtomicI64,
}

impl FakeClock {
    pub fn new(start_millis: i64) -> Self {
        Self {
            now: AtomicI64::new(start_millis),
        }
    }

    pub fn advance(&self, millis: i64) {
        self.now.fetch_add(millis, Ordering::SeqCst);
    }

    pub fn set(&self, millis: i64) {
        self.now.store(millis, Ordering::SeqCst);
    }
}

impl TaskClock for FakeClock {
    fn now_millis(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }
}

/// Time driver shared by the contract runner: the fake advances its injected
/// clock; the Mongo probe advances by sleeping so the server clock moves.
#[derive(Clone)]
pub enum TestTime {
    Controlled(Arc<FakeClock>),
    WallClock,
}

impl TestTime {
    pub fn now_millis(&self) -> i64 {
        match self {
            Self::Controlled(clock) => clock.now_millis(),
            Self::WallClock => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
                .unwrap_or(0),
        }
    }

    pub async fn advance(&self, millis: i64) {
        match self {
            Self::Controlled(clock) => clock.advance(millis),
            Self::WallClock => {
                if millis > 0 {
                    tokio::time::sleep(Duration::from_millis(millis as u64)).await;
                }
            }
        }
    }

    pub fn rollback(&self, millis: i64) {
        if let Self::Controlled(clock) = self {
            clock.advance(-millis);
        }
    }

    pub fn is_controlled(&self) -> bool {
        matches!(self, Self::Controlled(_))
    }
}

//! Scheduler test harness: deterministic fake admission and record helpers.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use skiff_task_control::model::TaskRecord;
use skiff_task_control::scheduler::{AdmissionDecision, AttemptAdmission};
use tokio::sync::watch;

/// Scripted admission seam. The queue is consumed per call; an empty queue
/// defaults to `Accepted`. Every admitted record is recorded, and a watch
/// channel lets tests observe calls deterministically (wake fast path).
pub struct FakeAdmission {
    decisions: Mutex<VecDeque<AdmissionDecision>>,
    calls: AtomicUsize,
    admitted: Mutex<Vec<TaskRecord>>,
    notified: watch::Sender<u64>,
}

impl FakeAdmission {
    pub fn new() -> Self {
        let (notified, _) = watch::channel(0);
        Self {
            decisions: Mutex::new(VecDeque::new()),
            calls: AtomicUsize::new(0),
            admitted: Mutex::new(Vec::new()),
            notified,
        }
    }

    pub fn push(&self, decision: AdmissionDecision) {
        self.decisions
            .lock()
            .expect("decisions lock")
            .push_back(decision);
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    pub fn admitted(&self) -> Vec<TaskRecord> {
        self.admitted.lock().expect("admitted lock").clone()
    }

    /// Wait until at least `count` admit calls happened. Used to observe the
    /// wake fast path without busy polling.
    pub async fn wait_for_calls(&self, count: usize, timeout: Duration) {
        let mut rx = self.notified.subscribe();
        tokio::time::timeout(timeout, async {
            while self.calls() < count {
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await
        .expect("admission call did not arrive in time");
    }
}

impl Default for FakeAdmission {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AttemptAdmission for FakeAdmission {
    async fn admit(&self, record: &TaskRecord) -> AdmissionDecision {
        let decision = self
            .decisions
            .lock()
            .expect("decisions lock")
            .pop_front()
            .unwrap_or(AdmissionDecision::Accepted);
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.admitted
            .lock()
            .expect("admitted lock")
            .push(record.clone());
        let _ = self.notified.send(self.calls.load(Ordering::SeqCst) as u64);
        decision
    }
}

/// Shared fake admission with an owned handle, for scheduler construction.
pub type SharedFakeAdmission = Arc<FakeAdmission>;

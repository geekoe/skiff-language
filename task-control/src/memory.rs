//! In-memory TaskStore fake sharing the pure reducer with the Mongo adapter.
//!
//! The fake is the deterministic contract seam for the shared tests in
//! `tests/support/contract.rs`; its store authority time comes from an
//! injectable [`TaskClock`], so due visibility, lease expiry and wall-clock
//! rollback are fully controllable. `fail_next_transient` exists only for the
//! error-classification sequence tests and has no production role.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::clock::{SystemClock, TaskClock};
use crate::error::{invalid_record, TaskStoreError};
use crate::model::{
    AttemptId, DurableUtcTimestamp, LeaseId, TaskCancelResult, TaskCancelResultKind, TaskId,
    TaskRecord, TaskState, TaskStatus, TaskStatusKind,
};
use crate::reducer;
use crate::store::{
    BacklogObservation, CancelInput, ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput,
    LeaseRecoveryInput, LeaseRecoveryOutcome, ReleaseInput, ReleaseOutcome, RenewInput,
    RenewOutcome, RenewRejection, ScanExpiredLeasesInput, SettleInput, SettleOutcome,
    SettleTransition, StatusInput, TaskStore,
};

#[derive(Debug)]
struct MemoryState {
    records: BTreeMap<TaskId, TaskRecord>,
    next_attempt: u64,
    next_lease: u64,
    transient_failures: u64,
    closed: bool,
}

#[derive(Clone)]
pub struct MemoryTaskStore {
    inner: Arc<RwLock<MemoryState>>,
    clock: Arc<dyn TaskClock>,
}

impl Default for MemoryTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryTaskStore {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock))
    }

    pub fn with_clock(clock: Arc<dyn TaskClock>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(MemoryState {
                records: BTreeMap::new(),
                next_attempt: 0,
                next_lease: 0,
                transient_failures: 0,
                closed: false,
            })),
            clock,
        }
    }

    pub async fn fail_next_transient(&self, count: u64) {
        let mut state = self.inner.write().await;
        state.transient_failures = count;
    }

    pub async fn records(&self) -> Vec<TaskRecord> {
        let state = self.inner.read().await;
        state.records.values().cloned().collect()
    }

    async fn gate(&self) -> Result<(), TaskStoreError> {
        let mut state = self.inner.write().await;
        if state.closed {
            return Err(TaskStoreError::Closed);
        }
        if state.transient_failures > 0 {
            state.transient_failures -= 1;
            return Err(TaskStoreError::Transient {
                message: "injected transient task store failure".to_string(),
            });
        }
        Ok(())
    }

    fn now(&self) -> DurableUtcTimestamp {
        DurableUtcTimestamp::from_millis(self.clock.now_millis())
    }
}

#[async_trait]
impl TaskStore for MemoryTaskStore {
    async fn now(&self) -> Result<DurableUtcTimestamp, TaskStoreError> {
        self.gate().await?;
        Ok(self.now())
    }

    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        self.gate().await?;
        record
            .validate_create()
            .map_err(|message| invalid_record(&record.task_id, message))?;
        let mut state = self.inner.write().await;
        match state.records.get(&record.task_id) {
            Some(existing) if *existing == record => Ok(existing.clone()),
            Some(_) => Err(TaskStoreError::DuplicateTaskId {
                task_id: record.task_id.clone(),
                message: "same TaskId with a different canonical record".to_string(),
            }),
            None => {
                state.records.insert(record.task_id.clone(), record.clone());
                Ok(record)
            }
        }
    }

    async fn claim(&self, input: ClaimInput) -> Result<ClaimOutcome, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(ClaimOutcome::Rejected(ClaimRejection::NotFound));
        };
        let now = self.now();
        let attempt_id = AttemptId::new(format!("attempt-{}", state.next_attempt));
        let lease_id = LeaseId::new(format!("lease-{}", state.next_lease));
        state.next_attempt += 1;
        state.next_lease += 1;
        match reducer::claim(&record, &input, now, attempt_id, lease_id) {
            Ok(next) => {
                state.records.insert(input.task_id, next.clone());
                Ok(ClaimOutcome::Claimed(next))
            }
            Err(rejection) => Ok(ClaimOutcome::Rejected(rejection)),
        }
    }

    async fn renew(&self, input: RenewInput) -> Result<RenewOutcome, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(RenewOutcome::Rejected(RenewRejection::NotFound));
        };
        match reducer::renew(&record, &input, self.now()) {
            Ok(next) => {
                state.records.insert(input.task_id, next.clone());
                Ok(RenewOutcome::Renewed(next))
            }
            Err(rejection) => Ok(RenewOutcome::Rejected(rejection)),
        }
    }

    async fn settle(&self, input: SettleInput) -> Result<SettleOutcome, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(SettleOutcome::NotFound);
        };
        let outcome = match reducer::settle(&record, &input, self.now()) {
            SettleTransition::Settled(next) => {
                state.records.insert(input.task_id, next.clone());
                SettleOutcome::Settled(next)
            }
            SettleTransition::AlreadySettled => SettleOutcome::AlreadySettled(record),
            SettleTransition::Conflict => SettleOutcome::Conflict(record),
            SettleTransition::StaleLease => SettleOutcome::StaleLease,
            SettleTransition::ExpiredLease => SettleOutcome::ExpiredLease,
            SettleTransition::NotLeased => SettleOutcome::NotLeased,
        };
        Ok(outcome)
    }

    async fn cancel(&self, input: CancelInput) -> Result<TaskCancelResult, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(TaskCancelResult {
                kind: TaskCancelResultKind::Expired,
            });
        };
        match reducer::cancel(&record, self.now()) {
            Ok(next) => {
                state.records.insert(input.task_id, next.clone());
                Ok(TaskCancelResult {
                    kind: TaskCancelResultKind::Canceled,
                })
            }
            Err(reducer::CancelRejection::AlreadyStarted) => Ok(TaskCancelResult {
                kind: TaskCancelResultKind::AlreadyStarted,
            }),
            Err(reducer::CancelRejection::AlreadyTerminal) => Ok(TaskCancelResult {
                kind: TaskCancelResultKind::AlreadyTerminal,
            }),
        }
    }

    async fn recover_expired_lease(
        &self,
        input: LeaseRecoveryInput,
    ) -> Result<LeaseRecoveryOutcome, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(LeaseRecoveryOutcome::NotFound);
        };
        match reducer::recover_expired_lease(&record, self.now(), input.retry_not_before) {
            Ok(next) => {
                state.records.insert(input.task_id, next.clone());
                Ok(LeaseRecoveryOutcome::Recovered(next))
            }
            Err(reducer::RecoveryRejection::NotExpired) => Ok(LeaseRecoveryOutcome::NotExpired),
            Err(reducer::RecoveryRejection::NotLeased) => Ok(LeaseRecoveryOutcome::NotLeased),
            Err(reducer::RecoveryRejection::Terminal) => Ok(LeaseRecoveryOutcome::Terminal),
        }
    }

    async fn release(&self, input: ReleaseInput) -> Result<ReleaseOutcome, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let Some(record) = state.records.get(&input.task_id).cloned() else {
            return Ok(ReleaseOutcome::NotFound);
        };
        match reducer::release(&record, &input, self.now()) {
            Ok(next) => {
                state.records.insert(input.task_id, next.clone());
                Ok(ReleaseOutcome::Released(next))
            }
            Err(reducer::ReleaseRejection::StaleLease) => Ok(ReleaseOutcome::StaleLease),
            Err(reducer::ReleaseRejection::ExpiredLease) => Ok(ReleaseOutcome::ExpiredLease),
            Err(reducer::ReleaseRejection::NotLeased) => Ok(ReleaseOutcome::NotLeased),
            Err(reducer::ReleaseRejection::Terminal) => Ok(ReleaseOutcome::Terminal),
        }
    }

    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.gate().await?;
        let mut state = self.inner.write().await;
        let now = self.now();
        let mut due = Vec::new();
        for record in state.records.values_mut() {
            if let Some(advanced) = reducer::advance_due(record, now) {
                *record = advanced;
            }
            if record.state == TaskState::Ready && record.due_at <= now {
                due.push(record.clone());
            }
        }
        due.sort_by_key(|record| record.due_at);
        due.truncate(input.limit);
        Ok(due)
    }

    async fn scan_expired_leases(
        &self,
        input: ScanExpiredLeasesInput,
    ) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.gate().await?;
        let state = self.inner.read().await;
        let now = self.now();
        let mut expired: Vec<TaskRecord> = state
            .records
            .values()
            .filter(|record| {
                record.state == TaskState::Leased
                    && record
                        .active_lease
                        .as_ref()
                        .is_some_and(|lease| lease.expiry <= now)
            })
            .cloned()
            .collect();
        expired.sort_by_key(|record| {
            record
                .active_lease
                .as_ref()
                .map(|lease| lease.expiry)
                .unwrap_or(DurableUtcTimestamp::from_millis(0))
        });
        expired.truncate(input.limit);
        Ok(expired)
    }

    async fn status(&self, input: StatusInput) -> Result<TaskStatus, TaskStoreError> {
        self.gate().await?;
        let state = self.inner.read().await;
        let Some(record) = state.records.get(&input.task_id) else {
            return Ok(TaskStatus {
                kind: TaskStatusKind::Expired,
            });
        };
        let expires = record
            .created_at
            .checked_add_millis(input.retention.millis());
        if expires.is_some_and(|expires| self.now() >= expires) {
            return Ok(TaskStatus {
                kind: TaskStatusKind::Expired,
            });
        }
        Ok(TaskStatus {
            kind: record.status_kind(),
        })
    }

    async fn observe_backlog(&self) -> Result<BacklogObservation, TaskStoreError> {
        self.gate().await?;
        let state = self.inner.read().await;
        let mut observation = BacklogObservation::default();
        for record in state.records.values() {
            match record.state {
                TaskState::Scheduled => {
                    observation.scheduled += 1;
                    observation.oldest_due_at =
                        Some(older(observation.oldest_due_at, record.due_at));
                }
                TaskState::Ready => {
                    observation.ready += 1;
                    observation.oldest_due_at =
                        Some(older(observation.oldest_due_at, record.due_at));
                }
                TaskState::Leased => observation.leased += 1,
                _ => {}
            }
        }
        Ok(observation)
    }

    async fn ensure_indexes(&self) -> Result<(), TaskStoreError> {
        Ok(())
    }

    async fn close(&self) -> Result<(), TaskStoreError> {
        self.inner.write().await.closed = true;
        Ok(())
    }
}

fn older(
    current: Option<DurableUtcTimestamp>,
    candidate: DurableUtcTimestamp,
) -> DurableUtcTimestamp {
    match current {
        Some(current) => current.min(candidate),
        None => candidate,
    }
}

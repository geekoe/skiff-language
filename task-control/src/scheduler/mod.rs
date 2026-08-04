//! Durable task dispatch scheduler (stage C2).
//!
//! The scheduler only selects work from TaskStore-visible facts: it is the
//! owner of timing, fairness, capacity, claim and Runtime candidate
//! selection, and it never interprets the business payload. Correctness
//! across replicas comes from the store's conditional claim / CAS, not from
//! leader election; every replica runs the same due scanner, lease-expiry
//! recovery loop and wake fast path.
//!
//! Durable timing decisions (lease expiry, retry not-before) always use
//! [`TaskStore::now`] as authority time. The injected clock is only used for
//! local loop pacing; the wake channel is a local fast-path optimization and
//! never bypasses TaskStore fairness.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::watch;

use crate::clock::TaskClock;
use crate::model::{
    DurableDuration, DurableUtcTimestamp, LeaseId, TaskId, TaskLease, TaskOutcome, TaskRecord,
    TaskState, TaskTerminal,
};
use crate::store::{
    ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput, LeaseRecoveryInput,
    LeaseRecoveryOutcome, ReleaseInput, ReleaseOutcome, RenewInput, RenewOutcome, RenewRejection,
    ScanExpiredLeasesInput, SettleInput, TaskStore,
};

pub mod admission;
pub mod backoff;

pub use admission::{AdmissionDecision, AttemptAdmission};
pub use backoff::{FixedJitter, Jitter, LcgJitter, RetryBackoffPolicy};

/// Observability seam for scheduler-owned transitions (authoritative design
/// "Observability And Retention": claim / eligible wait / lease renew / loss /
/// recovery / duplicate notification absorption / provable-rejection release).
///
/// The seam is strictly read-only: observers must never mutate store records,
/// leases, backoff or scheduler policy. The default implementation is a no-op,
/// so task-control has no telemetry dependency.
pub trait SchedulerObservation: Send + Sync {
    /// A scheduled record became due-visible in this scan (`ready` at store
    /// authority `now`).
    fn on_due_ready(&self, _record: &TaskRecord, _now: DurableUtcTimestamp) {}

    /// This replica won the claim CAS for the record (fresh attempt / lease).
    fn on_claim(&self, _record: &TaskRecord, _now: DurableUtcTimestamp) {}

    /// A claim CAS was rejected for an already visible task (ordinary
    /// duplicate delivery absorption; never creates a second logical task).
    fn on_claim_duplicate(&self, _task_id: &TaskId, _reason: &ClaimRejection) {}

    /// One accepted lease was renewed at store authority time.
    fn on_renewed(&self, _task_id: &TaskId, _lease_id: &LeaseId, _new_expiry: DurableUtcTimestamp) {
    }

    /// Renewal of one accepted lease was rejected (stale / expired / terminal /
    /// missing); bookkeeping for that exact lease ends.
    fn on_renew_lost(&self, _task_id: &TaskId, _lease_id: &LeaseId, _rejection: RenewRejection) {}

    /// Lease-expiry recovery won the CAS: the task is `ready` again and the
    /// next attempt is paced by the durable retry not-before.
    fn on_recover(&self, _task_id: &TaskId, _lease_id: &LeaseId) {}

    /// A provable-rejection release won the CAS: the lease is returned to
    /// `ready` with the scheduler-owned backoff.
    fn on_release(
        &self,
        _task_id: &TaskId,
        _lease_id: &LeaseId,
        _retry_not_before: DurableUtcTimestamp,
    ) {
    }
}

/// Default no-op observation (plain `Scheduler::new` keeps task-control free
/// of producers).
#[derive(Debug, Default)]
pub struct NoopSchedulerObservation;

impl SchedulerObservation for NoopSchedulerObservation {}

/// Scheduler policy and local loop configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Lease owner witness recorded on every claim made by this replica.
    pub scheduler_id: String,
    /// Local scan cadence. Renewals of accepted leases run on the same
    /// cadence, so `lease_duration` must be at least twice this interval.
    pub scan_interval: Duration,
    /// Batch cap for due scans and expired-lease scans (capacity / fairness
    /// boundary; due order is `due_at` ascending).
    pub batch_limit: usize,
    /// Lease span chosen per claim (store authority now + this duration).
    pub lease_duration: DurableDuration,
    /// Stage-C2 precondition for claim: the frozen execution image can be
    /// reactivated. The real Router admission (D2) will evaluate activation;
    /// this node exposes the flag so tests and composition choose it.
    pub image_activatable: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            scheduler_id: "scheduler".to_string(),
            scan_interval: Duration::from_secs(1),
            batch_limit: 64,
            lease_duration: DurableDuration::from_millis(60_000),
            image_activatable: true,
        }
    }
}

impl SchedulerConfig {
    /// Validate the loop / lease cadence invariant and non-empty batch.
    pub fn validate(&self) -> Result<(), String> {
        if self.scheduler_id.is_empty() {
            return Err("scheduler id must not be empty".to_string());
        }
        if self.scan_interval.is_zero() {
            return Err("scan interval must be positive".to_string());
        }
        if self.batch_limit == 0 {
            return Err("batch limit must be positive".to_string());
        }
        let interval_ms = i64::try_from(self.scan_interval.as_millis()).unwrap_or(i64::MAX);
        if self.lease_duration.millis() < 2 * interval_ms {
            return Err(
                "lease_duration must be at least twice scan_interval so loop renewals keep accepted leases alive"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// One scheduler replica. Cheap to clone: all mutable state is shared.
pub struct Scheduler {
    store: Arc<dyn TaskStore>,
    admission: Arc<dyn AttemptAdmission>,
    clock: Arc<dyn TaskClock>,
    config: SchedulerConfig,
    backoff: Arc<RetryBackoffPolicy>,
    observer: Arc<dyn SchedulerObservation>,
    /// Leases this replica accepted and must renew while the attempt is
    /// pending. Settlement / lease loss removes the entry by lease id; a
    /// later claim for the same TaskId has a fresh lease id and is never
    /// removed by stale bookkeeping.
    active_leases: Mutex<HashMap<TaskId, TaskLease>>,
    wake_tx: watch::Sender<u64>,
    wake_rx: watch::Receiver<u64>,
}

impl Clone for Scheduler {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            admission: self.admission.clone(),
            clock: self.clock.clone(),
            config: self.config.clone(),
            backoff: self.backoff.clone(),
            observer: self.observer.clone(),
            active_leases: Mutex::new(
                self.active_leases
                    .lock()
                    .expect("active leases lock")
                    .clone(),
            ),
            wake_tx: self.wake_tx.clone(),
            wake_rx: self.wake_rx.clone(),
        }
    }
}

impl Scheduler {
    pub fn new(
        store: Arc<dyn TaskStore>,
        admission: Arc<dyn AttemptAdmission>,
        clock: Arc<dyn TaskClock>,
        config: SchedulerConfig,
        backoff: RetryBackoffPolicy,
    ) -> Self {
        Self::with_observer(
            store,
            admission,
            clock,
            config,
            backoff,
            Arc::new(NoopSchedulerObservation),
        )
    }

    /// Scheduler with an injected observability seam (router telemetry).
    /// The observer is read-only and never influences execution semantics.
    pub fn with_observer(
        store: Arc<dyn TaskStore>,
        admission: Arc<dyn AttemptAdmission>,
        clock: Arc<dyn TaskClock>,
        config: SchedulerConfig,
        backoff: RetryBackoffPolicy,
        observer: Arc<dyn SchedulerObservation>,
    ) -> Self {
        config.validate().expect("invalid scheduler config");
        let (wake_tx, wake_rx) = watch::channel(0);
        Self {
            store,
            admission,
            clock,
            config,
            backoff: Arc::new(backoff),
            observer,
            active_leases: Mutex::new(HashMap::new()),
            wake_tx,
            wake_rx,
        }
    }

    /// Wake fast path: called after an immediate task's durable commit so
    /// this replica runs a cycle without waiting for the scan interval.
    pub fn wake(&self) {
        let next = self.wake_tx.borrow().saturating_add(1);
        let _ = self.wake_tx.send(next);
    }

    /// Number of accepted attempts this replica is currently renewing.
    pub fn active_lease_count(&self) -> usize {
        self.active_leases.lock().expect("active leases lock").len()
    }

    /// Stops renewing one exact accepted lease. Used by the control plane
    /// when an attempt terminal is uncertain (disconnect, shutdown, protocol
    /// loss): the task is neither settled nor released, so lease expiry at
    /// store authority time drives recovery with platform backoff.
    pub fn forget_active_lease(&self, task_id: &TaskId, lease_id: &LeaseId) {
        self.remove_active_lease_if(task_id, lease_id);
    }

    /// Main loop: waits for a wake or the scan interval, then runs one
    /// recovery + renewal + due-scan cycle. Runs forever until the task is
    /// aborted or the store is closed.
    pub async fn run(&self) {
        // The wake receiver is cloned once and reused for every cycle.
        // Cloning inside the wait would copy the receiver's "seen version"
        // from `wake_rx`, which is never advanced by `borrow_and_update`; a
        // fresh clone would then see every new wake as already seen and the
        // loop would never sleep after the first wake.
        let mut wake = self.wake_rx.clone();
        loop {
            self.wait_for_cycle(&mut wake).await;
            self.run_cycle().await;
        }
    }

    /// One full cycle: lease-expiry recovery, renewal of accepted leases,
    /// then due scan + claim + admission.
    pub async fn run_cycle(&self) {
        self.recover_once().await;
        self.renew_active_leases().await;
        self.scan_once().await;
    }

    /// Due scanner: claims due `ready` tasks in `due_at` order, bounded by
    /// `batch_limit`, and routes each claim through the admission seam.
    pub async fn scan_once(&self) {
        let Ok(now) = self.store.now().await else {
            return;
        };
        let Ok(records) = self
            .store
            .scan_due(DueScanInput {
                limit: self.config.batch_limit,
            })
            .await
        else {
            return;
        };
        for record in records {
            if record.state != TaskState::Ready {
                continue;
            }
            self.observer.on_due_ready(&record, now);
            if record
                .retry_not_before
                .is_some_and(|not_before| not_before > now)
            {
                continue;
            }
            if !self.config.image_activatable {
                continue;
            }
            let Some(lease_expiry) = now.checked_add_millis(self.config.lease_duration.millis())
            else {
                continue;
            };
            let claim = self
                .store
                .claim(ClaimInput {
                    task_id: record.task_id.clone(),
                    owner: self.config.scheduler_id.clone(),
                    lease_expiry,
                    image_activatable: self.config.image_activatable,
                })
                .await;
            let record = match claim {
                Ok(ClaimOutcome::Claimed(record)) => {
                    self.observer.on_claim(&record, now);
                    record
                }
                Ok(ClaimOutcome::Rejected(reason)) => {
                    self.observer.on_claim_duplicate(&record.task_id, &reason);
                    continue;
                }
                Err(_) => continue,
            };
            let decision = self.admission.admit(&record).await;
            self.handle_decision(record, decision, now).await;
        }
    }

    /// Lease-expiry recovery loop: recovers every expired lease visible in
    /// the store (from any replica), atomically applying platform backoff so
    /// the next attempt cannot hot retry.
    pub async fn recover_once(&self) {
        let Ok(now) = self.store.now().await else {
            return;
        };
        let Ok(records) = self
            .store
            .scan_expired_leases(ScanExpiredLeasesInput {
                limit: self.config.batch_limit,
            })
            .await
        else {
            return;
        };
        for record in records {
            if record.state != TaskState::Leased {
                continue;
            }
            let Some(lease) = record.active_lease.as_ref() else {
                continue;
            };
            let Some(retry_not_before) =
                now.checked_add_millis(self.backoff.delay_millis(record.attempt_generation))
            else {
                continue;
            };
            let recovered = self
                .store
                .recover_expired_lease(LeaseRecoveryInput {
                    task_id: record.task_id.clone(),
                    retry_not_before,
                })
                .await;
            if matches!(recovered, Ok(LeaseRecoveryOutcome::Recovered(_))) {
                self.observer.on_recover(&record.task_id, &lease.lease_id);
                self.remove_active_lease_if(&record.task_id, &lease.lease_id);
            }
        }
    }

    /// Renew every accepted lease this replica is tracking. A rejected renew
    /// (settled terminal, stale lease, expiry) ends bookkeeping for that
    /// exact lease id; transient store errors are retried next cycle.
    pub async fn renew_active_leases(&self) {
        let Ok(now) = self.store.now().await else {
            return;
        };
        let Some(new_expiry) = now.checked_add_millis(self.config.lease_duration.millis()) else {
            return;
        };
        let active: Vec<(TaskId, TaskLease)> = {
            let guard = self.active_leases.lock().expect("active leases lock");
            guard
                .iter()
                .map(|(task_id, lease)| (task_id.clone(), lease.clone()))
                .collect()
        };
        for (task_id, lease) in active {
            let renewed = self
                .store
                .renew(RenewInput {
                    task_id: task_id.clone(),
                    lease_id: lease.lease_id.clone(),
                    new_expiry,
                })
                .await;
            match renewed {
                Ok(RenewOutcome::Renewed(record)) => {
                    self.observer.on_renewed(
                        &task_id,
                        &lease.lease_id,
                        record
                            .active_lease
                            .as_ref()
                            .map(|next| next.expiry)
                            .unwrap_or(new_expiry),
                    );
                    if let Some(next) = record.active_lease.as_ref().cloned() {
                        let mut guard = self.active_leases.lock().expect("active leases lock");
                        if guard
                            .get(&task_id)
                            .is_some_and(|current| current.lease_id == lease.lease_id)
                        {
                            guard.insert(task_id, next);
                        }
                    }
                }
                Ok(RenewOutcome::Rejected(rejection)) => {
                    if matches!(
                        rejection,
                        RenewRejection::StaleLease
                            | RenewRejection::ExpiredLease
                            | RenewRejection::NotLeased
                            | RenewRejection::Terminal
                            | RenewRejection::NotFound
                    ) {
                        self.observer
                            .on_renew_lost(&task_id, &lease.lease_id, rejection.clone());
                        self.remove_active_lease_if(&task_id, &lease.lease_id);
                    }
                }
                Err(_) => {}
            }
        }
    }

    async fn handle_decision(
        &self,
        record: TaskRecord,
        decision: AdmissionDecision,
        now: DurableUtcTimestamp,
    ) {
        match decision {
            AdmissionDecision::Accepted => {
                if let Some(lease) = record.active_lease.as_ref().cloned() {
                    self.active_leases
                        .lock()
                        .expect("active leases lock")
                        .insert(record.task_id.clone(), lease);
                }
            }
            AdmissionDecision::RejectedProvable { .. } => {
                let Some(lease) = record.active_lease.as_ref() else {
                    return;
                };
                let Some(retry_not_before) =
                    now.checked_add_millis(self.backoff.delay_millis(record.attempt_generation))
                else {
                    return;
                };
                if matches!(
                    self.store
                        .release(ReleaseInput {
                            task_id: record.task_id.clone(),
                            lease_id: lease.lease_id.clone(),
                            retry_not_before,
                        })
                        .await,
                    Ok(ReleaseOutcome::Released(_))
                ) {
                    self.observer
                        .on_release(&record.task_id, &lease.lease_id, retry_not_before);
                }
                self.remove_active_lease_if(&record.task_id, &lease.lease_id);
            }
            AdmissionDecision::Uncertain { .. } => {
                // No settlement and no release: the attempt outcome cannot be
                // proven either way. Lease expiry is the store authority
                // arbiter; recovery produces a new attempt with backoff.
            }
            AdmissionDecision::PermanentFailure { reason } => {
                let Some(lease) = record.active_lease.as_ref() else {
                    return;
                };
                let terminal = TaskTerminal {
                    settled_at: now,
                    outcome: TaskOutcome::PlatformFailed { reason },
                };
                let _ = self
                    .store
                    .settle(SettleInput {
                        task_id: record.task_id.clone(),
                        lease_id: lease.lease_id.clone(),
                        terminal,
                    })
                    .await;
                self.remove_active_lease_if(&record.task_id, &lease.lease_id);
            }
        }
    }

    fn remove_active_lease_if(&self, task_id: &TaskId, lease_id: &LeaseId) {
        let mut guard = self.active_leases.lock().expect("active leases lock");
        if guard
            .get(task_id)
            .is_some_and(|lease| &lease.lease_id == lease_id)
        {
            guard.remove(task_id);
        }
    }

    async fn wait_for_cycle(&self, wake: &mut watch::Receiver<u64>) {
        let interval_ms = i64::try_from(self.config.scan_interval.as_millis()).unwrap_or(i64::MAX);
        let next = self.clock.now_millis().saturating_add(interval_ms);
        loop {
            let now = self.clock.now_millis();
            if now >= next {
                return;
            }
            let wait = Duration::from_millis((next - now).max(0) as u64);
            tokio::select! {
                _ = wake.changed() => return,
                _ = tokio::time::sleep(wait) => {}
            }
        }
    }
}

/// Convenience alias for the store handle consumed by schedulers.
pub type TaskStoreHandle = Arc<dyn TaskStore>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use async_trait::async_trait;

    #[test]
    fn config_validation_rejects_hot_lease_cadence() {
        let mut config = SchedulerConfig::default();
        config.lease_duration = DurableDuration::from_millis(1);
        assert!(config.validate().is_err(), "lease shorter than cadence");
        config.lease_duration = DurableDuration::from_millis(2_000);
        assert!(config.validate().is_ok());
        config.batch_limit = 0;
        assert!(config.validate().is_err());
    }

    #[test]
    fn wake_counter_is_monotonic() {
        let store: TaskStoreHandle = Arc::new(crate::MemoryTaskStore::new());
        let admission = Arc::new(NoopAdmission);
        let scheduler = Scheduler::new(
            store,
            admission,
            Arc::new(SystemClock),
            SchedulerConfig::default(),
            RetryBackoffPolicy::default(),
        );
        let first = *scheduler.wake_tx.borrow();
        scheduler.wake();
        scheduler.wake();
        assert_eq!(*scheduler.wake_tx.borrow(), first + 2);
    }

    #[test]
    fn wake_is_consumed_exactly_once_per_wake() {
        let store: TaskStoreHandle = Arc::new(crate::MemoryTaskStore::new());
        let admission = Arc::new(NoopAdmission);
        let scheduler = Scheduler::new(
            store,
            admission,
            Arc::new(SystemClock),
            SchedulerConfig::default(),
            RetryBackoffPolicy::default(),
        );
        let mut wake = scheduler.wake_rx.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");

        runtime.block_on(async {
            // One wake makes the first wait return immediately.
            scheduler.wake();
            tokio::time::timeout(
                Duration::from_millis(100),
                scheduler.wait_for_cycle(&mut wake),
            )
            .await
            .expect("first wait should return immediately after wake");

            // Without another wake the next wait must not return immediately:
            // the wake notification was consumed, so the wait falls through to
            // the scan-interval sleep and the timeout wins.
            let second = tokio::time::timeout(
                Duration::from_millis(100),
                scheduler.wait_for_cycle(&mut wake),
            )
            .await;
            assert!(
                second.is_err(),
                "second wait without a new wake must not return immediately (hot loop)"
            );

            // A fresh wake is observed again and returns immediately.
            scheduler.wake();
            tokio::time::timeout(
                Duration::from_millis(100),
                scheduler.wait_for_cycle(&mut wake),
            )
            .await
            .expect("third wait should return immediately after a fresh wake");
        });
    }

    struct NoopAdmission;

    #[async_trait]
    impl AttemptAdmission for NoopAdmission {
        async fn admit(&self, _record: &TaskRecord) -> AdmissionDecision {
            AdmissionDecision::Accepted
        }
    }
}

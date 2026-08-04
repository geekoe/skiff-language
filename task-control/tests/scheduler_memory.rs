//! Focused scheduler tests against the in-memory TaskStore (reference test
//! matrix items 6, 7, 8, 9, 12, 13, 14 scheduler parts + multi-replica
//! concurrency). All durable timing is driven by the fake store clock.

mod support;

use std::collections::HashSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use async_trait::async_trait;
use skiff_task_control::model::{
    DurableDuration, DurableUtcTimestamp, LeaseId, TaskId, TaskOutcome, TaskRecord, TaskState,
    TaskStatusKind, TaskTerminal,
};
use skiff_task_control::scheduler::{
    AdmissionDecision, FixedJitter, RetryBackoffPolicy, Scheduler, SchedulerConfig,
    SchedulerObservation,
};
use skiff_task_control::store::{
    BacklogObservation, CancelInput, ClaimInput, ClaimOutcome, ClaimRejection, DueScanInput,
    LeaseRecoveryInput, LeaseRecoveryOutcome, ReleaseInput, ReleaseOutcome, RenewInput,
    RenewOutcome, RenewRejection, ScanExpiredLeasesInput, SettleInput, SettleOutcome, StatusInput,
    TaskStore,
};
use skiff_task_control::{MemoryTaskStore, SystemClock, TaskClock, TaskStoreError};

use support::scheduler::FakeAdmission;
use support::{fixtures, FakeClock};

const START_MILLIS: i64 = 1_700_000_000_000;

fn test_config(scheduler_id: &str) -> SchedulerConfig {
    SchedulerConfig {
        scheduler_id: scheduler_id.to_string(),
        scan_interval: Duration::from_millis(1),
        min_cycle_interval: Duration::from_millis(10),
        batch_limit: 128,
        lease_duration: DurableDuration::from_millis(60_000),
        image_activatable: true,
    }
}

fn test_backoff(base_ms: i64, max_ms: i64, jitter_ms: i64) -> RetryBackoffPolicy {
    RetryBackoffPolicy::with_jitter(
        DurableDuration::from_millis(base_ms),
        DurableDuration::from_millis(max_ms),
        DurableDuration::from_millis(jitter_ms + 1),
        Box::new(FixedJitter(jitter_ms)),
    )
    .expect("valid test backoff")
}

fn immediate_record(seed: u64) -> skiff_task_control::model::TaskRecord {
    let mut record = fixtures::record(seed, START_MILLIS);
    record.created_at = DurableUtcTimestamp::from_millis(START_MILLIS);
    record
}

async fn record(store: &MemoryTaskStore, task_id: TaskId) -> skiff_task_control::model::TaskRecord {
    store
        .records()
        .await
        .into_iter()
        .find(|record| record.task_id == task_id)
        .unwrap_or_else(|| panic!("task {task_id} missing"))
}

fn task_id(seed: u64) -> TaskId {
    TaskId::new(format!("task-{seed}"))
}

#[derive(Debug, Default)]
struct RecordingObservation {
    events: Mutex<Vec<String>>,
}

impl RecordingObservation {
    fn events(&self) -> Vec<String> {
        self.events.lock().expect("observation lock").clone()
    }

    fn push(&self, event: String) {
        self.events.lock().expect("observation lock").push(event);
    }
}

impl SchedulerObservation for RecordingObservation {
    fn on_due_ready(&self, record: &TaskRecord, _now: DurableUtcTimestamp) {
        self.push(format!("ready:{}", record.task_id.as_str()));
    }

    fn on_claim(&self, record: &TaskRecord, _now: DurableUtcTimestamp) {
        self.push(format!("claim:{}", record.task_id.as_str()));
    }

    fn on_claim_duplicate(&self, task_id: &TaskId, reason: &ClaimRejection) {
        self.push(format!("duplicate:{}:{reason:?}", task_id.as_str()));
    }

    fn on_renewed(&self, task_id: &TaskId, _lease_id: &LeaseId, _new_expiry: DurableUtcTimestamp) {
        self.push(format!("renewed:{}", task_id.as_str()));
    }

    fn on_renew_lost(&self, task_id: &TaskId, _lease_id: &LeaseId, rejection: RenewRejection) {
        self.push(format!("renewLost:{}:{rejection:?}", task_id.as_str()));
    }

    fn on_recover(&self, task_id: &TaskId, _lease_id: &LeaseId) {
        self.push(format!("recovered:{}", task_id.as_str()));
    }

    fn on_release(
        &self,
        task_id: &TaskId,
        _lease_id: &LeaseId,
        _retry_not_before: DurableUtcTimestamp,
    ) {
        self.push(format!("released:{}", task_id.as_str()));
    }
}

/// Deterministic duplicate-delivery script: the next claim CAS is answered
/// `AlreadyLeased` (as if another replica won the race) while the due scan
/// still reports the record as ready. All other operations delegate to the
/// in-memory store, so the observation seam path is exercised without racing.
struct ScriptedDuplicateStore {
    inner: MemoryTaskStore,
    duplicate_next_claim: AtomicBool,
}

#[async_trait]
impl TaskStore for ScriptedDuplicateStore {
    async fn now(&self) -> Result<DurableUtcTimestamp, TaskStoreError> {
        self.inner.now().await
    }

    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError> {
        self.inner.create(record).await
    }

    async fn claim(&self, input: ClaimInput) -> Result<ClaimOutcome, TaskStoreError> {
        if self.duplicate_next_claim.swap(false, Ordering::SeqCst) {
            return Ok(ClaimOutcome::Rejected(ClaimRejection::AlreadyLeased));
        }
        self.inner.claim(input).await
    }

    async fn renew(&self, input: RenewInput) -> Result<RenewOutcome, TaskStoreError> {
        self.inner.renew(input).await
    }

    async fn settle(&self, input: SettleInput) -> Result<SettleOutcome, TaskStoreError> {
        self.inner.settle(input).await
    }

    async fn cancel(
        &self,
        input: CancelInput,
    ) -> Result<skiff_task_control::model::TaskCancelResult, TaskStoreError> {
        self.inner.cancel(input).await
    }

    async fn recover_expired_lease(
        &self,
        input: LeaseRecoveryInput,
    ) -> Result<LeaseRecoveryOutcome, TaskStoreError> {
        self.inner.recover_expired_lease(input).await
    }

    async fn release(&self, input: ReleaseInput) -> Result<ReleaseOutcome, TaskStoreError> {
        self.inner.release(input).await
    }

    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_due(input).await
    }

    async fn scan_expired_leases(
        &self,
        input: ScanExpiredLeasesInput,
    ) -> Result<Vec<TaskRecord>, TaskStoreError> {
        self.inner.scan_expired_leases(input).await
    }

    async fn status(
        &self,
        input: StatusInput,
    ) -> Result<skiff_task_control::model::TaskStatus, TaskStoreError> {
        self.inner.status(input).await
    }

    async fn observe_backlog(&self) -> Result<BacklogObservation, TaskStoreError> {
        self.inner.observe_backlog().await
    }

    async fn ensure_indexes(&self) -> Result<(), TaskStoreError> {
        self.inner.ensure_indexes().await
    }

    async fn close(&self) -> Result<(), TaskStoreError> {
        self.inner.close().await
    }
}

#[tokio::test]
async fn scheduler_observation_seam_records_lifecycle_events() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let observation = Arc::new(RecordingObservation::default());
    let scheduler = Scheduler::with_observer(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("obs"),
        test_backoff(100, 400, 0),
        observation.clone(),
    );

    store
        .create(immediate_record(501))
        .await
        .expect("create task");
    scheduler.scan_once().await;
    let after_scan = observation.events();
    assert!(
        after_scan.iter().any(|event| event == "ready:task-501"),
        "scheduled -> ready must be observed, got {after_scan:?}"
    );
    assert!(
        after_scan.iter().any(|event| event == "claim:task-501"),
        "claim must be observed, got {after_scan:?}"
    );

    scheduler.renew_active_leases().await;
    assert!(
        observation
            .events()
            .iter()
            .any(|event| event == "renewed:task-501"),
        "accepted lease renewal must be observed"
    );

    // Deterministic duplicate-delivery absorption: the due scan reports the
    // record ready, but the claim CAS loses to another replica.
    let scripted = Arc::new(ScriptedDuplicateStore {
        inner: MemoryTaskStore::with_clock(clock.clone()),
        duplicate_next_claim: AtomicBool::new(true),
    });
    let duplicate_observer = Arc::new(RecordingObservation::default());
    let duplicate_scheduler = Scheduler::with_observer(
        scripted.clone(),
        Arc::new(FakeAdmission::new()),
        clock.clone(),
        test_config("obs-duplicate"),
        test_backoff(100, 400, 0),
        duplicate_observer.clone(),
    );
    scripted
        .create(immediate_record(502))
        .await
        .expect("create duplicate task");
    duplicate_scheduler.scan_once().await;
    assert!(
        duplicate_observer
            .events()
            .iter()
            .any(|event| event == "duplicate:task-502:AlreadyLeased"),
        "duplicate claim rejection must be observed"
    );

    clock.advance(60_000);
    scheduler.recover_once().await;
    assert!(
        observation
            .events()
            .iter()
            .any(|event| event == "recovered:task-501"),
        "lease-expiry recovery must be observed"
    );
}

#[tokio::test]
async fn due_scan_respects_due_at_and_clock_rollback() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(100, 400, 7),
    );

    let future = fixtures::record(301, START_MILLIS + 10_000);
    store.create(future).await.expect("create future task");
    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        0,
        "future task must not be visible before due"
    );

    clock.advance(10_000);
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1, "task is claimable once due");
    let claimed = record(&store, task_id(301)).await;
    assert_eq!(claimed.state, TaskState::Leased);
    assert_eq!(claimed.attempt_generation, 1);

    // Wall-clock rollback cannot reveal a not-yet-due task.
    store
        .create(fixtures::record(302, START_MILLIS + 20_000))
        .await
        .expect("create second future task");
    clock.advance(20_000);
    clock.set(START_MILLIS + 5_000); // roll back before due
    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        1,
        "rollback must not reveal a future task"
    );
    assert_eq!(
        record(&store, task_id(302)).await.state,
        TaskState::Scheduled
    );
}

#[tokio::test]
async fn concurrent_replicas_claim_each_task_exactly_once() {
    const TASK_COUNT: u64 = 32;
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    for seed in 0..TASK_COUNT {
        store
            .create(immediate_record(seed))
            .await
            .expect("create task");
    }

    let admission_a = Arc::new(FakeAdmission::new());
    let admission_b = Arc::new(FakeAdmission::new());
    let scheduler_a = Scheduler::new(
        store.clone(),
        admission_a.clone(),
        clock.clone(),
        test_config("scheduler-a"),
        test_backoff(100, 400, 10),
    );
    let scheduler_b = Scheduler::new(
        store.clone(),
        admission_b.clone(),
        clock.clone(),
        test_config("scheduler-b"),
        test_backoff(100, 400, 10),
    );

    tokio::join!(scheduler_a.scan_once(), scheduler_b.scan_once());

    assert_eq!(
        admission_a.calls() + admission_b.calls(),
        TASK_COUNT as usize,
        "every task must be admitted exactly once across replicas"
    );
    let records = store.records().await;
    assert_eq!(records.len(), TASK_COUNT as usize);
    for record in records {
        assert_eq!(record.state, TaskState::Leased);
        assert_eq!(record.attempt_generation, 1, "no task may be claimed twice");
    }
    let mut admitted_ids = HashSet::new();
    for admitted in admission_a
        .admitted()
        .into_iter()
        .chain(admission_b.admitted())
    {
        let task_id = admitted.task_id.clone();
        assert!(
            admitted_ids.insert(task_id.clone()),
            "duplicate claim for {task_id}"
        );
    }
    assert_eq!(admitted_ids.len(), TASK_COUNT as usize);
}

#[tokio::test]
async fn lease_expiry_recovery_creates_new_attempt_with_backoff() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(1_000, 4_000, 0),
    );

    store.create(immediate_record(401)).await.expect("create");
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1);
    assert_eq!(record(&store, task_id(401)).await.attempt_generation, 1);

    // Lease expires at store authority time; recovery returns ready and
    // atomically writes retry_not_before (base 1000ms, jitter 0).
    clock.advance(60_000);
    scheduler.recover_once().await;
    let recovered = record(&store, task_id(401)).await;
    assert_eq!(recovered.state, TaskState::Ready);
    assert!(recovered.active_lease.is_none());
    assert_eq!(
        recovered.retry_not_before,
        Some(DurableUtcTimestamp::from_millis(START_MILLIS + 61_000))
    );

    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        1,
        "backoff must prevent hot retry before retry_not_before"
    );

    clock.advance(1_000);
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 2, "new attempt after backoff elapses");
    let retried = record(&store, task_id(401)).await;
    assert_eq!(retried.attempt_generation, 2, "generation advances");
    assert_eq!(retried.state, TaskState::Leased);
}

#[tokio::test]
async fn accepted_attempt_renews_until_settled() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(100, 400, 10),
    );

    store.create(immediate_record(501)).await.expect("create");
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1);
    assert_eq!(scheduler.active_lease_count(), 1, "accepted lease tracked");

    let first = record(&store, task_id(501)).await;
    let first_expiry = first.active_lease.as_ref().expect("lease").expiry.millis();

    // Pending attempt: renewal extends the lease on store authority time.
    clock.advance(10_000);
    scheduler.renew_active_leases().await;
    let renewed = record(&store, task_id(501)).await;
    let renewed_expiry = renewed
        .active_lease
        .as_ref()
        .expect("lease")
        .expiry
        .millis();
    assert!(renewed_expiry > first_expiry, "lease must be renewed");
    assert_eq!(
        renewed_expiry,
        START_MILLIS + 10_000 + 60_000,
        "renewal uses store authority now + configured lease span"
    );

    // Runtime settles the attempt with the current lease id.
    let lease = renewed.active_lease.clone().expect("lease");
    let settled = store
        .settle(SettleInput {
            task_id: task_id(501),
            lease_id: lease.lease_id.clone(),
            terminal: TaskTerminal {
                settled_at: DurableUtcTimestamp::from_millis(START_MILLIS + 10_000),
                outcome: TaskOutcome::Succeeded,
            },
        })
        .await
        .expect("settle");
    assert!(matches!(settled, SettleOutcome::Settled(_)));

    scheduler.renew_active_leases().await;
    assert_eq!(
        scheduler.active_lease_count(),
        0,
        "settlement must end renewal for that lease"
    );
    let terminal = record(&store, task_id(501)).await;
    assert_eq!(terminal.state, TaskState::Succeeded);
    assert!(terminal.active_lease.is_none());
}

#[tokio::test]
async fn provable_rejection_releases_claim_with_backoff() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(1_000, 4_000, 0),
    );
    admission.push(AdmissionDecision::RejectedProvable {
        reason: "runtime admission refused".to_string(),
    });

    store.create(immediate_record(601)).await.expect("create");
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1);
    assert_eq!(scheduler.active_lease_count(), 0);

    let released = record(&store, task_id(601)).await;
    assert_eq!(
        released.state,
        TaskState::Ready,
        "provable rejection releases the claim"
    );
    assert!(released.active_lease.is_none());
    assert_eq!(
        released.retry_not_before,
        Some(DurableUtcTimestamp::from_millis(START_MILLIS + 1_000))
    );

    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        1,
        "released task must not hot retry before backoff"
    );
    clock.advance(1_000);
    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        2,
        "task can be claimed again after backoff"
    );
    assert_eq!(record(&store, task_id(601)).await.attempt_generation, 2);
}

#[tokio::test]
async fn uncertain_admission_waits_for_expiry_then_backoff() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(1_000, 4_000, 0),
    );
    admission.push(AdmissionDecision::Uncertain {
        reason: "admission response lost".to_string(),
    });

    store.create(immediate_record(701)).await.expect("create");
    scheduler.scan_once().await;
    let leased = record(&store, task_id(701)).await;
    assert_eq!(leased.state, TaskState::Leased);
    assert!(
        leased.retry_not_before.is_none(),
        "uncertain admission must not release or settle"
    );
    assert_eq!(scheduler.active_lease_count(), 0);

    // Lease expiry is the store-authority arbiter: recovery produces the
    // next attempt, paced by retry_not_before.
    clock.advance(60_000);
    scheduler.recover_once().await;
    let recovered = record(&store, task_id(701)).await;
    assert_eq!(recovered.state, TaskState::Ready);
    assert_eq!(
        recovered.retry_not_before,
        Some(DurableUtcTimestamp::from_millis(START_MILLIS + 61_000))
    );
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1, "no hot retry while backing off");
    clock.advance(1_000);
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 2);
    assert_eq!(record(&store, task_id(701)).await.attempt_generation, 2);
}

#[tokio::test]
async fn wake_fast_path_triggers_cycle_without_waiting_for_scan_interval() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let mut config = test_config("s1");
    config.scan_interval = Duration::from_secs(3_600);
    config.lease_duration = DurableDuration::from_millis(7_200_000);
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        config,
        test_backoff(100, 400, 10),
    );

    let runner = scheduler.clone();
    let handle = tokio::spawn(async move { runner.run().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    store
        .create(immediate_record(801))
        .await
        .expect("durable commit");
    scheduler.wake();

    admission.wait_for_calls(1, Duration::from_secs(2)).await;
    assert_eq!(
        admission.calls(),
        1,
        "wake must run a cycle far before the 1h scan interval"
    );
    assert_eq!(record(&store, task_id(801)).await.state, TaskState::Leased);

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn saturated_burst_drains_without_waiting_for_scan_interval() {
    const TASK_COUNT: u64 = 200;
    const BATCH_LIMIT: usize = 128;

    let store = Arc::new(MemoryTaskStore::new());
    let admission = Arc::new(FakeAdmission::new());
    let mut config = test_config("drain");
    config.batch_limit = BATCH_LIMIT;
    config.scan_interval = Duration::from_secs(3_600);
    config.lease_duration = DurableDuration::from_millis(7_200_000);
    config.min_cycle_interval = Duration::from_millis(10);
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        Arc::new(SystemClock),
        config,
        test_backoff(100, 400, 0),
    );

    let runner = scheduler.clone();
    let handle = tokio::spawn(async move { runner.run().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    for seed in 0..TASK_COUNT {
        store
            .create(immediate_record(seed))
            .await
            .expect("create burst task");
    }
    scheduler.wake();

    admission
        .wait_for_calls(TASK_COUNT as usize, Duration::from_secs(2))
        .await;
    assert_eq!(
        admission.calls(),
        TASK_COUNT as usize,
        "all burst tasks must be claimed without waiting for the scan interval"
    );
    let records = store.records().await;
    assert_eq!(
        records
            .iter()
            .filter(|record| record.state == TaskState::Leased)
            .count(),
        TASK_COUNT as usize,
        "every burst task must be leased"
    );

    handle.abort();
    let _ = handle.await;
}

#[tokio::test]
async fn scan_once_reports_saturation_until_burst_is_drained() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let mut config = test_config("saturation");
    config.batch_limit = 2;
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        config,
        test_backoff(100, 400, 0),
    );

    for seed in 0..3 {
        store
            .create(immediate_record(seed))
            .await
            .expect("create burst task");
    }

    assert!(
        scheduler.scan_once().await,
        "a full batch means more due work may remain"
    );
    assert_eq!(admission.calls(), 2);
    assert!(
        !scheduler.scan_once().await,
        "a short batch means the burst has been drained"
    );
    assert_eq!(admission.calls(), 3);
}

#[tokio::test]
async fn permanent_failure_converges_to_platform_failed_without_retry() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(100, 400, 7),
    );
    admission.push(AdmissionDecision::PermanentFailure {
        reason: "execution record permanently destroyed".to_string(),
    });

    store.create(immediate_record(901)).await.expect("create");
    scheduler.scan_once().await;
    assert_eq!(admission.calls(), 1);
    assert_eq!(scheduler.active_lease_count(), 0);

    let terminal = record(&store, task_id(901)).await;
    assert_eq!(terminal.state, TaskState::PlatformFailed);
    assert!(terminal.active_lease.is_none());
    assert!(matches!(
        terminal.terminal,
        Some(TaskTerminal {
            outcome: TaskOutcome::PlatformFailed { .. },
            ..
        })
    ));

    scheduler.scan_once().await;
    assert_eq!(
        admission.calls(),
        1,
        "platform-failed task must never be retried"
    );
    let status = store
        .status(StatusInput {
            task_id: task_id(901),
            retention: DurableDuration::from_millis(365 * 24 * 3600 * 1000),
        })
        .await
        .expect("status");
    assert_eq!(status.kind, TaskStatusKind::PlatformFailed);
}

#[tokio::test]
async fn backoff_upper_bound_and_jitter_apply_end_to_end() {
    let clock = Arc::new(FakeClock::new(START_MILLIS));
    let store = Arc::new(MemoryTaskStore::with_clock(clock.clone()));
    let admission = Arc::new(FakeAdmission::new());
    let scheduler = Scheduler::new(
        store.clone(),
        admission.clone(),
        clock.clone(),
        test_config("s1"),
        test_backoff(100, 400, 7),
    );

    store.create(immediate_record(951)).await.expect("create");
    scheduler.scan_once().await;
    assert_eq!(record(&store, task_id(951)).await.attempt_generation, 1);

    let expected_delays = [107i64, 207, 407, 407];
    let mut generation = 1u64;
    for expected in &expected_delays {
        clock.advance(60_000);
        scheduler.recover_once().await;
        let recovered = record(&store, task_id(951)).await;
        let retry = recovered
            .retry_not_before
            .expect("recovery applies retry not-before");
        assert_eq!(
            retry.millis() - clock.now_millis(),
            *expected,
            "generation {generation} backoff must be min(base*2^n, max) + jitter"
        );
        clock.advance(*expected);
        scheduler.scan_once().await;
        generation += 1;
        assert_eq!(
            record(&store, task_id(951)).await.attempt_generation,
            generation
        );
    }
    assert!(generation >= 4, "capped delay still produces new attempts");
    assert_eq!(
        expected_delays.last(),
        Some(&407),
        "backoff must cap at max + jitter bound"
    );
}

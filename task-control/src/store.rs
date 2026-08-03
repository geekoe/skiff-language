//! TaskStore port: durable create, conditional claim / renew / settlement /
//! cancel / lease-expiry recovery, due scan and status.
//!
//! Every transition is a conditional write against the current
//! state / lease; the Mongo adapter executes the same contracts with
//! server-authority CAS. The in-memory fake shares the pure reducer so the
//! contract tests in `tests/support/contract.rs` cover both adapters.

use async_trait::async_trait;

use crate::error::TaskStoreError;
use crate::model::{
    DurableDuration, DurableUtcTimestamp, LeaseId, TaskCancelResult, TaskId, TaskRecord,
    TaskStatus, TaskTerminal,
};

#[async_trait]
pub trait TaskStore: Send + Sync {
    /// Store authority UTC now. Scheduler timing decisions (lease expiry,
    /// retry not-before) must use this clock, never a local wall clock.
    async fn now(&self) -> Result<DurableUtcTimestamp, TaskStoreError>;

    /// Durable TaskId-idempotent create. Retrying the exact canonical record
    /// returns the existing record; the same TaskId with a different canonical
    /// record is rejected with `DuplicateTaskId`.
    async fn create(&self, record: TaskRecord) -> Result<TaskRecord, TaskStoreError>;

    /// Conditional claim: only `ready` + `due_at <= store now` +
    /// `image_activatable`. Atomically writes `leased`, a fresh
    /// AttemptId / lease id, the lease owner / expiry and a monotonic attempt
    /// generation bump.
    async fn claim(&self, input: ClaimInput) -> Result<ClaimOutcome, TaskStoreError>;

    /// Lease heartbeat. Only the current lease id may renew, and only while
    /// the lease is unexpired at store authority time.
    async fn renew(&self, input: RenewInput) -> Result<RenewOutcome, TaskStoreError>;

    /// Terminal settlement CAS. Accepts the exact same terminal write
    /// idempotently, rejects stale leases, expired leases and conflicting
    /// outcomes.
    async fn settle(&self, input: SettleInput) -> Result<SettleOutcome, TaskStoreError>;

    /// Before-start cancellation: scheduled / ready -> canceled;
    /// leased -> alreadyStarted; terminal -> alreadyTerminal;
    /// missing / retention-expired -> expired.
    async fn cancel(&self, input: CancelInput) -> Result<TaskCancelResult, TaskStoreError>;

    /// Lease-expiry recovery CAS: only `leased` with `lease_expiry <= store
    /// now`; atomically clears the lease and returns to `ready`.
    async fn recover_expired_lease(
        &self,
        input: LeaseRecoveryInput,
    ) -> Result<LeaseRecoveryOutcome, TaskStoreError>;

    /// Provable-rejection release CAS: only the current, unexpired lease may
    /// be returned to `ready`. The scheduler-owned retry not-before is set
    /// atomically (monotonic max) so concurrent replicas converge on the
    /// latest backoff. An expired lease loses to lease-expiry recovery.
    async fn release(&self, input: ReleaseInput) -> Result<ReleaseOutcome, TaskStoreError>;

    /// Due scan: advances `scheduled` tasks whose `due_at` has arrived to
    /// `ready` (store authority time) and returns due `ready` records ordered
    /// by `due_at`. Duplicate scans never create a second logical task.
    async fn scan_due(&self, input: DueScanInput) -> Result<Vec<TaskRecord>, TaskStoreError>;

    /// Scan for leased records whose lease has expired at store authority
    /// time, ordered by lease expiry (oldest first) and capped by `limit`.
    /// Feeds the scheduler's lease-expiry recovery loop across all replicas.
    async fn scan_expired_leases(
        &self,
        input: ScanExpiredLeasesInput,
    ) -> Result<Vec<TaskRecord>, TaskStoreError>;

    /// Status query. Missing records and records past their retention horizon
    /// return `expired`; terminal records never reopen.
    async fn status(&self, input: StatusInput) -> Result<TaskStatus, TaskStoreError>;

    /// Read-only backlog projection for observability: non-terminal record
    /// counts by state and the oldest `due_at` among scheduled / ready
    /// records, at store authority time. Never mutates state and never
    /// advances due visibility; duplicate scanners keep their own CAS.
    async fn observe_backlog(&self) -> Result<BacklogObservation, TaskStoreError>;

    /// Create / verify storage indexes (no-op for in-memory stores).
    async fn ensure_indexes(&self) -> Result<(), TaskStoreError>;

    /// Release storage resources; operations after close fail with `Closed`.
    async fn close(&self) -> Result<(), TaskStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimInput {
    pub task_id: TaskId,
    /// Scheduler / execution witness identity recorded in the lease.
    pub owner: String,
    /// Lease expiry chosen by the claimant; must be after store authority now.
    pub lease_expiry: DurableUtcTimestamp,
    /// Scheduler precondition: the frozen execution image can be reactivated.
    pub image_activatable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewInput {
    pub task_id: TaskId,
    pub lease_id: crate::model::LeaseId,
    pub new_expiry: DurableUtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettleInput {
    pub task_id: TaskId,
    pub lease_id: crate::model::LeaseId,
    pub terminal: TaskTerminal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelInput {
    pub task_id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseRecoveryInput {
    pub task_id: TaskId,
    /// Scheduler-owned pacing: no future attempt may be claimed before this
    /// durable store-authority timestamp. Recovery writes it atomically.
    pub retry_not_before: DurableUtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueScanInput {
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanExpiredLeasesInput {
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInput {
    pub task_id: TaskId,
    pub lease_id: LeaseId,
    pub retry_not_before: DurableUtcTimestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInput {
    pub task_id: TaskId,
    pub retention: DurableDuration,
}

/// One read-only backlog snapshot (authoritative design "Observability And
/// Retention": backlog depth, oldest eligible age, terminal age and store
/// authority observation time). Never mutates state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BacklogObservation {
    /// Records still in `scheduled` (future or due but not yet advanced).
    pub scheduled: usize,
    /// Records in `ready` (due and claimable).
    pub ready: usize,
    /// Records in `leased` (an active attempt owns them).
    pub leased: usize,
    /// Oldest `due_at` across scheduled + ready records.
    pub oldest_due_at: Option<DurableUtcTimestamp>,
    /// Terminal records still retained by the store (status/audit horizon).
    pub terminal_count: usize,
    /// Oldest `settled_at` across retained terminal records.
    pub oldest_terminal_at: Option<DurableUtcTimestamp>,
    /// Store-authority UTC time of this observation (terminal age / eligible
    /// age are derived against this timestamp, never a local wall clock).
    pub observed_at: Option<DurableUtcTimestamp>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClaimOutcome {
    Claimed(TaskRecord),
    Rejected(ClaimRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimRejection {
    /// State is scheduled (due scanner has not advanced it yet).
    NotReady,
    /// `due_at` is after store authority now.
    NotDue,
    /// Claimant precondition: execution image cannot be reactivated.
    NotActivatable,
    /// Already leased; a second valid lease is impossible.
    AlreadyLeased,
    /// Terminal tasks never reopen.
    Terminal,
    /// No record with this TaskId.
    NotFound,
    /// Lease expiry supplied by the claimant is not in the future.
    InvalidLeaseExpiry,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RenewOutcome {
    Renewed(TaskRecord),
    Rejected(RenewRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewRejection {
    /// Lease id does not match the current lease.
    StaleLease,
    /// Current lease already expired at store authority time.
    ExpiredLease,
    /// New expiry is not in the future at store authority time.
    InvalidExpiry,
    /// Not leased (scheduled / ready).
    NotLeased,
    /// Terminal tasks cannot be renewed.
    Terminal,
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettleOutcome {
    /// This settlement won the CAS and wrote the terminal.
    Settled(TaskRecord),
    /// Exact same terminal write replayed; idempotently accepted.
    AlreadySettled(TaskRecord),
    /// Same lease but a different terminal outcome already converged.
    Conflict(TaskRecord),
    StaleLease,
    ExpiredLease,
    NotLeased,
    NotFound,
}

/// Reducer-level settlement result; adapters add the current record context
/// when mapping to [`SettleOutcome`].
#[derive(Debug, Clone, PartialEq)]
pub enum SettleTransition {
    Settled(TaskRecord),
    AlreadySettled,
    Conflict,
    StaleLease,
    ExpiredLease,
    NotLeased,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LeaseRecoveryOutcome {
    /// Recovery won the CAS; lease cleared and task is `ready` again.
    Recovered(TaskRecord),
    /// Lease is still valid at store authority time.
    NotExpired,
    /// Not leased (scheduled / ready).
    NotLeased,
    Terminal,
    NotFound,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReleaseOutcome {
    /// Release won the CAS; lease cleared and task is `ready` with the
    /// scheduler-owned retry not-before applied.
    Released(TaskRecord),
    /// Lease id does not match the current lease.
    StaleLease,
    /// Lease already expired at store authority time; recovery owns it.
    ExpiredLease,
    /// Not leased (scheduled / ready).
    NotLeased,
    Terminal,
    NotFound,
}

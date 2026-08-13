use std::{
    fmt,
    num::NonZeroU64,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use serde_json::{Map, Value};
use skiff_runtime_capability_context::ExecutionBudgetReason;
use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmBudgetTerminal, VmSemanticCharge};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

pub const DEFAULT_INSTRUCTION_LIMIT: u64 = 10_000_000;
pub const DEFAULT_POLL_INTERVAL: u64 = 1024;

/// Trusted finite request policy. Artifact and wire data cannot construct it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionBudgetPolicy {
    hard_raw_limit: u64,
    raw_poll_interval: NonZeroU64,
}

impl ExecutionBudgetPolicy {
    pub const fn new(hard_raw_limit: u64, raw_poll_interval: NonZeroU64) -> Self {
        Self {
            hard_raw_limit,
            raw_poll_interval,
        }
    }

    pub const fn runtime_default() -> Self {
        Self::new(
            DEFAULT_INSTRUCTION_LIMIT,
            NonZeroU64::new(DEFAULT_POLL_INTERVAL).expect("default poll interval is non-zero"),
        )
    }

    pub const fn hard_raw_limit(self) -> u64 {
        self.hard_raw_limit
    }

    pub const fn raw_poll_interval(self) -> NonZeroU64 {
        self.raw_poll_interval
    }
}

/// Absolute monotonic request deadline admitted once at ingress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmittedRequestDeadline(Instant);

impl AdmittedRequestDeadline {
    pub const fn new(at: Instant) -> Self {
        Self(at)
    }

    pub const fn at(self) -> Instant {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestDeadlineAdmissionError {
    InvalidShape,
    InvalidTimeout,
    InvalidExpiry,
    Unrepresentable,
}

impl fmt::Display for RequestDeadlineAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidShape => "request deadline must be an object",
            Self::InvalidTimeout => "request deadline timeoutMs must be a non-negative integer",
            Self::InvalidExpiry => "request deadline expiresAt must be an RFC3339 timestamp",
            Self::Unrepresentable => "request deadline is outside the monotonic clock range",
        })
    }
}

impl std::error::Error for RequestDeadlineAdmissionError {}

/// Host-owned monotonic time source retained by the request budget.
pub trait TrustedMonotonicClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Debug, Default)]
pub struct SystemTrustedMonotonicClock;

impl TrustedMonotonicClock for SystemTrustedMonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecutionWinner {
    Succeeded,
    Failed,
    Cancelled,
    DeadlineExceeded,
    InstructionLimitExceeded,
    InternalStop,
    AccountingFailure,
}

impl ExecutionWinner {
    const fn vm_terminal(self) -> VmBudgetTerminal {
        match self {
            Self::Succeeded => VmBudgetTerminal::Succeeded,
            Self::Failed => VmBudgetTerminal::Failed,
            Self::Cancelled => VmBudgetTerminal::Cancelled,
            Self::DeadlineExceeded => VmBudgetTerminal::DeadlineExceeded,
            Self::InstructionLimitExceeded => VmBudgetTerminal::InstructionLimitExceeded,
            Self::InternalStop => VmBudgetTerminal::InternalStop,
            Self::AccountingFailure => VmBudgetTerminal::AccountingFailure,
        }
    }

    const fn budget_reason(self) -> Option<ExecutionBudgetReason> {
        match self {
            Self::Cancelled | Self::InternalStop => Some(ExecutionBudgetReason::Cancelled),
            Self::DeadlineExceeded => Some(ExecutionBudgetReason::DeadlineExceeded),
            Self::InstructionLimitExceeded => Some(ExecutionBudgetReason::InstructionLimitExceeded),
            Self::Succeeded | Self::Failed | Self::AccountingFailure => None,
        }
    }
}

/// Terminal sink registered by the request driver while one of its VM
/// continuations is parked on an actual pending operation.
///
/// The budget is the sole authority that decides a request terminal. When it
/// selects a winner while pending cells are parked, it notifies every sink
/// exactly once; the sink converts the winner to the VM resume terminal and
/// settles the pending cell so the suspended fiber can unwind. A sink must
/// never re-enter the budget, settle twice or emit a request observation.
pub trait RequestPendingSink: Send + Sync + 'static {
    /// Delivered exactly once when the budget selects a terminal winner.
    fn on_terminal(&self, winner: ExecutionWinner);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionCandidate {
    Success,
    Failure,
}

impl CompletionCandidate {
    const fn winner(self) -> ExecutionWinner {
        match self {
            Self::Success => ExecutionWinner::Succeeded,
            Self::Failure => ExecutionWinner::Failed,
        }
    }
}

#[derive(Debug)]
pub struct ExecutionSettlement {
    winner: ExecutionWinner,
    raw_executed_count: u64,
    semantic_charge_count: u64,
    hard_raw_limit: u64,
    poll_count: u64,
    started_at: Instant,
    finished_at: Instant,
}

impl ExecutionSettlement {
    pub const fn winner(&self) -> ExecutionWinner {
        self.winner
    }

    pub const fn raw_executed_count(&self) -> u64 {
        self.raw_executed_count
    }

    pub(crate) const fn semantic_charge_count(&self) -> u64 {
        self.semantic_charge_count
    }

    pub const fn hard_raw_limit(&self) -> u64 {
        self.hard_raw_limit
    }

    pub const fn poll_count(&self) -> u64 {
        self.poll_count
    }

    pub const fn started_at(&self) -> Instant {
        self.started_at
    }

    pub const fn finished_at(&self) -> Instant {
        self.finished_at
    }

    pub fn elapsed(&self) -> Duration {
        self.finished_at
            .checked_duration_since(self.started_at)
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug)]
pub enum SettlementDisposition {
    Won(Arc<ExecutionSettlement>),
    AlreadySettled(Arc<ExecutionSettlement>),
}

impl SettlementDisposition {
    pub fn settlement(&self) -> &Arc<ExecutionSettlement> {
        match self {
            Self::Won(settlement) | Self::AlreadySettled(settlement) => settlement,
        }
    }

    pub fn into_settlement(self) -> Arc<ExecutionSettlement> {
        match self {
            Self::Won(settlement) | Self::AlreadySettled(settlement) => settlement,
        }
    }

    pub const fn won(&self) -> bool {
        matches!(self, Self::Won(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmAdapterAttachError {
    AlreadyAttached,
    AlreadySettled,
}

impl fmt::Display for VmAdapterAttachError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyAttached => "request execution budget already has a VM adapter",
            Self::AlreadySettled => "request execution budget is already settled",
        })
    }
}

impl std::error::Error for VmAdapterAttachError {}

struct ExecutionBudgetState {
    raw_executed_count: u64,
    semantic_charge_count: u64,
    poll_count: u64,
    last_polled_raw_count: Option<u64>,
    vm_adapter_attached: bool,
    settlement: Option<Arc<ExecutionSettlement>>,
    pending_sinks: Vec<Arc<dyn RequestPendingSink>>,
}

pub struct ExecutionBudget {
    policy: ExecutionBudgetPolicy,
    deadline: Option<AdmittedRequestDeadline>,
    started_at: Instant,
    clock: Arc<dyn TrustedMonotonicClock>,
    state: Mutex<ExecutionBudgetState>,
}

impl fmt::Debug for ExecutionBudget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutionBudget")
            .field("policy", &self.policy)
            .field("deadline", &self.deadline)
            .field("started_at", &self.started_at)
            .finish_non_exhaustive()
    }
}

impl ExecutionBudget {
    pub fn new(
        policy: ExecutionBudgetPolicy,
        deadline: Option<AdmittedRequestDeadline>,
        clock: Arc<dyn TrustedMonotonicClock>,
    ) -> Self {
        let started_at = clock.now();
        Self {
            policy,
            deadline,
            started_at,
            clock,
            state: Mutex::new(ExecutionBudgetState {
                raw_executed_count: 0,
                semantic_charge_count: 0,
                poll_count: 0,
                last_polled_raw_count: None,
                vm_adapter_attached: false,
                settlement: None,
                pending_sinks: Vec::new(),
            }),
        }
    }

    pub fn for_runtime_request(deadline: Option<AdmittedRequestDeadline>) -> Self {
        Self::new(
            ExecutionBudgetPolicy::runtime_default(),
            deadline,
            Arc::new(SystemTrustedMonotonicClock),
        )
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.deadline.map(AdmittedRequestDeadline::at)
    }

    pub(crate) fn attach_vm(self: &Arc<Self>) -> Result<BytecodeVmBudget, VmAdapterAttachError> {
        let mut state = self.lock_state();
        if state.settlement.is_some() {
            return Err(VmAdapterAttachError::AlreadySettled);
        }
        if state.vm_adapter_attached {
            return Err(VmAdapterAttachError::AlreadyAttached);
        }
        state.vm_adapter_attached = true;
        drop(state);
        Ok(BytecodeVmBudget {
            execution_budget: Arc::clone(self),
        })
    }

    pub fn settle(&self, candidate: CompletionCandidate) -> SettlementDisposition {
        self.settle_requested(candidate.winner())
    }

    pub fn request_cancel(&self) -> SettlementDisposition {
        self.settle_requested(ExecutionWinner::Cancelled)
    }

    pub fn request_internal_stop(&self) -> SettlementDisposition {
        self.settle_requested(ExecutionWinner::InternalStop)
    }

    /// Registers a pending-cell sink and reports an already-selected winner.
    ///
    /// `None` means the budget is still open and the sink is now registered:
    /// the budget will notify it exactly once when a terminal winner is
    /// selected. `Some(winner)` means the budget had already frozen (or the
    /// deadline is already due); the caller must settle the parked cell with
    /// that winner inline, and the sink is not registered.
    pub fn register_pending_sink(
        &self,
        sink: Arc<dyn RequestPendingSink>,
    ) -> Option<ExecutionWinner> {
        let mut state = self.lock_state();
        if let Some(settlement) = &state.settlement {
            return Some(settlement.winner);
        }
        let now = self.clock.now();
        if self.deadline_is_due(now) {
            let settlement = self.freeze(&mut state, ExecutionWinner::DeadlineExceeded, now);
            let winner = settlement.winner;
            drop(state);
            self.complete_pending_sinks(winner);
            return Some(winner);
        }
        state.pending_sinks.push(sink);
        None
    }

    /// Authoritative terminal arbitration for a parked request completion.
    ///
    /// `None` means the request is still open and the completion may deliver
    /// its host value. `Some(winner)` means the request is already terminal or
    /// its deadline is due; the completion must be converted to that terminal
    /// instead. A due deadline freezes the budget as `DeadlineExceeded` so the
    /// same single winner is reported to every later racer.
    pub fn pending_terminal_winner(&self) -> Option<ExecutionWinner> {
        let mut state = self.lock_state();
        if let Some(settlement) = &state.settlement {
            return Some(settlement.winner);
        }
        let now = self.clock.now();
        if self.deadline_is_due(now) {
            let settlement = self.freeze(&mut state, ExecutionWinner::DeadlineExceeded, now);
            let winner = settlement.winner;
            drop(state);
            self.complete_pending_sinks(winner);
            return Some(winner);
        }
        None
    }

    pub fn settlement(&self) -> Option<Arc<ExecutionSettlement>> {
        self.lock_state().settlement.clone()
    }

    pub fn stats_snapshot(&self) -> ExecutionStats {
        let state = self.lock_state();
        let now = state
            .settlement
            .as_ref()
            .map_or_else(|| self.clock.now(), |settlement| settlement.finished_at);
        let winner = state
            .settlement
            .as_ref()
            .map(|settlement| settlement.winner);
        ExecutionStats {
            instruction_count: state.raw_executed_count,
            budget_limit: Some(self.policy.hard_raw_limit),
            poll_count: state.poll_count,
            elapsed_ms: now
                .checked_duration_since(self.started_at)
                .unwrap_or_default()
                .as_secs_f64()
                * 1000.0,
            budget_exceeded: matches!(
                winner,
                Some(ExecutionWinner::DeadlineExceeded | ExecutionWinner::InstructionLimitExceeded)
            ),
            budget_reason: winner.and_then(ExecutionWinner::budget_reason),
        }
    }

    fn settle_requested(&self, requested: ExecutionWinner) -> SettlementDisposition {
        let (disposition, winner) = {
            let mut state = self.lock_state();
            if let Some(settlement) = &state.settlement {
                return SettlementDisposition::AlreadySettled(Arc::clone(settlement));
            }
            let now = self.clock.now();
            let winner = if self.deadline_is_due(now) {
                ExecutionWinner::DeadlineExceeded
            } else {
                requested
            };
            let settlement = self.freeze(&mut state, winner, now);
            (SettlementDisposition::Won(settlement), winner)
        };
        self.complete_pending_sinks(winner);
        disposition
    }

    fn complete_pending_sinks(&self, winner: ExecutionWinner) {
        let sinks = {
            let mut state = self.lock_state();
            std::mem::take(&mut state.pending_sinks)
        };
        for sink in sinks {
            sink.on_terminal(winner);
        }
    }

    fn before_dispatch(&self) -> Result<(), VmBudgetClosed> {
        let mut state = self.lock_state();
        if let Some(settlement) = &state.settlement {
            return Err(Self::already_closed(settlement));
        }
        if state.raw_executed_count >= self.policy.hard_raw_limit {
            let now = self.poll_locked(&mut state)?;
            let settlement =
                self.freeze(&mut state, ExecutionWinner::InstructionLimitExceeded, now);
            debug_assert_eq!(settlement.raw_executed_count, self.policy.hard_raw_limit);
            return Err(VmBudgetClosed::InstructionLimitExceeded);
        }
        let raw = state.raw_executed_count;
        if raw % self.policy.raw_poll_interval.get() == 0
            && state.last_polled_raw_count != Some(raw)
        {
            let _ = self.poll_locked(&mut state)?;
        }
        let Some(next_raw) = state.raw_executed_count.checked_add(1) else {
            return Err(self.freeze_accounting_after_deadline_check(&mut state));
        };
        state.raw_executed_count = next_raw;
        Ok(())
    }

    fn poll_interrupt(&self) -> Result<(), VmBudgetClosed> {
        let mut state = self.lock_state();
        self.poll_locked(&mut state).map(|_| ())
    }

    fn charge_semantic(&self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
        self.charge_semantic_unit()
    }

    fn charge_semantic_unit(&self) -> Result<(), VmBudgetClosed> {
        let mut state = self.lock_state();
        if let Some(settlement) = &state.settlement {
            return Err(Self::already_closed(settlement));
        }
        let Some(next_semantic) = state.semantic_charge_count.checked_add(1) else {
            return Err(self.freeze_accounting_after_deadline_check(&mut state));
        };
        state.semantic_charge_count = next_semantic;
        Ok(())
    }

    fn poll_locked(&self, state: &mut ExecutionBudgetState) -> Result<Instant, VmBudgetClosed> {
        if let Some(settlement) = &state.settlement {
            return Err(Self::already_closed(settlement));
        }
        let now = self.clock.now();
        let next_poll = state.poll_count.checked_add(1);
        if self.deadline_is_due(now) {
            if let Some(next_poll) = next_poll {
                state.poll_count = next_poll;
                state.last_polled_raw_count = Some(state.raw_executed_count);
            }
            self.freeze(state, ExecutionWinner::DeadlineExceeded, now);
            return Err(VmBudgetClosed::DeadlineExceeded);
        }
        let Some(next_poll) = next_poll else {
            self.freeze(state, ExecutionWinner::AccountingFailure, now);
            return Err(VmBudgetClosed::AccountingFailure);
        };
        state.poll_count = next_poll;
        state.last_polled_raw_count = Some(state.raw_executed_count);
        Ok(now)
    }

    fn freeze_accounting_after_deadline_check(
        &self,
        state: &mut ExecutionBudgetState,
    ) -> VmBudgetClosed {
        let now = self.clock.now();
        if self.deadline_is_due(now) {
            self.freeze(state, ExecutionWinner::DeadlineExceeded, now);
            VmBudgetClosed::DeadlineExceeded
        } else {
            self.freeze(state, ExecutionWinner::AccountingFailure, now);
            VmBudgetClosed::AccountingFailure
        }
    }

    fn freeze(
        &self,
        state: &mut ExecutionBudgetState,
        winner: ExecutionWinner,
        finished_at: Instant,
    ) -> Arc<ExecutionSettlement> {
        debug_assert!(state.settlement.is_none());
        let settlement = Arc::new(ExecutionSettlement {
            winner,
            raw_executed_count: state.raw_executed_count,
            semantic_charge_count: state.semantic_charge_count,
            hard_raw_limit: self.policy.hard_raw_limit,
            poll_count: state.poll_count,
            started_at: self.started_at,
            finished_at,
        });
        state.settlement = Some(Arc::clone(&settlement));
        settlement
    }

    fn deadline_is_due(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline.at())
    }

    fn already_closed(settlement: &ExecutionSettlement) -> VmBudgetClosed {
        VmBudgetClosed::AlreadySettled(settlement.winner.vm_terminal())
    }

    fn lock_state(&self) -> MutexGuard<'_, ExecutionBudgetState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The one non-cloneable VM adapter attached by the canonical request start.
pub(crate) struct BytecodeVmBudget {
    execution_budget: Arc<ExecutionBudget>,
}

impl VmBudget for BytecodeVmBudget {
    fn before_dispatch(&mut self) -> Result<(), VmBudgetClosed> {
        self.execution_budget.before_dispatch()
    }

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetClosed> {
        self.execution_budget.poll_interrupt()
    }

    fn charge_semantic(&mut self, charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetClosed> {
        self.execution_budget.charge_semantic(charge)
    }
}

#[derive(Clone, Debug)]
pub struct ExecutionStats {
    pub instruction_count: u64,
    pub budget_limit: Option<u64>,
    pub poll_count: u64,
    pub elapsed_ms: f64,
    pub budget_exceeded: bool,
    pub budget_reason: Option<ExecutionBudgetReason>,
}

pub fn admit_request_deadline(
    extra: &Map<String, Value>,
) -> Result<Option<AdmittedRequestDeadline>, RequestDeadlineAdmissionError> {
    let Some(value) = extra.get("deadline") else {
        return Ok(None);
    };
    let deadline = value
        .as_object()
        .ok_or(RequestDeadlineAdmissionError::InvalidShape)?;
    let monotonic_now = Instant::now();
    let mut candidates = Vec::with_capacity(2);

    if let Some(value) = deadline.get("timeoutMs") {
        let timeout_ms = value
            .as_u64()
            .ok_or(RequestDeadlineAdmissionError::InvalidTimeout)?;
        let timeout_ms = i64::try_from(timeout_ms)
            .map_err(|_| RequestDeadlineAdmissionError::Unrepresentable)?;
        candidates.push(
            monotonic_now
                .checked_add(Duration::from_millis(timeout_ms as u64))
                .ok_or(RequestDeadlineAdmissionError::Unrepresentable)?,
        );
    }

    if let Some(value) = deadline.get("expiresAt") {
        let expires_at = value
            .as_str()
            .ok_or(RequestDeadlineAdmissionError::InvalidExpiry)
            .and_then(|value| {
                OffsetDateTime::parse(value, &Rfc3339)
                    .map_err(|_| RequestDeadlineAdmissionError::InvalidExpiry)
            })?;
        let wall_now = OffsetDateTime::now_utc();
        if expires_at <= wall_now {
            candidates.push(monotonic_now);
        } else {
            candidates.push(
                monotonic_now
                    .checked_add((expires_at - wall_now).unsigned_abs())
                    .ok_or(RequestDeadlineAdmissionError::Unrepresentable)?,
            );
        }
    }

    if candidates.is_empty() {
        return Err(RequestDeadlineAdmissionError::InvalidShape);
    }
    Ok(candidates
        .into_iter()
        .min()
        .map(AdmittedRequestDeadline::new))
}

#[cfg(test)]
mod tests;

use std::{
    num::NonZeroU64,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde_json::Map;
use skiff_runtime_vm::{VmBudget, VmBudgetClosed, VmBudgetTerminal};

use super::{
    admit_request_deadline, AdmittedRequestDeadline, CompletionCandidate, ExecutionBudget,
    ExecutionBudgetPolicy, ExecutionWinner, SettlementDisposition, TrustedMonotonicClock,
    VmAdapterAttachError,
};
use crate::{cancellation::CancellationToken, ExecutionControl};

#[derive(Debug)]
struct FakeClock(Mutex<Instant>);

impl FakeClock {
    fn new(now: Instant) -> Self {
        Self(Mutex::new(now))
    }

    fn set(&self, now: Instant) {
        *self.0.lock().unwrap() = now;
    }
}

impl TrustedMonotonicClock for FakeClock {
    fn now(&self) -> Instant {
        *self.0.lock().unwrap()
    }
}

#[derive(Debug)]
struct ClockGate {
    block_next: bool,
    entered: bool,
    released: bool,
}

#[derive(Debug)]
struct BlockingClock {
    now: Mutex<Instant>,
    gate: Mutex<ClockGate>,
    changed: Condvar,
}

impl BlockingClock {
    fn new(now: Instant) -> Self {
        Self {
            now: Mutex::new(now),
            gate: Mutex::new(ClockGate {
                block_next: false,
                entered: false,
                released: false,
            }),
            changed: Condvar::new(),
        }
    }

    fn block_next_sample(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.block_next = true;
        gate.entered = false;
        gate.released = false;
    }

    fn wait_until_sample_entered(&self) {
        let mut gate = self.gate.lock().unwrap();
        while !gate.entered {
            gate = self.changed.wait(gate).unwrap();
        }
    }

    fn release_sample(&self) {
        let mut gate = self.gate.lock().unwrap();
        gate.released = true;
        self.changed.notify_all();
    }
}

impl TrustedMonotonicClock for BlockingClock {
    fn now(&self) -> Instant {
        let now = *self.now.lock().unwrap();
        let mut gate = self.gate.lock().unwrap();
        if gate.block_next {
            gate.block_next = false;
            gate.entered = true;
            self.changed.notify_all();
            while !gate.released {
                gate = self.changed.wait(gate).unwrap();
            }
        }
        now
    }
}

fn policy(limit: u64, poll_interval: u64) -> ExecutionBudgetPolicy {
    ExecutionBudgetPolicy::new(limit, NonZeroU64::new(poll_interval).unwrap())
}

fn budget(
    limit: u64,
    interval: u64,
    deadline: Option<Instant>,
    clock: Arc<dyn TrustedMonotonicClock>,
) -> Arc<ExecutionBudget> {
    Arc::new(ExecutionBudget::new(
        policy(limit, interval),
        deadline.map(AdmittedRequestDeadline::new),
        clock,
    ))
}

#[test]
fn exact_limit_counts_attempts_and_fails_only_on_n_plus_one() {
    let now = Instant::now();
    let budget = budget(2, 1024, None, Arc::new(FakeClock::new(now)));
    let mut adapter = budget.attach_vm().unwrap();

    assert_eq!(adapter.before_dispatch(), Ok(()));
    assert_eq!(adapter.before_dispatch(), Ok(()));
    assert_eq!(budget.stats_snapshot().instruction_count, 2);
    assert_eq!(
        adapter.before_dispatch(),
        Err(VmBudgetClosed::InstructionLimitExceeded)
    );

    let settlement = budget.settlement().expect("fuel freezes settlement");
    assert_eq!(
        settlement.winner(),
        ExecutionWinner::InstructionLimitExceeded
    );
    assert_eq!(settlement.raw_executed_count(), 2);
    assert_eq!(settlement.hard_raw_limit(), 2);
}

#[test]
fn exactly_one_non_clone_vm_adapter_can_attach_while_open() {
    let budget = budget(2, 1024, None, Arc::new(FakeClock::new(Instant::now())));
    let adapter = budget.attach_vm().unwrap();
    assert!(matches!(
        budget.attach_vm(),
        Err(VmAdapterAttachError::AlreadyAttached)
    ));
    drop(adapter);
    assert!(matches!(
        budget.attach_vm(),
        Err(VmAdapterAttachError::AlreadyAttached)
    ));
}

#[test]
fn zero_limit_rejects_the_first_dispatch_without_counting_it() {
    let budget = budget(0, 1024, None, Arc::new(FakeClock::new(Instant::now())));
    let mut adapter = budget.attach_vm().unwrap();

    assert_eq!(
        adapter.before_dispatch(),
        Err(VmBudgetClosed::InstructionLimitExceeded)
    );
    assert_eq!(budget.stats_snapshot().instruction_count, 0);
}

#[test]
fn max_limit_advances_max_minus_one_to_max_then_fails_without_overflow() {
    let budget = budget(
        u64::MAX,
        u64::MAX,
        None,
        Arc::new(FakeClock::new(Instant::now())),
    );
    let mut adapter = budget.attach_vm().unwrap();
    {
        let mut state = budget.lock_state();
        state.raw_executed_count = u64::MAX - 1;
    }

    assert_eq!(adapter.before_dispatch(), Ok(()));
    assert_eq!(budget.stats_snapshot().instruction_count, u64::MAX);
    assert_eq!(
        adapter.before_dispatch(),
        Err(VmBudgetClosed::InstructionLimitExceeded)
    );
    assert_eq!(budget.stats_snapshot().instruction_count, u64::MAX);
}

#[test]
fn explicit_poll_covers_the_same_raw_cadence_coordinate_once() {
    let budget = budget(10, 2, None, Arc::new(FakeClock::new(Instant::now())));
    let mut adapter = budget.attach_vm().unwrap();

    adapter.poll_interrupt().unwrap();
    assert_eq!(budget.stats_snapshot().poll_count, 1);
    adapter.before_dispatch().unwrap();
    assert_eq!(budget.stats_snapshot().poll_count, 1);
    adapter.before_dispatch().unwrap();
    adapter.before_dispatch().unwrap();
    assert_eq!(budget.stats_snapshot().poll_count, 2);
}

#[test]
fn semantic_and_poll_overflow_fail_closed_without_mutating_raw_count() {
    let semantic = budget(10, 2, None, Arc::new(FakeClock::new(Instant::now())));
    {
        let mut state = semantic.lock_state();
        state.semantic_charge_count = u64::MAX;
    }
    assert_eq!(
        semantic.charge_semantic_unit(),
        Err(VmBudgetClosed::AccountingFailure)
    );
    assert_eq!(semantic.stats_snapshot().instruction_count, 0);
    assert_eq!(
        semantic.settlement().unwrap().winner(),
        ExecutionWinner::AccountingFailure
    );

    let polling = budget(10, 2, None, Arc::new(FakeClock::new(Instant::now())));
    let mut adapter = polling.attach_vm().unwrap();
    {
        let mut state = polling.lock_state();
        state.poll_count = u64::MAX;
    }
    assert_eq!(
        adapter.poll_interrupt(),
        Err(VmBudgetClosed::AccountingFailure)
    );
    assert_eq!(polling.stats_snapshot().poll_count, u64::MAX);
}

#[test]
fn due_deadline_wins_every_open_transition_but_cannot_replace_a_frozen_winner() {
    let start = Instant::now();
    let deadline = start + Duration::from_millis(10);
    let clock = Arc::new(FakeClock::new(start));
    let deadline_budget = budget(10, 2, Some(deadline), clock.clone());

    clock.set(deadline + Duration::from_millis(1));
    let cancelled = deadline_budget.request_cancel().into_settlement();
    assert_eq!(cancelled.winner(), ExecutionWinner::DeadlineExceeded);

    clock.set(deadline + Duration::from_secs(1));
    let later = deadline_budget
        .settle(CompletionCandidate::Success)
        .into_settlement();
    assert!(Arc::ptr_eq(&cancelled, &later));
    assert_eq!(later.winner(), ExecutionWinner::DeadlineExceeded);

    let early_clock = Arc::new(FakeClock::new(start));
    let early = budget(10, 2, Some(deadline), early_clock.clone());
    let success = early.settle(CompletionCandidate::Success).into_settlement();
    early_clock.set(deadline + Duration::from_millis(1));
    assert_eq!(
        early.request_internal_stop().into_settlement().winner(),
        ExecutionWinner::Succeeded
    );
    assert_eq!(success.winner(), ExecutionWinner::Succeeded);
}

#[test]
fn stop_holds_the_budget_lock_while_sampling_and_rejects_a_racing_dispatch() {
    let clock = Arc::new(BlockingClock::new(Instant::now()));
    let budget = budget(10, 2, None, clock.clone());
    let mut adapter = budget.attach_vm().unwrap();
    clock.block_next_sample();

    let stopping_budget = budget.clone();
    let stopping = thread::spawn(move || stopping_budget.request_internal_stop());
    clock.wait_until_sample_entered();
    let dispatching = thread::spawn(move || adapter.before_dispatch());
    clock.release_sample();

    assert!(matches!(
        stopping.join().unwrap(),
        SettlementDisposition::Won(_)
    ));
    assert_eq!(
        dispatching.join().unwrap(),
        Err(VmBudgetClosed::AlreadySettled(
            VmBudgetTerminal::InternalStop
        ))
    );
    assert_eq!(budget.stats_snapshot().instruction_count, 0);
}

#[test]
fn request_execution_control_forwards_the_admitted_deadline() {
    let deadline = Instant::now() + Duration::from_secs(30);
    let budget = budget(
        100,
        4,
        Some(deadline),
        Arc::new(FakeClock::new(Instant::now())),
    );
    let control = ExecutionControl::new(CancellationToken::new(), &budget);

    assert_eq!(control.deadline(), Some(deadline));
    assert_eq!(control.owned().deadline(), Some(deadline));
    assert_eq!(control.owned().borrow().deadline(), Some(deadline));
}

#[test]
fn unrepresentable_timeout_is_rejected_instead_of_disabling_the_deadline() {
    let mut extra = Map::new();
    extra.insert(
        "deadline".to_string(),
        serde_json::json!({ "timeoutMs": u64::MAX }),
    );

    assert!(admit_request_deadline(&extra).is_err());
}

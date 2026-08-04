use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationSource, ExecutionBudgetReason, ExecutionControlError, ExecutionDeadlineSource,
    ExecutionScopeTerminal,
};

use super::ExecutionControl;
use crate::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

fn site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn budget(deadline: Option<Instant>) -> Arc<ExecutionBudget> {
    Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(100),
            poll_interval: 1,
        },
        deadline,
    ))
}

#[test]
fn add_instruction_units_defers_full_check_until_interval_crossing_or_limit() {
    let now = Instant::now();
    let request_budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(10_000),
            poll_interval: 1024,
        },
        Some(now + Duration::from_secs(60)),
    ));
    let control = ExecutionControl::new(CancellationSource::new().token(), &request_budget);

    for _ in 0..1023 {
        control
            .add_instruction_units(1)
            .expect("accounting below the poll interval must not fail");
    }
    assert_eq!(
        request_budget.stats_snapshot().poll_count,
        0,
        "per-node accounting must not poll the budget until an interval crossing"
    );

    control
        .add_instruction_units(1)
        .expect("crossing the poll interval performs a full check that still passes");
    assert_eq!(
        request_budget.stats_snapshot().poll_count,
        1,
        "the interval-crossing unit must run the full check"
    );

    let limited_budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(5),
            poll_interval: 1024,
        },
        None,
    ));
    let limited = ExecutionControl::new(CancellationSource::new().token(), &limited_budget);
    for _ in 0..4 {
        limited
            .add_instruction_units(1)
            .expect("accounting below the instruction limit must not fail");
    }
    let error = limited
        .add_instruction_units(1)
        .expect_err("reaching the instruction limit must run the full check");
    assert!(matches!(
        error,
        ExecutionControlError::BudgetExceeded(failure)
            if failure.reason == ExecutionBudgetReason::InstructionLimitExceeded
    ));
    assert_eq!(limited_budget.stats_snapshot().poll_count, 1);
}

#[test]
fn add_instruction_units_observes_cancel_on_interval_crossing() {
    let request_budget = budget(None);
    let ancestor = CancellationSource::new();
    let control = ExecutionControl::new(ancestor.token(), &request_budget);
    ancestor.cancel();

    assert_eq!(
        control.add_instruction_units(1),
        Err(ExecutionControlError::Cancelled)
    );
}

#[test]
fn derived_control_shares_instruction_and_poll_accounting_but_not_local_failure_telemetry() {
    let now = Instant::now();
    let request_budget = budget(None);
    let parent_cancel = CancellationSource::new();
    let parent = ExecutionControl::new(parent_cancel.token(), &request_budget);
    let child = parent
        .derive_scope(
            now + Duration::from_millis(10),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derived scope");

    child
        .borrow()
        .add_instruction_units_at(1, now)
        .expect("child instruction");
    parent
        .add_instruction_units_at(1, now)
        .expect("parent instruction");
    let before_timeout = request_budget.stats_snapshot();
    assert_eq!(before_timeout.instruction_count, 2);
    assert_eq!(before_timeout.poll_count, 2);

    let timeout = child
        .borrow()
        .poll_execution_budget_at(now + Duration::from_millis(10))
        .expect_err("local deadline should terminate child scope");
    assert!(matches!(
        timeout,
        ExecutionControlError::BudgetExceeded(failure)
            if failure.reason == ExecutionBudgetReason::DeadlineExceeded
    ));
    let after_timeout = request_budget.stats_snapshot();
    assert_eq!(after_timeout.poll_count, 3, "poll accounting is shared");
    assert_eq!(
        after_timeout.budget_reason, None,
        "local timeout must not record request-wide first-failure telemetry"
    );
    child
        .borrow()
        .check_cancelled()
        .expect("a local deadline is not ancestor cancellation");
    assert!(
        !parent_cancel.is_cancelled(),
        "local timeout cannot cancel the request token"
    );
    parent
        .poll_execution_budget_at(now + Duration::from_millis(11))
        .expect("parent execution continues after child timeout is caught");
}

#[test]
fn request_and_outer_deadlines_bound_nested_scopes_with_stable_source_and_nesting() {
    let now = Instant::now();
    let request_deadline = now + Duration::from_millis(30);
    let request_budget = budget(Some(request_deadline));
    let root = ExecutionControl::new(CancellationSource::new().token(), &request_budget);
    let request_bounded = root
        .derive_scope(
            now + Duration::from_millis(60),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("request-bounded scope");
    let effective = request_bounded
        .effective_deadline()
        .expect("request deadline");
    assert_eq!(effective.at(), request_deadline);
    assert_eq!(effective.source(), &ExecutionDeadlineSource::Request);
    assert_eq!(effective.nesting(), 0);
    assert!(matches!(
        request_bounded
            .borrow()
            .poll_execution_budget_at(request_deadline),
        Err(ExecutionControlError::BudgetExceeded(failure))
            if failure.reason == ExecutionBudgetReason::DeadlineExceeded
    ));
    assert_eq!(
        request_budget.stats_snapshot().budget_reason,
        Some(ExecutionBudgetReason::DeadlineExceeded),
        "the request owner, unlike a local scope, records request deadline telemetry"
    );

    let no_request_budget = budget(None);
    let parent = ExecutionControl::new(CancellationSource::new().token(), &no_request_budget);
    let outer = parent
        .derive_scope(
            now + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("outer scope");
    let inner = outer
        .derive_scope(
            now + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("tied inner scope");
    assert_eq!(inner.scope_nesting(), 2);
    assert_eq!(
        inner.effective_deadline(),
        outer.effective_deadline(),
        "same absolute deadline keeps the outer owner"
    );
    assert!(matches!(
        inner.scope_terminal_at(now + Duration::from_millis(20)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    assert!(matches!(
        outer.scope_terminal_at(now + Duration::from_millis(20)),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
}

#[test]
fn ancestor_cancel_is_terminal_and_wins_when_deadline_is_also_ready() {
    let now = Instant::now();
    let request_budget = budget(None);
    let ancestor = CancellationSource::new();
    let root = ExecutionControl::new(ancestor.token(), &request_budget);
    let child = root
        .derive_scope(
            now + Duration::from_millis(10),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("child scope");

    ancestor.cancel();
    assert_eq!(
        child
            .borrow()
            .poll_execution_budget_at(now + Duration::from_millis(10)),
        Err(ExecutionControlError::Cancelled)
    );
    assert_eq!(
        child.scope_terminal_at(now + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert_eq!(
        request_budget.stats_snapshot().budget_reason,
        Some(ExecutionBudgetReason::Cancelled)
    );
}

#[test]
fn dropping_or_finishing_a_child_restores_the_unchanged_parent_control() {
    let now = Instant::now();
    let request_budget = budget(None);
    let parent_cancel = CancellationSource::new();
    let parent = ExecutionControl::new(parent_cancel.token(), &request_budget);

    {
        let child = parent
            .derive_scope(
                now + Duration::from_millis(10),
                site(SyntheticInstructionSiteReason::RuntimeControlFlow),
            )
            .expect("child scope");
        assert_eq!(child.scope_nesting(), 1);
    }

    assert_eq!(parent.scope_nesting(), 0);
    assert_eq!(parent.effective_deadline(), None);
    assert!(!parent_cancel.is_cancelled());
    parent
        .poll_execution_budget_at(now + Duration::from_secs(1))
        .expect("dropping child leaves parent usable");
}

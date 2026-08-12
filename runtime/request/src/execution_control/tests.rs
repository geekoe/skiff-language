use std::{
    num::NonZeroU64,
    sync::Arc,
    time::{Duration, Instant},
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationSource, ExecutionBudgetReason, ExecutionControlError, ExecutionDeadlineSource,
    ExecutionScopeTerminal,
};

use super::ExecutionControl;
use crate::execution_budget::{
    AdmittedRequestDeadline, CompletionCandidate, ExecutionBudget, ExecutionBudgetPolicy,
    SystemTrustedMonotonicClock,
};

fn site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn budget(deadline: Option<Instant>) -> Arc<ExecutionBudget> {
    Arc::new(ExecutionBudget::new(
        ExecutionBudgetPolicy::new(100, NonZeroU64::new(1).unwrap()),
        deadline.map(AdmittedRequestDeadline::new),
        Arc::new(SystemTrustedMonotonicClock),
    ))
}

#[test]
fn control_does_not_create_a_second_raw_accounting_path() {
    let request_budget = budget(None);
    let control = ExecutionControl::new(CancellationSource::new().token(), &request_budget);

    control.poll_execution_budget().unwrap();
    assert_eq!(request_budget.stats_snapshot().instruction_count, 0);
    assert_eq!(request_budget.stats_snapshot().poll_count, 0);
}

#[test]
fn local_deadline_is_scoped_and_does_not_freeze_the_request_budget() {
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

    let timeout = child
        .borrow()
        .poll_execution_budget_at(now + Duration::from_millis(10))
        .expect_err("local deadline should terminate the child scope");
    assert!(matches!(
        timeout,
        ExecutionControlError::BudgetExceeded(failure)
            if failure.reason == ExecutionBudgetReason::DeadlineExceeded
    ));
    assert!(request_budget.settlement().is_none());
    assert!(!parent_cancel.is_cancelled());
    parent.poll_execution_budget().unwrap();
}

#[test]
fn request_and_outer_deadlines_keep_stable_scope_source_and_nesting() {
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
    let effective = request_bounded.effective_deadline().unwrap();
    assert_eq!(effective.at(), request_deadline);
    assert_eq!(effective.source(), &ExecutionDeadlineSource::Request);
    assert_eq!(effective.nesting(), 0);

    let no_request_budget = budget(None);
    let parent = ExecutionControl::new(CancellationSource::new().token(), &no_request_budget);
    let outer = parent
        .derive_scope(
            now + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .unwrap();
    let inner = outer
        .derive_scope(
            now + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .unwrap();
    assert_eq!(inner.scope_nesting(), 2);
    assert_eq!(inner.effective_deadline(), outer.effective_deadline());
    assert!(matches!(
        inner.scope_terminal_at(now + Duration::from_millis(20)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
}

#[test]
fn ancestor_cancel_is_wake_only_and_request_winner_remains_budget_owned() {
    let request_budget = budget(None);
    let ancestor = CancellationSource::new();
    let control = ExecutionControl::new(ancestor.token(), &request_budget);
    ancestor.cancel();

    assert_eq!(
        control.poll_execution_budget(),
        Err(ExecutionControlError::Cancelled)
    );
    assert!(request_budget.settlement().is_none());

    let frozen = request_budget
        .settle(CompletionCandidate::Success)
        .into_settlement();
    assert_eq!(
        frozen.winner(),
        crate::execution_budget::ExecutionWinner::Succeeded
    );
}

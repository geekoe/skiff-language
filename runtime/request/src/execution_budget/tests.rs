use std::time::{Duration, Instant};

use serde_json::Map;
use skiff_runtime_capability_context::ExecutionBudgetReason;

use super::{deadline_from_request_extra, ExecutionBudget, ExecutionBudgetConfig};

#[test]
fn disabled_budget_does_not_count_or_poll_limits() {
    let budget = ExecutionBudget::disabled();

    assert!(!budget.add_units(10_000));
    assert!(budget.poll(false, Instant::now()).is_ok());

    let stats = budget.stats_snapshot();
    assert_eq!(stats.instruction_count, 0);
    assert_eq!(stats.budget_reason, None);
}

#[test]
fn instruction_limit_fails_on_poll() {
    let budget = ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(3),
            poll_interval: 1024,
        },
        None,
    );

    assert!(budget.add_units(3));
    assert_eq!(
        budget.poll(false, Instant::now()),
        Err(ExecutionBudgetReason::InstructionLimitExceeded)
    );
    assert_eq!(
        budget.stats_snapshot().budget_reason,
        Some(ExecutionBudgetReason::InstructionLimitExceeded)
    );
}

#[test]
fn expired_deadline_fails_on_poll() {
    let budget = ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(1_000),
            poll_interval: 1024,
        },
        Some(Instant::now() - Duration::from_millis(1)),
    );

    assert_eq!(
        budget.poll(false, Instant::now()),
        Err(ExecutionBudgetReason::DeadlineExceeded)
    );
}

#[test]
fn cancel_takes_priority_over_deadline() {
    let budget = ExecutionBudget::new(
        ExecutionBudgetConfig {
            enabled: true,
            instruction_limit: Some(1_000),
            poll_interval: 1024,
        },
        Some(Instant::now() - Duration::from_millis(1)),
    );

    assert_eq!(
        budget.poll(true, Instant::now()),
        Err(ExecutionBudgetReason::Cancelled)
    );
    assert_eq!(
        budget.stats_snapshot().budget_reason,
        Some(ExecutionBudgetReason::Cancelled)
    );
}

#[test]
fn huge_timeout_deadline_does_not_panic() {
    let mut extra = Map::new();
    extra.insert(
        "deadline".to_string(),
        serde_json::json!({ "timeoutMs": u64::MAX }),
    );

    let _deadline = deadline_from_request_extra(&extra);
}

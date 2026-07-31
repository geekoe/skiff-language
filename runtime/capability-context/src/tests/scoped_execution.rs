use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};

use crate::{
    CancellationSource, ExecutionDeadlineSource, ExecutionScope, ExecutionScopeAccessError,
    ExecutionScopeLeaseTerminal, ExecutionScopeTerminal,
};

fn site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn install_current_scope_contract(
    control: &crate::ExecutionControl<'_>,
    deadline: std::time::Instant,
    site: InstructionSourceSite,
) -> Result<crate::OwnedExecutionControl, ExecutionScopeAccessError> {
    control.derive_scope(deadline, site)
}

fn read_invocation_scope_contract(
    control: &crate::ExecutionControl<'_>,
) -> Result<ExecutionScope, ExecutionScopeAccessError> {
    control.execution_scope()
}

#[test]
fn execution_control_facade_freezes_current_scope_consumer_contracts() {
    let _ = install_current_scope_contract;
    let _ = read_invocation_scope_contract;

    let adapter = UnscopedAdapter;
    assert!(matches!(
        crate::ExecutionControlApi::execution_scope(&adapter),
        Err(ExecutionScopeAccessError::Unavailable)
    ));
    assert!(matches!(
        crate::ExecutionControlApi::derive_scope(
            &adapter,
            std::time::Instant::now(),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        ),
        Err(ExecutionScopeAccessError::Unavailable)
    ));
}

struct UnscopedAdapter;

impl crate::ExecutionControlApi for UnscopedAdapter {
    fn owned(&self) -> crate::OwnedExecutionControl {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn cancellation_token(&self) -> crate::CancellationToken {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn check_cancelled(&self) -> crate::ExecutionControlResult<()> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn add_instruction_units(&self, _units: u64) -> crate::ExecutionControlResult<()> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn poll_execution_budget(&self) -> crate::ExecutionControlResult<()> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }

    fn file_source_stream_context(
        &self,
        _stream_runtime: crate::StreamRuntime,
    ) -> crate::FileSourceStreamContext<'static> {
        unreachable!("scope contract test does not use unrelated adapter methods")
    }
}

#[test]
fn effective_deadline_selects_earliest_and_keeps_outer_on_ties() {
    let now = std::time::Instant::now();
    let request_deadline = now + Duration::from_millis(100);
    let root = ExecutionScope::request(CancellationSource::new().token(), Some(request_deadline));

    let local_earlier = root
        .derive(
            now + Duration::from_millis(40),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("first nested scope");
    let effective = local_earlier
        .effective_deadline()
        .expect("local deadline should be effective");
    assert_eq!(effective.at(), now + Duration::from_millis(40));
    assert_eq!(effective.nesting(), 1);
    assert_eq!(
        effective.source(),
        &ExecutionDeadlineSource::Scope {
            site: site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        }
    );

    let request_earlier = root
        .derive(
            now + Duration::from_millis(200),
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("request-bounded scope");
    assert_eq!(
        request_earlier.effective_deadline(),
        root.effective_deadline(),
        "a local scope cannot extend the request deadline"
    );

    let outer = ExecutionScope::request(CancellationSource::new().token(), None)
        .derive(
            now + Duration::from_millis(60),
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("outer scope");
    let tied_inner = outer
        .derive(
            now + Duration::from_millis(60),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("inner scope");
    assert_eq!(tied_inner.nesting(), 2);
    assert_eq!(
        tied_inner.effective_deadline(),
        outer.effective_deadline(),
        "an equal inner deadline must retain the outer source and nesting"
    );
}

#[test]
fn only_the_scope_owned_deadline_projects_timeout_error() {
    let now = std::time::Instant::now();
    let parent = ExecutionScope::request(CancellationSource::new().token(), None)
        .derive(
            now + Duration::from_millis(10),
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("parent scope");
    let child = parent
        .derive(
            now + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("child scope");

    let inherited = child
        .terminal_at(now + Duration::from_millis(10))
        .expect("parent deadline should stop child");
    assert!(matches!(
        inherited,
        ExecutionScopeTerminal::InheritedDeadlineExceeded(_)
    ));
    assert_eq!(inherited.ordinary_payload(), None);
    assert_eq!(inherited.ordinary_catch_projection(), None);

    let local = parent
        .terminal_at(now + Duration::from_millis(10))
        .expect("owner should observe local deadline");
    let payload = local
        .ordinary_payload()
        .expect("owned local deadline is a TimeoutError");
    assert_eq!(payload.code, "TimeoutError");
    assert_eq!(
        payload.details,
        Some(json!({
            "reason": "deadlineExceeded",
            "deadlineSource": "scope",
            "deadlineNesting": 1,
            "deadlineSite": {
                "kind": "synthetic",
                "reason": "compilerGeneratedWrapper",
            },
        }))
    );
    assert!(
        local.ordinary_catch_projection().is_some(),
        "the owning scope may project its timeout"
    );

    let ancestor_cancelled = ExecutionScopeTerminal::AncestorCancelled;
    assert_eq!(ancestor_cancelled.ordinary_payload(), None);
    assert_eq!(ancestor_cancelled.ordinary_catch_projection(), None);
}

#[tokio::test(start_paused = true)]
async fn scope_leases_release_waiters_timers_and_child_work_for_every_exit() {
    let baseline = tokio::time::Instant::now();

    let normal_scope = ExecutionScope::request(CancellationSource::new().token(), None);
    let (normal_lease, normal_completion) = normal_scope.acquire_lease();
    assert_eq!(normal_scope.lifecycle_snapshot().active_leases, 1);
    assert_eq!(normal_scope.lifecycle_snapshot().active_waiters, 1);
    assert_eq!(normal_scope.lifecycle_snapshot().active_timers, 0);
    assert!(normal_completion.complete());
    assert_eq!(
        normal_lease.wait().await,
        ExecutionScopeLeaseTerminal::Completed
    );
    assert_eq!(
        normal_scope.lifecycle_snapshot(),
        Default::default(),
        "normal completion must release all lifecycle resources"
    );
    assert!(!normal_completion.complete(), "completion is one-shot");

    let parent_cancel = CancellationSource::new();
    let cancelled_scope = ExecutionScope::request(
        parent_cancel.token(),
        Some((baseline + Duration::from_secs(1)).into_std()),
    )
    .derive(
        (baseline + Duration::from_millis(20)).into_std(),
        site(SyntheticInstructionSiteReason::RuntimeControlFlow),
    )
    .expect("cancelled scope");
    let (cancelled_lease, _) = cancelled_scope.acquire_lease();
    let cancelled_child = cancelled_lease.child_cancellation_token();
    parent_cancel.cancel();
    tokio::time::advance(Duration::from_millis(20)).await;
    assert_eq!(
        cancelled_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::AncestorCancelled),
        "ancestor cancellation must beat a simultaneously ready local deadline"
    );
    assert!(cancelled_child.is_cancelled());
    assert_eq!(cancelled_scope.lifecycle_snapshot(), Default::default());

    let timeout_parent = CancellationSource::new();
    let timeout_scope = ExecutionScope::request(timeout_parent.token(), None)
        .derive(
            (tokio::time::Instant::now() + Duration::from_millis(10)).into_std(),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("timeout scope");
    let (timeout_lease, timeout_completion) = timeout_scope.acquire_lease();
    let timeout_child = timeout_lease.child_cancellation_token();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(
        !timeout_completion.complete(),
        "completion after the absolute deadline cannot beat the local terminal"
    );
    assert!(matches!(
        timeout_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert!(timeout_child.is_cancelled());
    assert!(timeout_scope.cancellation_signals().is_cancelled());
    assert!(
        !timeout_parent.is_cancelled(),
        "local timeout cannot cancel the shared parent token"
    );
    assert_eq!(timeout_scope.lifecycle_snapshot(), Default::default());
    assert!(
        !timeout_completion.complete(),
        "completion is permanently fenced"
    );

    let drop_scope = ExecutionScope::request(CancellationSource::new().token(), None)
        .derive(
            (tokio::time::Instant::now() + Duration::from_secs(1)).into_std(),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("drop scope");
    let (drop_lease, drop_completion) = drop_scope.acquire_lease();
    let dropped_child = drop_lease.child_cancellation_token();
    drop(drop_lease);
    assert!(dropped_child.is_cancelled());
    assert_eq!(drop_scope.lifecycle_snapshot(), Default::default());
    assert!(!drop_completion.complete());
}

#[tokio::test(start_paused = true)]
async fn equal_nested_deadline_is_only_projectable_by_the_outer_scope() {
    let deadline = (tokio::time::Instant::now() + Duration::from_millis(25)).into_std();
    let request_cancel = CancellationSource::new();
    let outer = ExecutionScope::request(request_cancel.token(), None)
        .derive(
            deadline,
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("outer scope");
    let inner = outer
        .derive(
            deadline,
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("inner scope");
    let (inner_lease, _) = inner.acquire_lease();
    let (outer_lease, _) = outer.acquire_lease();

    tokio::time::advance(Duration::from_millis(25)).await;
    let inner_terminal = inner_lease.wait().await;
    assert!(matches!(
        inner_terminal,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    if let ExecutionScopeLeaseTerminal::Control(terminal) = &inner_terminal {
        assert_eq!(terminal.ordinary_catch_projection(), None);
    }

    let outer_terminal = outer_lease.wait().await;
    assert!(matches!(
        outer_terminal,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    if let ExecutionScopeLeaseTerminal::Control(terminal) = &outer_terminal {
        assert!(terminal.ordinary_catch_projection().is_some());
    }
    assert!(!request_cancel.is_cancelled());
    assert_eq!(outer.lifecycle_snapshot(), Default::default());
}

#[tokio::test(start_paused = true)]
async fn lease_child_scope_preserves_scope_identity_and_normal_completion() {
    let deadline = (tokio::time::Instant::now() + Duration::from_millis(25)).into_std();
    let request_cancel = CancellationSource::new();
    let parent = ExecutionScope::request(request_cancel.token(), None)
        .derive(
            deadline,
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("parent scope");
    let (lease, completion) = parent.acquire_lease();
    let child = lease.child_execution_scope();

    assert_eq!(child.effective_deadline(), parent.effective_deadline());
    assert_eq!(child.nesting(), parent.nesting());
    assert_eq!(child.lifecycle_snapshot(), parent.lifecycle_snapshot());
    assert!(completion.complete());
    assert_eq!(lease.wait().await, ExecutionScopeLeaseTerminal::Completed);
    assert_eq!(
        child.terminal_at(tokio::time::Instant::now().into_std()),
        None
    );
    assert_eq!(
        parent.terminal_at(tokio::time::Instant::now().into_std()),
        None
    );
    assert!(!child.cancellation_signals().is_cancelled());
    assert!(!parent.cancellation_signals().is_cancelled());
    assert_eq!(parent.lifecycle_snapshot(), Default::default());
}

#[tokio::test(start_paused = true)]
async fn lease_drop_cancels_only_its_child_scope() {
    let parent = ExecutionScope::request(CancellationSource::new().token(), None);
    let (lease, completion) = parent.acquire_lease();
    let child = lease.child_execution_scope();

    drop(lease);

    assert_eq!(
        child.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert_eq!(
        parent.terminal_at(tokio::time::Instant::now().into_std()),
        None
    );
    assert!(!parent.cancellation_signals().is_cancelled());
    assert!(!completion.complete());
    assert_eq!(parent.lifecycle_snapshot(), Default::default());
}

#[tokio::test(start_paused = true)]
async fn lease_control_terminals_cancel_child_without_changing_parent_owner() {
    let baseline = tokio::time::Instant::now();

    let ancestor_cancel = CancellationSource::new();
    let ancestor_scope = ExecutionScope::request(ancestor_cancel.token(), None);
    let (ancestor_lease, _) = ancestor_scope.acquire_lease();
    let ancestor_child = ancestor_lease.child_execution_scope();
    ancestor_cancel.cancel();
    assert_eq!(
        ancestor_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert_eq!(
        ancestor_child.terminal_at(baseline.into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert_eq!(ancestor_scope.lifecycle_snapshot(), Default::default());

    let local_deadline = (baseline + Duration::from_millis(10)).into_std();
    let local_scope = ExecutionScope::request(CancellationSource::new().token(), None)
        .derive(
            local_deadline,
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("local scope");
    let (local_lease, local_completion) = local_scope.acquire_lease();
    let local_child = local_lease.child_execution_scope();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(!local_completion.complete());
    assert!(matches!(
        local_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(
        local_child.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert!(matches!(
        local_scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(local_scope.lifecycle_snapshot(), Default::default());

    let outer_deadline = (tokio::time::Instant::now() + Duration::from_millis(10)).into_std();
    let outer = ExecutionScope::request(CancellationSource::new().token(), None)
        .derive(
            outer_deadline,
            site(SyntheticInstructionSiteReason::CompilerGeneratedWrapper),
        )
        .expect("outer scope");
    let inherited = outer
        .derive(
            outer_deadline + Duration::from_millis(20),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("inherited scope");
    let (inherited_lease, _) = inherited.acquire_lease();
    let inherited_child = inherited_lease.child_execution_scope();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert!(matches!(
        inherited_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    assert_eq!(
        inherited_child.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert!(matches!(
        inherited.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    assert!(matches!(
        outer.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(inherited.lifecycle_snapshot(), Default::default());
}

#[tokio::test(start_paused = true)]
async fn lease_child_scope_keeps_cancel_first_and_sibling_isolation() {
    let baseline = tokio::time::Instant::now();
    let request_cancel = CancellationSource::new();
    let parent = ExecutionScope::request(request_cancel.token(), None)
        .derive(
            (baseline + Duration::from_millis(10)).into_std(),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("parent scope");
    let (first_lease, _) = parent.acquire_lease();
    let first_child = first_lease.child_execution_scope();
    let (second_lease, second_completion) = parent.acquire_lease();
    let second_child = second_lease.child_execution_scope();

    drop(first_lease);
    assert_eq!(
        first_child.terminal_at(baseline.into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert_eq!(second_child.terminal_at(baseline.into_std()), None);
    assert_eq!(parent.terminal_at(baseline.into_std()), None);

    request_cancel.cancel();
    tokio::time::advance(Duration::from_millis(10)).await;
    assert_eq!(
        second_lease.wait().await,
        ExecutionScopeLeaseTerminal::Control(ExecutionScopeTerminal::AncestorCancelled),
        "ancestor cancellation must beat a simultaneously ready local deadline"
    );
    assert_eq!(
        second_child.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    );
    assert!(!second_completion.complete());
    assert_eq!(parent.lifecycle_snapshot(), Default::default());
}

use std::{
    future::pending,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    CancellationSource, CancellationToken, ExecutionControl, ExecutionControlApi,
    ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError, ExecutionScopeTerminal,
    FileSourceStreamContext, OwnedExecutionControl, OwnedExecutionControlApi, StreamRuntime,
};
use tokio::sync::oneshot;

use crate::{
    error::RuntimeError,
    host::http_client_runtime::{
        await_http_body_open_lower_with_current_scope, await_http_lower_with_current_scope,
        await_http_request_lower_with_current_scope, await_http_sse_open_lower_with_current_scope,
    },
};

#[derive(Clone)]
struct TestControlState {
    scope: ExecutionScope,
    root_token: CancellationToken,
    root_flag: Arc<AtomicBool>,
}

impl TestControlState {
    fn from_scope(scope: ExecutionScope, root_token: CancellationToken) -> Self {
        Self {
            scope,
            root_token,
            root_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(TestOwnedControl(self.clone()))
    }
}

struct TestBorrowedControl(TestControlState);

impl ExecutionControlApi for TestBorrowedControl {
    fn owned(&self) -> OwnedExecutionControl {
        self.0.owned()
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.0.root_flag.clone()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.root_token.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0
            .scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.0.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        Ok(TestControlState {
            scope: self
                .0
                .scope
                .derive(local_deadline, site)
                .map_err(ExecutionScopeAccessError::from)?,
            root_token: self.0.root_token.clone(),
            root_flag: self.0.root_flag.clone(),
        }
        .owned())
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn file_source_stream_context(
        &self,
        _stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        unreachable!("HTTP current-scope tests do not use file streams")
    }
}

struct TestOwnedControl(TestControlState);

impl OwnedExecutionControlApi for TestOwnedControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(TestBorrowedControl(self.0.clone()))
    }

    fn cancelled(&self) -> &AtomicBool {
        self.0.root_flag.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.0.root_token.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.0
            .scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.0.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: std::time::Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        self.borrow().derive_scope(local_deadline, site)
    }
}

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn site(reason: SyntheticInstructionSiteReason) -> InstructionSourceSite {
    InstructionSourceSite::Synthetic { reason }
}

fn assert_owner_zero(scope: &ExecutionScope) {
    assert_eq!(
        scope.lifecycle_snapshot(),
        Default::default(),
        "HTTP current-scope operation must release lease, waiter, and timer ownership"
    );
}

async fn pending_lower_with_drop_probe(
    dropped: Arc<AtomicUsize>,
) -> crate::error::Result<&'static str> {
    let _drop_probe = DropProbe(dropped);
    pending().await
}

#[tokio::test]
async fn f445h_i6_http_current_scope_unary_open_observes_ancestor_stop() {
    let ancestor = CancellationSource::new();
    let scope = ExecutionScope::request(ancestor.token(), None);
    ancestor.cancel();
    assert_entry_observes_ancestor_stop(scope, |scope, dropped| {
        Box::pin(await_http_request_lower_with_current_scope(
            scope,
            None,
            || pending_lower_with_drop_probe(dropped),
        ))
    })
    .await;
}

#[tokio::test]
async fn f445h_i6_http_current_scope_body_stream_open_observes_ancestor_stop() {
    let ancestor = CancellationSource::new();
    let scope = ExecutionScope::request(ancestor.token(), None);
    ancestor.cancel();
    assert_entry_observes_ancestor_stop(scope, |scope, dropped| {
        Box::pin(await_http_body_open_lower_with_current_scope(
            scope,
            None,
            || pending_lower_with_drop_probe(dropped),
        ))
    })
    .await;
}

#[tokio::test]
async fn f445h_i6_http_current_scope_sse_open_observes_ancestor_stop() {
    let ancestor = CancellationSource::new();
    let scope = ExecutionScope::request(ancestor.token(), None);
    ancestor.cancel();
    assert_entry_observes_ancestor_stop(scope, |scope, dropped| {
        Box::pin(await_http_sse_open_lower_with_current_scope(
            scope,
            None,
            || pending_lower_with_drop_probe(dropped),
        ))
    })
    .await;
}

async fn assert_entry_observes_ancestor_stop<F>(scope: ExecutionScope, operation: F)
where
    F: FnOnce(
        ExecutionScope,
        Arc<AtomicUsize>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = crate::error::Result<&'static str>>>,
    >,
{
    let dropped = Arc::new(AtomicUsize::new(0));

    let error = operation(scope.clone(), dropped.clone())
        .await
        .expect_err("ancestor stop must cancel pending HTTP open");

    assert!(error.is_cancellation_terminal());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_owner_zero(&scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_current_deadline_stops_pending_lower() {
    let root = CancellationSource::new();
    let request_scope = ExecutionScope::request(root.token(), None);
    let deadline = std::time::Instant::now() - Duration::from_millis(1);
    let current_scope = request_scope
        .derive(
            deadline,
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derive current scope");
    let dropped = Arc::new(AtomicUsize::new(0));
    let operation = await_http_lower_with_current_scope(current_scope.clone(), None, || {
        pending_lower_with_drop_probe(dropped.clone())
    });
    let error = operation
        .await
        .expect_err("current deadline must stop pending HTTP lower");

    assert!(error.is_cancellation_terminal());
    assert!(matches!(
        current_scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert_owner_zero(&current_scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_outer_deadline_stops_pending_lower() {
    let root = CancellationSource::new();
    let request_scope = ExecutionScope::request(root.token(), None);
    let now = std::time::Instant::now();
    let outer_scope = request_scope
        .derive(
            now - Duration::from_millis(1),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derive outer scope");
    let current_scope = outer_scope
        .derive(
            now + Duration::from_secs(1),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derive current scope");
    let operation = await_http_lower_with_current_scope(current_scope.clone(), None, || async {
        pending::<crate::error::Result<()>>().await
    });
    let error = operation
        .await
        .expect_err("outer deadline must stop pending HTTP lower");

    assert!(error.is_cancellation_terminal());
    assert!(matches!(
        current_scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    assert_owner_zero(&current_scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_internal_parent_stop_stops_pending_lower() {
    let root = CancellationSource::new();
    let parent_scope = ExecutionScope::request(root.token(), None);
    let (parent_lease, _parent_completion) = parent_scope.acquire_lease();
    let current_scope = parent_lease.child_execution_scope();
    drop(parent_lease);

    let error = await_http_lower_with_current_scope(current_scope.clone(), None, || async {
        pending::<crate::error::Result<()>>().await
    })
    .await
    .expect_err("internal parent stop must cancel pending HTTP lower");

    assert!(error.is_cancellation_terminal());
    assert_owner_zero(&current_scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_primitive_timeout_is_timeout_error() {
    let root = CancellationSource::new();
    let scope = ExecutionScope::request(root.token(), None);
    let operation = await_http_lower_with_current_scope(scope.clone(), Some(0), || async {
        pending::<crate::error::Result<()>>().await
    });
    let error = operation
        .await
        .expect_err("HTTP primitive timeout must stop pending lower");

    match error {
        RuntimeError::ExternalErrorPayload { code, details, .. } => {
            assert_eq!(code, "TimeoutError");
            assert_eq!(
                details
                    .as_ref()
                    .and_then(|value| value.get("reason"))
                    .and_then(serde_json::Value::as_str),
                Some("httpRequestTimeout")
            );
        }
        other => panic!("expected TimeoutError payload, got {other:?}"),
    }
    assert_owner_zero(&scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_earlier_current_deadline_beats_primitive_timeout() {
    let root = CancellationSource::new();
    let request_scope = ExecutionScope::request(root.token(), None);
    let current_scope = request_scope
        .derive(
            std::time::Instant::now() - Duration::from_millis(1),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derive current scope");
    let operation =
        await_http_lower_with_current_scope(current_scope.clone(), Some(40), || async {
            pending::<crate::error::Result<()>>().await
        });
    let error = operation
        .await
        .expect_err("earlier current deadline must win");

    assert!(error.is_cancellation_terminal());
    assert_owner_zero(&current_scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_ready_lower_commits_before_same_turn_signal() {
    let ancestor = CancellationSource::new();
    let scope = ExecutionScope::request(ancestor.token(), None);
    ancestor.cancel();

    let output =
        await_http_lower_with_current_scope(scope.clone(), None, || async { Ok("committed") })
            .await
            .expect("ready lower completion must commit before a same-turn signal");

    assert_eq!(output, "committed");
    assert!(matches!(
        scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    ));
    assert_owner_zero(&scope);
}

#[tokio::test]
async fn f445h_i6_http_current_scope_late_lower_completion_cannot_deliver() {
    let ancestor = CancellationSource::new();
    let scope = ExecutionScope::request(ancestor.token(), None);
    let (lower_tx, lower_rx) = oneshot::channel::<crate::error::Result<&'static str>>();
    let dropped = Arc::new(AtomicUsize::new(0));
    let operation = await_http_lower_with_current_scope(scope.clone(), None, || async {
        let _drop_probe = DropProbe(dropped.clone());
        lower_rx
            .await
            .expect("test lower sender should stay available")
    });
    tokio::pin!(operation);

    ancestor.cancel();
    let error = operation
        .await
        .expect_err("scope winner must prevent lower delivery");
    assert!(error.is_cancellation_terminal());
    assert_eq!(dropped.load(Ordering::SeqCst), 1);
    assert!(
        lower_tx.send(Ok("late")).is_err(),
        "late lower completion receiver must already be dropped"
    );
    assert_owner_zero(&scope);
}

#[test]
fn f445h_i6_http_current_scope_owned_carrier_exposes_full_current_scope() {
    let root = CancellationSource::new();
    let request_scope = ExecutionScope::request(root.token(), None);
    let current_scope = request_scope
        .derive(
            std::time::Instant::now() + Duration::from_secs(1),
            site(SyntheticInstructionSiteReason::RuntimeControlFlow),
        )
        .expect("derive current scope");
    let owned = TestControlState::from_scope(current_scope.clone(), root.token()).owned();

    let received = owned.execution_scope().expect("read full current scope");

    assert_eq!(received.nesting(), current_scope.nesting());
    assert_eq!(
        received.effective_deadline().map(|deadline| deadline.at()),
        current_scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    );
    assert_owner_zero(&received);
}

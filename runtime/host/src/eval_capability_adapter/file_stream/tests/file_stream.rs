use std::{
    future,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};

use super::*;

#[derive(Clone)]
struct TestExecution {
    token: capability_contract::CancellationToken,
    cancel_flag: Arc<AtomicBool>,
    scope: capability_contract::ExecutionScope,
}

impl TestExecution {
    fn request(
        request_deadline: Option<Instant>,
    ) -> (
        capability_contract::CancellationSource,
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScope,
    ) {
        let cancellation = capability_contract::CancellationSource::new();
        let token = cancellation.token();
        let scope = capability_contract::ExecutionScope::request(token.clone(), request_deadline);
        let execution = Self {
            cancel_flag: token.cancel_flag(),
            token,
            scope: scope.clone(),
        };
        (
            cancellation,
            capability_contract::OwnedExecutionControl::new(execution),
            scope,
        )
    }

    fn execution_error(&self) -> Option<capability_contract::ExecutionControlError> {
        match self.scope.terminal_at(Instant::now()) {
            Some(capability_contract::ExecutionScopeTerminal::AncestorCancelled) => {
                Some(capability_contract::ExecutionControlError::Cancelled)
            }
            Some(
                capability_contract::ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | capability_contract::ExecutionScopeTerminal::InheritedDeadlineExceeded(_),
            ) => Some(capability_contract::ExecutionControlError::BudgetExceeded(
                capability_contract::ExecutionBudgetFailure {
                    reason: capability_contract::ExecutionBudgetReason::DeadlineExceeded,
                    instruction_count: 0,
                    limit: None,
                    elapsed_ms: 0.0,
                },
            )),
            None => None,
        }
    }
}

impl capability_contract::ExecutionControlApi for TestExecution {
    fn owned(&self) -> capability_contract::OwnedExecutionControl {
        capability_contract::OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    fn cancellation_token(&self) -> capability_contract::CancellationToken {
        self.token.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(capability_contract::EffectiveDeadline::at)
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        let scope = self.scope.derive(local_deadline, site)?;
        Ok(capability_contract::OwnedExecutionControl::new(Self {
            token: self.token.clone(),
            cancel_flag: Arc::clone(&self.cancel_flag),
            scope,
        }))
    }

    fn check_cancelled(&self) -> capability_contract::ExecutionControlResult<()> {
        self.execution_error().map_or(Ok(()), Err)
    }

    fn add_instruction_units(
        &self,
        _units: u64,
    ) -> capability_contract::ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> capability_contract::ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        _stream_runtime: capability_contract::StreamRuntime,
    ) -> capability_contract::FileSourceStreamContext<'static> {
        panic!("file source context is not used by scoped adapter tests")
    }
}

impl capability_contract::OwnedExecutionControlApi for TestExecution {
    fn borrow(&self) -> capability_contract::ExecutionControl<'_> {
        capability_contract::ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancel_flag.as_ref()
    }

    fn cancellation_token(&self) -> capability_contract::CancellationToken {
        self.token.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        capability_contract::ExecutionControlApi::deadline(self)
    }

    fn execution_scope(
        &self,
    ) -> std::result::Result<
        capability_contract::ExecutionScope,
        capability_contract::ExecutionScopeAccessError,
    > {
        capability_contract::ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<
        capability_contract::OwnedExecutionControl,
        capability_contract::ExecutionScopeAccessError,
    > {
        capability_contract::ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

struct DropProbe(Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn assert_scope_released(scope: &capability_contract::ExecutionScope) {
    assert_eq!(
        scope.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default(),
    );
}

fn assert_cancelled(error: FileCapabilityError) {
    assert!(matches!(
        error,
        FileCapabilityError::Execution(capability_contract::ExecutionControlError::Cancelled)
    ));
}

fn assert_deadline(error: FileCapabilityError) {
    assert!(matches!(
        error,
        FileCapabilityError::Execution(capability_contract::ExecutionControlError::BudgetExceeded(
            capability_contract::ExecutionBudgetFailure {
                reason: capability_contract::ExecutionBudgetReason::DeadlineExceeded,
                ..
            }
        ))
    ));
}

#[tokio::test]
async fn cloned_request_stream_owner_keeps_scope_open_until_last_clone_drops() {
    let concrete_runtime = concrete::StreamRuntime::default();
    let runtime =
        capability_contract::StreamRuntime::new(RuntimeStreamRuntime(concrete_runtime.clone()));
    let (runtime, owner) = runtime.request_scope(47);
    let escaping_owner = owner.clone();
    let (stream, sink) = runtime.channel_stream();
    sink.send(json!("still-owned"))
        .await
        .expect("request stream item");

    drop(owner);

    assert_eq!(concrete_runtime.active_stream_count_in_scope(47), 1);
    assert!(matches!(
        runtime.next(&stream).await.expect("stream remains registered"),
        capability_contract::StreamPoll::Item(value) if value == json!("still-owned")
    ));

    drop(escaping_owner);

    assert_eq!(concrete_runtime.active_stream_count_in_scope(47), 0);
    assert!(runtime
        .next(&stream)
        .await
        .expect_err("last owner clone closes the request scope")
        .to_string()
        .contains("unknown Stream value"));
}

#[tokio::test]
async fn detached_stream_task_can_retain_the_selected_request_scope() {
    let concrete_runtime = concrete::StreamRuntime::default();
    let runtime =
        capability_contract::StreamRuntime::new(RuntimeStreamRuntime(concrete_runtime.clone()));
    let (runtime, request_owner) = runtime.request_scope(48);
    let task_owner = runtime
        .retain_request_scope()
        .expect("scoped runtime can open a detached task owner");
    let (stream, sink) = runtime.channel_stream();
    sink.send(json!("task-owned"))
        .await
        .expect("request stream item");

    drop(request_owner);

    assert_eq!(concrete_runtime.active_stream_count_in_scope(48), 1);
    assert!(matches!(
        runtime.next(&stream).await.expect("task owner retains scope"),
        capability_contract::StreamPoll::Item(value) if value == json!("task-owned")
    ));

    drop(task_owner);

    assert_eq!(concrete_runtime.active_stream_count_in_scope(48), 0);
}

#[tokio::test]
async fn f445h_i6_file_scope_direct_ready_has_no_residual_owner() {
    let (_cancellation, execution, scope) = TestExecution::request(None);
    let output = scoped_file_future(
        execution,
        "direct-ready",
        future::ready(Ok(json!({"ready": true}))),
    )
    .await
    .expect("ready lower result");
    assert_eq!(output, json!({"ready": true}));
    assert_scope_released(&scope);
}

#[tokio::test]
async fn f445h_i6_file_scope_direct_pending_current_deadline_drops_lower() {
    let (_cancellation, root, _) = TestExecution::request(None);
    let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
    let execution = root
        .derive_scope(deadline.into_std(), site())
        .expect("current scope");
    let scope = execution.execution_scope().expect("scope");
    let drops = Arc::new(AtomicUsize::new(0));
    let lower_drops = Arc::clone(&drops);
    let wait = scoped_file_future(execution, "direct-pending", async move {
        let _probe = DropProbe(lower_drops);
        future::pending::<capability_contract::FileCapabilityResult<Value>>().await
    });
    tokio::pin!(wait);
    assert!(
        future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(true)
        })
        .await
    );
    assert_eq!(scope.lifecycle_snapshot().active_leases, 1);
    assert_deadline(
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .expect("current deadline wakes")
            .expect_err("current deadline wins"),
    );
    assert_eq!(drops.load(Ordering::Acquire), 1);
    assert_scope_released(&scope);
}

#[tokio::test]
async fn f445h_i6_file_scope_provider_pending_outer_deadline_keeps_owner() {
    let request_deadline = (tokio::time::Instant::now() + Duration::from_millis(20)).into_std();
    let (_cancellation, root, _) = TestExecution::request(Some(request_deadline));
    let execution = root
        .derive_scope(
            (tokio::time::Instant::now() + Duration::from_secs(30)).into_std(),
            site(),
        )
        .expect("derived provider scope");
    let scope = execution.execution_scope().expect("scope");
    let owner = scope
        .effective_deadline()
        .expect("deadline")
        .source()
        .clone();
    let wait = scoped_file_future(execution, "provider-pending", async {
        future::pending::<capability_contract::FileCapabilityResult<Value>>().await
    });
    tokio::pin!(wait);
    assert_deadline(
        tokio::time::timeout(Duration::from_secs(1), wait)
            .await
            .expect("outer deadline wakes")
            .expect_err("outer deadline wins"),
    );
    assert_eq!(owner, capability_contract::ExecutionDeadlineSource::Request,);
    assert_scope_released(&scope);
}

#[tokio::test]
async fn f445h_i6_file_scope_source_pending_ancestor_stop_fences_late_item() {
    let (cancellation, execution, scope) = TestExecution::request(None);
    let (late_tx, late_rx) =
        tokio::sync::oneshot::channel::<capability_contract::FileCapabilityResult<Option<Value>>>();
    let (late_error_tx, late_error_rx) =
        tokio::sync::oneshot::channel::<capability_contract::FileCapabilityResult<Option<Value>>>();
    let wait = scoped_file_future(execution.clone(), "source-next-item", async move {
        late_rx.await.expect("late source sender")
    });
    let error_wait = scoped_file_future(execution, "source-next-error", async move {
        late_error_rx.await.expect("late source error sender")
    });
    tokio::pin!(wait);
    tokio::pin!(error_wait);
    assert!(
        future::poll_fn(|cx| {
            assert!(wait.as_mut().poll(cx).is_pending());
            assert!(error_wait.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(true)
        })
        .await
    );
    assert_eq!(scope.lifecycle_snapshot().active_leases, 2);
    cancellation.cancel();
    assert_cancelled(wait.await.expect_err("ancestor stop wins"));
    assert_cancelled(error_wait.await.expect_err("ancestor stop fences errors"));
    assert!(
        late_tx.send(Ok(Some(json!("late")))).is_err(),
        "scope terminal drops the source receiver before a late item can publish"
    );
    assert!(
        late_error_tx
            .send(Err(FileCapabilityError::file("late error")))
            .is_err(),
        "scope terminal drops the source receiver before a late error can publish"
    );
    assert_scope_released(&scope);
}

#[tokio::test]
async fn f445h_i6_file_scope_normal_completion_commits_before_late_stop() {
    let (cancellation, execution, scope) = TestExecution::request(None);
    let (normal_tx, normal_rx) = tokio::sync::oneshot::channel();
    let wait = tokio::spawn(scoped_file_future(execution, "normal-race", async move {
        normal_rx
            .await
            .map_err(|error| FileCapabilityError::decode(error.to_string()))
    }));
    tokio::task::yield_now().await;
    normal_tx
        .send(json!("normal"))
        .expect("normal result sender");
    let output = wait
        .await
        .expect("join normal wait")
        .expect("normal result commits");
    cancellation.cancel();
    assert_eq!(output, json!("normal"));
    assert_scope_released(&scope);
}

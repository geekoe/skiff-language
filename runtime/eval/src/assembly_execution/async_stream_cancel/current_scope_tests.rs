use std::{
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_activation::RequestActivationContext;
use skiff_runtime_capability_context::{
    CancellationToken, ExecutionControl, ExecutionControlApi, ExecutionControlResult,
    ExecutionScope, ExecutionScopeAccessError, FileSourceStreamContext, OwnedExecutionControl,
    OwnedExecutionControlApi, StreamCancelSignal, StreamCancelSignalApi, StreamRuntimeResult,
};
use skiff_runtime_model::runtime_value::RuntimeValue;

use super::*;
use crate::{assembly_execution::ordinary::tests::test_runtime, error::RuntimeError};

#[derive(Clone)]
struct ScopedControl {
    root: CancellationToken,
    cancelled: Arc<AtomicBool>,
    scope: ExecutionScope,
}

impl ScopedControl {
    fn new(root: CancellationToken, scope: ExecutionScope) -> Self {
        Self {
            cancelled: root.cancel_flag(),
            root,
            scope,
        }
    }

    fn owned(self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self)
    }
}

impl ExecutionControlApi for ScopedControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.root.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self
            .scope
            .derive(local_deadline, site)
            .map_err(ExecutionScopeAccessError::Derive)?;
        Ok(Self::new(self.root.clone(), scope).owned())
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.root.is_cancelled() {
            Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: crate::capabilities::StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for ScopedControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.root.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

#[derive(Debug)]
struct TestStreamCancel(CancellationToken);

impl StreamCancelSignalApi for TestStreamCancel {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(self.0.wait_cancelled())
    }
}

fn stream_cancel() -> (CancellationToken, StreamCancelSignal) {
    let token = CancellationToken::new();
    (
        token.clone(),
        StreamCancelSignal::new(TestStreamCancel(token)),
    )
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

#[tokio::test]
async fn f445h_e4r_stream_provider_terminal_observes_lease_child_signal() {
    let root = CancellationToken::new();
    let parent = ExecutionScope::request(root.clone(), None);
    let (lease, _completion) = parent.acquire_lease();
    let execution = ScopedControl::new(root, lease.child_execution_scope()).owned();
    let (_cancel, stream_cancel) = stream_cancel();
    let (started, provider_started) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn(async move {
        await_provider_stream_terminal(&execution, &stream_cancel, async move {
            started.send(()).expect("provider wait start gate");
            std::future::pending::<Result<RuntimeValue>>().await
        })
        .await
    });
    provider_started
        .await
        .expect("provider future enters the real terminal wait");
    drop(lease);
    assert!(matches!(
        waiter.await.expect("provider waiter joins"),
        ProviderTerminal::RequestCancelled
    ));
    assert_eq!(
        parent.lifecycle_snapshot(),
        Default::default(),
        "provider terminal wait leaves no lease waiter or timer state"
    );
}

#[tokio::test]
async fn f445h_e4r_stream_provider_publication_preserves_local_deadline_owner() {
    let root = CancellationToken::new();
    let parent = ExecutionScope::request(root.clone(), None);
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("expired deadline");
    let scope = parent
        .derive(deadline, site())
        .expect("derive provider scope");
    let execution = ScopedControl::new(root, scope.clone()).owned();
    let (_cancel, stream_cancel) = stream_cancel();
    let request =
        RequestActivationContext::begin(super::tests::activation("scope-publication", "build"))
            .expect("provider request");
    let publication =
        await_provider_publication(&execution, &stream_cancel, &request, std::future::pending())
            .await;
    let ProviderPublication::DeadlineExceeded(RuntimeError::ScopeTerminal(carrier)) = publication
    else {
        panic!("publication must retain the local deadline carrier");
    };
    assert_eq!(carrier.effective_deadline().at(), deadline);
    assert!(carrier.is_owned_by(&scope));
    assert!(request.open_stream().is_none());
}

#[tokio::test]
async fn f445h_e4r_stream_provider_cancel_wins_same_time_deadline_and_ready_result() {
    let root = CancellationToken::new();
    let parent = ExecutionScope::request(root.clone(), None);
    let deadline = Instant::now()
        .checked_sub(Duration::from_millis(1))
        .expect("expired deadline");
    let scope = parent
        .derive(deadline, site())
        .expect("derive provider scope");
    root.cancel();
    let execution = ScopedControl::new(root, scope).owned();
    let (_cancel, stream_cancel) = stream_cancel();
    let terminal = await_provider_stream_terminal(&execution, &stream_cancel, async {
        Ok(RuntimeValue::Bool(true))
    })
    .await;
    assert!(matches!(terminal, ProviderTerminal::RequestCancelled));
}

#[tokio::test]
async fn f445h_e4r_stream_provider_item_publication_observes_lease_child_signal() {
    let root = CancellationToken::new();
    let parent = ExecutionScope::request(root.clone(), None);
    let (lease, _completion) = parent.acquire_lease();
    let execution = ScopedControl::new(root, lease.child_execution_scope()).owned();
    let request = RequestActivationContext::begin(super::tests::activation("scope-item", "build"))
        .expect("provider request");
    let (started, publication_started) = tokio::sync::oneshot::channel();
    let waiter = tokio::spawn({
        let request = request.clone();
        async move {
            await_stream_item_publication(&execution, &request, async move {
                started.send(()).expect("item publication start gate");
                std::future::pending::<StreamRuntimeResult<()>>().await
            })
            .await
        }
    });
    publication_started
        .await
        .expect("item enters the real publication wait");
    drop(lease);
    let error = waiter
        .await
        .expect("publication waiter joins")
        .expect_err("lease child stop terminates publication");
    assert!(matches!(RuntimeError::from(error), RuntimeError::Cancelled));
    assert!(request.open_stream().is_none());
    assert_eq!(
        parent.lifecycle_snapshot(),
        Default::default(),
        "item publication leaves no lease waiter or timer state"
    );
}

#[tokio::test]
async fn f445h_e4r_stream_provider_task_runs_real_terminal_publication_path() {
    let (mut task, _generation, stream_runtime, stream_value, _) =
        super::tests::provider_stream_failure_task();
    let activity_probe = Arc::new(ProviderStreamTaskActivityProbe::default());
    task.activity_probe = Some(Arc::clone(&activity_probe));
    run_provider_stream(task).await;
    let error = stream_runtime
        .next(&stream_value)
        .await
        .expect_err("provider task publishes its typed failure");
    assert!(error.fixed_service_failure_parts().is_some());
    assert_eq!(
        activity_probe.entered(),
        1,
        "direct task execution enters its provider task owner exactly once"
    );
    assert_eq!(
        activity_probe.active(),
        0,
        "direct task execution leaves no provider task owner behind"
    );
}

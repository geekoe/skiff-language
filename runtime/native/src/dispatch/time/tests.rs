use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Wake, Waker},
    time::Duration,
};

use skiff_artifact_model::InstructionSourceSite;
use skiff_runtime_capability_context::{
    CancellationSource, CancellationToken, ExecutionControl, ExecutionControlApi,
    ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError,
    ExecutionScopeLifecycleSnapshot, ExecutionScopeTerminal, FileSourceStreamContext,
    OwnedExecutionControl, OwnedExecutionControlApi, StreamRuntime,
};

use super::*;

#[derive(Clone)]
struct ScopeTimeContext {
    execution: OwnedExecutionControl,
    budget_polls: Arc<AtomicUsize>,
}

impl NativeTimeCapability for ScopeTimeContext {
    fn execution_control(&self) -> OwnedExecutionControl {
        self.execution.clone()
    }

    fn poll_execution_budget(&self) -> Result<()> {
        self.budget_polls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

#[derive(Clone)]
struct ScopeControl {
    scope: ExecutionScope,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
}

impl ScopeControl {
    fn owned(scope: ExecutionScope, cancellation: CancellationToken) -> OwnedExecutionControl {
        OwnedExecutionControl::new(Self {
            scope,
            cancelled: cancellation.cancel_flag(),
            cancellation,
        })
    }
}

impl ExecutionControlApi for ScopeControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
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
        panic!("time scope tests do not create file source streams")
    }
}

impl OwnedExecutionControlApi for ScopeControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }
}

fn scope_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn context(
    scope: ExecutionScope,
    cancellation: CancellationToken,
) -> (ScopeTimeContext, Arc<AtomicUsize>) {
    let budget_polls = Arc::new(AtomicUsize::new(0));
    (
        ScopeTimeContext {
            execution: ScopeControl::owned(scope, cancellation),
            budget_polls: Arc::clone(&budget_polls),
        },
        budget_polls,
    )
}

fn assert_idle(scope: &ExecutionScope) {
    assert_eq!(
        scope.lifecycle_snapshot(),
        ExecutionScopeLifecycleSnapshot::default()
    );
}

struct TimeScopeNoopWake;

impl Wake for TimeScopeNoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_sleep<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(TimeScopeNoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

#[tokio::test]
async fn f445h_i6_time_scope_normal_wake_commits_and_releases_owner() {
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), None);
    let (time, polls) = context(scope.clone(), cancellation.token());

    let mut sleep = Box::pin(sleep_for_millis(time, 5));
    assert!(matches!(poll_sleep(sleep.as_mut()), Poll::Pending));
    assert_eq!(scope.lifecycle_snapshot().active_leases, 1);

    tokio::time::sleep(Duration::from_millis(10)).await;
    let Poll::Ready(result) = poll_sleep(sleep.as_mut()) else {
        panic!("normal timer must wake sleep");
    };
    result.expect("normal wake");
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&scope);

    cancellation.cancel();
    assert_idle(&scope);
}

#[tokio::test]
async fn f445h_i6_time_scope_current_deadline_wakes_without_polling() {
    let cancellation = CancellationSource::new();
    let base = tokio::time::Instant::now().into_std();
    let root = ExecutionScope::request(cancellation.token(), None);
    let scope = root
        .derive(base + Duration::from_millis(5), scope_site())
        .expect("current scope");
    let (time, polls) = context(scope.clone(), cancellation.token());

    let mut sleep = Box::pin(sleep_for_millis(time, 100));
    assert!(matches!(poll_sleep(sleep.as_mut()), Poll::Pending));
    assert_eq!(
        scope.lifecycle_snapshot(),
        ExecutionScopeLifecycleSnapshot {
            active_leases: 1,
            active_waiters: 1,
            active_timers: 1,
        }
    );

    tokio::time::sleep(Duration::from_millis(10)).await;
    let Poll::Ready(result) = poll_sleep(sleep.as_mut()) else {
        panic!("current deadline must wake sleep");
    };
    assert!(matches!(result, Err(RuntimeError::Cancelled),));
    assert!(matches!(
        scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&scope);
}

#[tokio::test]
async fn f445h_i6_time_scope_outer_deadline_keeps_inherited_owner() {
    let cancellation = CancellationSource::new();
    let base = tokio::time::Instant::now().into_std();
    let root = ExecutionScope::request(cancellation.token(), None);
    let outer = root
        .derive(base + Duration::from_millis(5), scope_site())
        .expect("outer scope");
    let current = outer
        .derive(base + Duration::from_millis(20), scope_site())
        .expect("current scope");
    let (time, polls) = context(current.clone(), cancellation.token());

    let mut sleep = Box::pin(sleep_for_millis(time, 100));
    assert!(matches!(poll_sleep(sleep.as_mut()), Poll::Pending));
    tokio::time::sleep(Duration::from_millis(10)).await;
    let Poll::Ready(result) = poll_sleep(sleep.as_mut()) else {
        panic!("outer deadline must wake sleep");
    };
    assert!(matches!(result, Err(RuntimeError::Cancelled),));
    assert!(matches!(
        current.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&current);
}

#[tokio::test]
async fn f445h_i6_time_scope_ancestor_stop_wakes_immediately() {
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), None);
    let (time, polls) = context(scope.clone(), cancellation.token());

    let mut sleep = Box::pin(sleep_for_millis(time, 100));
    assert!(matches!(poll_sleep(sleep.as_mut()), Poll::Pending));
    cancellation.cancel();

    let Poll::Ready(result) = poll_sleep(sleep.as_mut()) else {
        panic!("ancestor stop must wake sleep without a timer tick");
    };
    assert!(matches!(result, Err(RuntimeError::Cancelled),));
    assert!(matches!(
        scope.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::AncestorCancelled)
    ));
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&scope);
}

#[tokio::test]
async fn f445h_i6_time_scope_internal_deadline_signal_wakes_with_clock_stationary() {
    let cancellation = CancellationSource::new();
    let base = tokio::time::Instant::now().into_std();
    let deadline = base + Duration::from_secs(60);
    let root = ExecutionScope::request(cancellation.token(), None);
    let scope = root.derive(deadline, scope_site()).expect("current scope");
    let (time, polls) = context(scope.clone(), cancellation.token());

    let mut sleep = Box::pin(sleep_for_millis(time, TIME_SLEEP_MAX_MILLIS));
    assert!(matches!(poll_sleep(sleep.as_mut()), Poll::Pending));
    assert!(matches!(
        scope.terminal_at(deadline),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));

    let Poll::Ready(result) = poll_sleep(sleep.as_mut()) else {
        panic!("internal scope signal must wake sleep without a timer tick");
    };
    assert!(matches!(result, Err(RuntimeError::Cancelled),));
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&scope);
}

#[tokio::test]
async fn f445h_i6_time_scope_zero_duration_is_ready_without_owner() {
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), None);
    let (time, polls) = context(scope.clone(), cancellation.token());

    sleep_for_millis(time, 0)
        .await
        .expect("zero duration remains Ready");

    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_idle(&scope);
}

#[test]
fn f445h_i6_time_scope_decode_clamp_and_sync_date_helper_stay_synchronous() {
    assert_eq!(
        sleep_millis_from_runtime_value(&RuntimeValue::Number(-1.0))
            .expect("negative duration clamps"),
        0
    );
    assert_eq!(
        sleep_millis_from_runtime_value(&RuntimeValue::Number((TIME_SLEEP_MAX_MILLIS + 1) as f64,))
            .expect("large duration clamps"),
        TIME_SLEEP_MAX_MILLIS
    );
    assert!(matches!(
        sleep_millis_from_runtime_value(&RuntimeValue::Number(1.5)),
        Err(RuntimeError::Decode(_))
    ));

    assert!(!TimeNativeDispatch::matches("core.date.now"));
    assert!(
        crate::registry::NativeRegistry
            .dispatch("core.date.now", &[])
            .expect("sync date helper dispatch")
            .is_some(),
        "date/time synchronous helpers remain on the immediate registry path"
    );
}

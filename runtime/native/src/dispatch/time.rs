use super::{
    prepared::run_prepared_native_call, unsupported_native_target, PreparedExternalNativeOperation,
    PreparedNativeCall, RuntimeNativeInvocation,
};
use crate::capability::NativeTimeCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeValue};
use skiff_runtime_capability_context::ExecutionScopeLeaseTerminal;

const TIME_SLEEP_KEY: &str = "std.time.sleep";
pub(super) const TIME_SLEEP_MAX_MILLIS: u64 = 60_000;

pub(super) struct TimeNativeDispatch;

impl TimeNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        target == TIME_SLEEP_KEY
    }

    pub(super) fn prepare<'a, TimeContext>(
        time_context: TimeContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<PreparedNativeCall<'a>>
    where
        TimeContext: NativeTimeCapability + Send + 'a,
    {
        let binding_key = invocation.binding_key();
        match binding_key {
            TIME_SLEEP_KEY => {
                let value = args.first().ok_or_else(|| {
                    RuntimeError::Decode(format!("{diagnostic_target} requires duration"))
                })?;
                let value = invocation.native_boundary()?.coerce_arg(
                    0,
                    value,
                    &format!("{diagnostic_target} duration"),
                    heap,
                )?;
                let millis = sleep_millis_from_runtime_value(&value)?;
                Ok(PreparedNativeCall::ExternalWait(
                    PreparedExternalNativeOperation::new(
                        async move {
                            sleep_for_millis(time_context, millis).await?;
                            Ok(())
                        },
                        move |(), heap| {
                            invocation.native_boundary()?.coerce_return(
                                &RuntimeValue::Null,
                                &format!("{diagnostic_target} response"),
                                heap,
                            )
                        },
                    ),
                ))
            }
            _ => Err(unsupported_native_target(binding_key)),
        }
    }

    #[allow(dead_code)]
    pub(super) async fn dispatch<TimeContext>(
        time_context: TimeContext,
        invocation: RuntimeNativeInvocation,
        diagnostic_target: String,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue>
    where
        TimeContext: NativeTimeCapability + Send,
    {
        let prepared = Self::prepare(time_context, invocation, diagnostic_target, args, heap)?;
        run_prepared_native_call(prepared, heap).await
    }
}

pub(super) fn sleep_millis_from_runtime_value(value: &RuntimeValue) -> Result<u64> {
    let RuntimeValue::Number(value) = value else {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be an integer millisecond payload".to_string(),
        ));
    };
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be an integer millisecond payload".to_string(),
        ));
    }
    if value.abs() > 9_007_199_254_740_991.0 {
        return Err(RuntimeError::Decode(
            "std.time.sleep duration must be a safe integer millisecond payload".to_string(),
        ));
    }
    Ok(clamp_sleep_millis(*value))
}

pub(super) fn clamp_sleep_millis(value: f64) -> u64 {
    if value <= 0.0 {
        return 0;
    }
    if value >= TIME_SLEEP_MAX_MILLIS as f64 {
        return TIME_SLEEP_MAX_MILLIS;
    }
    value as u64
}

async fn sleep_for_millis(time_context: impl NativeTimeCapability, millis: u64) -> Result<()> {
    time_context.poll_execution_budget()?;
    if millis == 0 {
        return Ok(());
    }

    let execution = time_context.execution_control();
    let scope = execution.execution_scope().map_err(|error| {
        RuntimeError::InvalidArtifact(format!(
            "current execution scope is unavailable for std.time.sleep: {error}"
        ))
    })?;
    let (lease, completion) = scope.acquire_lease();
    let normal_wake = async move {
        tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
        completion.complete()
    };
    tokio::pin!(normal_wake);

    tokio::select! {
        biased;
        completed = &mut normal_wake => {
            if completed {
                Ok(())
            } else {
                Err(RuntimeError::Cancelled)
            }
        }
        terminal = lease.wait() => match terminal {
            ExecutionScopeLeaseTerminal::Control(_) => Err(RuntimeError::Cancelled),
            ExecutionScopeLeaseTerminal::Completed => {
                unreachable!("time sleep scope completion is owned by the normal wake branch")
            }
        },
    }
}

#[cfg(test)]
mod tests {
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

        fn execution_scope(
            &self,
        ) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
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

        fn execution_scope(
            &self,
        ) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
            Ok(self.scope.clone())
        }
    }

    fn scope_site() -> InstructionSourceSite {
        InstructionSourceSite::Synthetic {
            reason:
                skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
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
            sleep_millis_from_runtime_value(&RuntimeValue::Number(
                (TIME_SLEEP_MAX_MILLIS + 1) as f64,
            ))
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
}

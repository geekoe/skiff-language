use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use skiff_runtime_capability_context::{
    CancellationToken, ExecutionDeadlineSource, ExecutionScopeTerminal, OwnedExecutionControl,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        ErrorCorrelation, ExceptionStackFrame, PlatformBuiltinErrorIdentity, RequestException,
    },
};

use super::super::{
    concurrent_scheduler::{run_concurrent_scheduler, ConcurrentSchedulerResult},
    concurrent_scheduler_test_support::*,
    ConcurrentPlan, ConcurrentPlanKind, LaneCompletion, LaneEvaluation, LaneExecutionState,
};
use crate::{
    error::{RuntimeError, UserException},
    program_execution::ExecutionCheckpointKind,
};

#[tokio::test]
async fn concurrent_scheduler_outer_terminal_beats_ready_lane_error() {
    let outer = TestOuter::new();
    let cancel = outer.cancellation.clone();
    let executor = TestExecutor::new(move |lane, state| {
        if lane.source_order() == 0 {
            cancel.cancel();
        }
        boxed_lane(async move {
            LaneCompletion::error(
                state,
                RuntimeError::Decode(format!("lane-{}", lane.source_order())),
            )
        })
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
    ]);
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("outer cancellation wins");

    assert!(error.is_cancellation_terminal());
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_outer_cancel_wakes_pending_lane_scope() {
    let starts = Arc::new(AtomicUsize::new(0));
    let drop_probe = Arc::new(DropProbe::default());
    let executor = TestExecutor::new({
        let starts = starts.clone();
        let drop_probe = drop_probe.clone();
        move |_lane, state| {
            starts.fetch_add(1, Ordering::AcqRel);
            Box::pin(PendingLane::new(state, drop_probe.clone()))
        }
    });
    let plan = statement_plan(vec![statement_lane(0, vec![], None)]);
    let outer = TestOuter::new();
    let cancellation = outer.cancellation.clone();
    let parent_env = env_with_slots(0);
    let mut parent_heap = RequestHeap::default();

    let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(
            run_concurrent_scheduler(&plan, &parent_env, &mut parent_heap, &outer, &executor,),
            async move {
                while starts.load(Ordering::Acquire) == 0 {
                    tokio::task::yield_now().await;
                }
                cancellation.cancel();
            }
        )
    })
    .await
    .expect("outer cancellation wakes the pending lane scope");
    let error = result.expect_err("outer cancellation wins");

    assert!(error.is_cancellation_terminal());
    assert_eq!(drop_probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(drop_probe.cancelled_drops.load(Ordering::Acquire), 1);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_winner_cancels_running_and_blocks_new_lane() {
    let starts = Arc::new(Mutex::new(Vec::new()));
    let drop_probe = Arc::new(DropProbe::default());
    let executor = TestExecutor::new({
        let starts = starts.clone();
        let drop_probe = drop_probe.clone();
        move |lane, mut state| {
            starts.lock().unwrap().push(lane.source_order());
            match lane.source_order() {
                0 => boxed_lane(async move {
                    LaneCompletion::error(state, RuntimeError::Decode("winner".to_string()))
                }),
                1 => {
                    let late = state
                        .heap_mut()
                        .alloc_array(vec![RuntimeValue::Number(123.0)])
                        .unwrap();
                    state
                        .env_mut()
                        .declare_binding("late", Some(0), RuntimeValue::Heap(late))
                        .unwrap();
                    Box::pin(PendingLane::new(state, drop_probe.clone()))
                }
                _ => panic!("blocked lane must not start"),
            }
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
        statement_lane(2, vec![1], None),
    ]);
    let outer = TestOuter::new();
    let parent_env = env_with_slots(1);
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(&plan, &parent_env, &mut parent_heap, &outer, &executor)
        .await
        .expect_err("lane zero wins");

    assert_eq!(error.to_string(), "winner");
    assert_eq!(*starts.lock().unwrap(), vec![0, 1]);
    assert_eq!(drop_probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(drop_probe.cancelled_drops.load(Ordering::Acquire), 1);
    assert!(parent_heap.is_empty());
    assert!(parent_env.get_slot(0).is_err());
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_winner_error_materialization_rolls_back_parent_heap() {
    let executor = TestExecutor::new(|_lane, mut state| {
        boxed_lane(async move {
            let shallow = state.heap_mut().alloc_bytes(vec![1]).unwrap();
            let deep_leaf = state.heap_mut().alloc_bytes(vec![2]).unwrap();
            let deep_inner = state
                .heap_mut()
                .alloc_array(vec![RuntimeValue::Heap(deep_leaf)])
                .unwrap();
            let deep_outer = state
                .heap_mut()
                .alloc_array(vec![RuntimeValue::Heap(deep_inner)])
                .unwrap();
            let payload = state
                .heap_mut()
                .alloc_array(vec![
                    RuntimeValue::Heap(shallow),
                    RuntimeValue::Heap(deep_outer),
                ])
                .unwrap();
            let source = site();
            let request = RequestException::local(
                RuntimeValueCarrier::identified(
                    RuntimeValue::Heap(payload),
                    PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
                ),
                source.clone(),
                vec![ExceptionStackFrame::Local { site: source }],
                ErrorCorrelation {
                    trace_id: "trace-concurrent".to_string(),
                    error_id: "trace-concurrent:local-error:1".to_string(),
                },
            )
            .unwrap();
            LaneCompletion::error(
                state,
                RuntimeError::UserException(UserException::new(request)),
            )
        })
    });
    let plan = statement_plan(vec![statement_lane(0, vec![], None)]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::new(RequestHeapLimits {
        max_clone_depth: 2,
        ..RequestHeapLimits::default()
    });
    let sentinel = parent_heap.alloc_bytes(vec![9]).unwrap();
    let before_checkpoint = parent_heap.checkpoint();
    let before_stats = parent_heap.stats();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("winner local carrier exceeds the parent clone-depth limit");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error
        .to_string()
        .contains("winner error materialization failed"));
    assert_eq!(parent_heap.checkpoint(), before_checkpoint);
    assert_eq!(parent_heap.stats(), before_stats);
    assert!(parent_heap.get(sentinel).is_ok());
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_tail_fence_clones_result_into_parent_heap() {
    let starts = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let starts = starts.clone();
        move |lane, mut state| {
            starts.lock().unwrap().push(lane.source_order());
            boxed_lane(async move {
                match lane.evaluation() {
                    LaneEvaluation::Tail { .. } => {
                        let array = state
                            .heap_mut()
                            .alloc_array(vec![RuntimeValue::Number(42.0)])
                            .unwrap();
                        LaneCompletion::value(state, RuntimeValue::Heap(array).into())
                    }
                    _ => LaneCompletion::normal(state),
                }
            })
        }
    });
    let plan = ConcurrentPlan::for_test(
        ConcurrentPlanKind::Value,
        vec![statement_lane(0, vec![], None), tail_lane(1, vec![0])],
    );
    let outer = TestOuter::new();
    let parent_env = env_with_slots(0);
    let mut parent_heap = RequestHeap::default();

    let result = run_concurrent_scheduler(&plan, &parent_env, &mut parent_heap, &outer, &executor)
        .await
        .unwrap();
    let ConcurrentSchedulerResult::Value(value) = result else {
        panic!("value plan returns its tail");
    };
    let RuntimeValue::Heap(handle) = value.value() else {
        panic!("tail stays heap-backed");
    };

    assert_eq!(*starts.lock().unwrap(), vec![0, 1]);
    assert_eq!(
        parent_heap
            .array_item_carrier(*handle, 0)
            .unwrap()
            .unwrap()
            .value(),
        &RuntimeValue::Number(42.0)
    );
    assert_eq!(
        *outer.checkpoints.lock().unwrap(),
        vec![
            ExecutionCheckpointKind::LaneStart,
            ExecutionCheckpointKind::LaneEnd,
            ExecutionCheckpointKind::TailStart,
            ExecutionCheckpointKind::LaneEnd,
        ]
    );
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_drop_cancels_scope_before_future() {
    let drop_probe = Arc::new(DropProbe::default());
    let executor = TestExecutor::new({
        let drop_probe = drop_probe.clone();
        move |_lane, state| Box::pin(PendingLane::new(state, drop_probe.clone()))
    });
    let plan = statement_plan(vec![statement_lane(0, vec![], None)]);
    let outer = TestOuter::with_deadline(Instant::now() + Duration::from_secs(60));
    let parent_env = env_with_slots(0);
    let mut parent_heap = RequestHeap::default();
    let mut scheduler = Box::pin(run_concurrent_scheduler(
        &plan,
        &parent_env,
        &mut parent_heap,
        &outer,
        &executor,
    ));

    tokio::select! {
        result = &mut scheduler => panic!("pending lane unexpectedly finished: {}", result.is_ok()),
        () = tokio::task::yield_now() => {}
    }
    let active = outer.scope.lifecycle_snapshot();
    assert_eq!(active.active_leases, 1);
    assert_eq!(active.active_waiters, 1);
    assert_eq!(active.active_timers, 1);
    drop(scheduler);

    assert_eq!(drop_probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(drop_probe.cancelled_drops.load(Ordering::Acquire), 1);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_nested_scope_inherits_lane_owner_and_cancel() {
    let nested_probe = Arc::new(NestedScopeProbe::default());
    let executor = TestExecutor::new({
        let nested_probe = nested_probe.clone();
        move |lane, state| match lane.source_order() {
            0 => boxed_lane(async move {
                LaneCompletion::error(state, RuntimeError::Decode("winner".to_string()))
            }),
            1 => {
                let nested = state
                    .execution_control()
                    .derive_scope(Instant::now() + Duration::from_secs(60), site())
                    .unwrap();
                let nested_scope = nested.execution_scope().unwrap();
                assert_eq!(nested_scope.nesting(), 1);
                assert_eq!(
                    nested_scope.effective_deadline().unwrap().source(),
                    &ExecutionDeadlineSource::Scope { site: site() }
                );
                Box::pin(PendingNestedLane {
                    _state: Some(state),
                    nested,
                    probe: nested_probe.clone(),
                })
            }
            _ => unreachable!(),
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
    ]);
    let outer = TestOuter::new();
    let original_scope = outer.scope.clone();
    let mut parent_heap = RequestHeap::default();

    run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("lane zero wins");

    assert_eq!(nested_probe.drops.load(Ordering::Acquire), 1);
    assert_eq!(
        nested_probe.inherited_cancellation.load(Ordering::Acquire),
        1
    );
    assert_eq!(original_scope.nesting(), outer.scope.nesting());
    assert!(outer.scope.terminal_at(Instant::now()).is_none());
    assert_clean_scope(&outer);
}

#[derive(Default)]
struct DropProbe {
    drops: AtomicUsize,
    cancelled_drops: AtomicUsize,
}

struct PendingLane {
    _state: Option<LaneExecutionState>,
    cancellation: CancellationToken,
    probe: Arc<DropProbe>,
}

impl PendingLane {
    fn new(state: LaneExecutionState, probe: Arc<DropProbe>) -> Self {
        let cancellation = state.execution_control().cancellation_token();
        Self {
            _state: Some(state),
            cancellation,
            probe,
        }
    }
}

impl Future for PendingLane {
    type Output = LaneCompletion;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for PendingLane {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
        if self.cancellation.is_cancelled() {
            self.probe.cancelled_drops.fetch_add(1, Ordering::AcqRel);
        }
    }
}

#[derive(Default)]
struct NestedScopeProbe {
    drops: AtomicUsize,
    inherited_cancellation: AtomicUsize,
}

struct PendingNestedLane {
    _state: Option<LaneExecutionState>,
    nested: OwnedExecutionControl,
    probe: Arc<NestedScopeProbe>,
}

impl Future for PendingNestedLane {
    type Output = LaneCompletion;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for PendingNestedLane {
    fn drop(&mut self) {
        self.probe.drops.fetch_add(1, Ordering::AcqRel);
        if matches!(
            self.nested
                .execution_scope()
                .unwrap()
                .terminal_at(Instant::now()),
            Some(ExecutionScopeTerminal::AncestorCancelled)
        ) {
            self.probe
                .inherited_cancellation
                .fetch_add(1, Ordering::AcqRel);
        }
    }
}

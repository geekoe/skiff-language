use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use super::super::{
    concurrent_scheduler::run_concurrent_scheduler, concurrent_scheduler_test_support::*,
    LaneCompletion,
};
use crate::error::RuntimeError;

#[tokio::test]
async fn concurrent_scheduler_keeps_an_entire_ready_batch_live() {
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let started = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let barrier = barrier.clone();
        let started = started.clone();
        move |lane, state| {
            started.lock().unwrap().push(lane.source_order());
            let barrier = barrier.clone();
            boxed_lane(async move {
                barrier.wait().await;
                LaneCompletion::normal(state)
            })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
    ]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();
    let parent_env = env_with_slots(0);

    tokio::time::timeout(
        Duration::from_secs(1),
        run_concurrent_scheduler(&plan, &parent_env, &mut parent_heap, &outer, &executor),
    )
    .await
    .expect("a serial await would deadlock at the barrier")
    .expect("both ready lanes complete");

    assert_eq!(*started.lock().unwrap(), vec![0, 1]);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_dependency_starts_only_after_predecessor_normal() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let events = events.clone();
        move |lane, state| {
            let events = events.clone();
            boxed_lane(async move {
                events
                    .lock()
                    .unwrap()
                    .push(format!("poll-{}", lane.source_order()));
                if lane.source_order() == 0 {
                    tokio::task::yield_now().await;
                    events.lock().unwrap().push("normal-0".to_string());
                }
                LaneCompletion::normal(state)
            })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![0], None),
    ]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec!["poll-0", "normal-0", "poll-1"]
    );
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_handoff_imports_only_declared_deep_cloned_exports() {
    let inspected = Arc::new(AtomicBool::new(false));
    let executor = TestExecutor::new({
        let inspected = inspected.clone();
        move |lane, mut state| {
            let inspected = inspected.clone();
            boxed_lane(async move {
                match lane.source_order() {
                    0 => {
                        let array = state
                            .heap_mut()
                            .alloc_array(vec![RuntimeValue::Number(7.0)])
                            .unwrap();
                        state
                            .env_mut()
                            .declare_binding("export", Some(0), RuntimeValue::Heap(array))
                            .unwrap();
                        state
                            .env_mut()
                            .declare_binding("temporary", Some(2), RuntimeValue::Number(99.0))
                            .unwrap();
                    }
                    1 => {
                        state
                            .env_mut()
                            .declare_binding("sibling", Some(1), RuntimeValue::Number(11.0))
                            .unwrap();
                    }
                    2 => {
                        let carrier = state.env().get_slot(0).expect("declared export");
                        let RuntimeValue::Heap(handle) = carrier.value() else {
                            panic!("export stays heap-backed");
                        };
                        assert_eq!(
                            state
                                .heap()
                                .array_item_carrier(*handle, 0)
                                .unwrap()
                                .unwrap()
                                .value(),
                            &RuntimeValue::Number(7.0)
                        );
                        assert!(state.env().get_slot(1).is_err());
                        assert!(state.env().get_slot(2).is_err());
                        inspected.store(true, Ordering::Release);
                    }
                    _ => unreachable!(),
                }
                LaneCompletion::normal(state)
            })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], Some(0)),
        statement_lane(1, vec![], Some(1)),
        statement_lane(2, vec![0], None),
    ]);
    let outer = TestOuter::new();
    let parent_env = env_with_slots(3);
    let mut parent_heap = RequestHeap::default();

    run_concurrent_scheduler(&plan, &parent_env, &mut parent_heap, &outer, &executor)
        .await
        .unwrap();

    assert!(inspected.load(Ordering::Acquire));
    assert!(parent_env.get_slot(0).is_err());
    assert!(parent_env.get_slot(1).is_err());
    assert!(parent_env.get_slot(2).is_err());
    assert!(parent_heap.is_empty());
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_same_turn_errors_choose_lowest_source_order() {
    let executor = TestExecutor::new(|lane, state| {
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
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("both lanes fail");

    assert_eq!(error.to_string(), "lane-0");
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_winner_discards_same_turn_late_value() {
    let executor = TestExecutor::new(|lane, mut state| {
        boxed_lane(async move {
            if lane.source_order() == 0 {
                LaneCompletion::error(state, RuntimeError::Decode("winner".to_string()))
            } else {
                let late = state
                    .heap_mut()
                    .alloc_array(vec![RuntimeValue::Number(88.0)])
                    .unwrap();
                LaneCompletion::value(state, RuntimeValue::Heap(late).into())
            }
        })
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], None),
        statement_lane(1, vec![], None),
    ]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(0),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("the source-order winner discards the same-turn late value");

    assert_eq!(error.to_string(), "winner");
    assert!(parent_heap.is_empty());
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_missing_direct_let_export_fails_closed() {
    let starts = Arc::new(AtomicUsize::new(0));
    let executor = TestExecutor::new({
        let starts = starts.clone();
        move |_lane, state| {
            starts.fetch_add(1, Ordering::Relaxed);
            boxed_lane(async move { LaneCompletion::normal(state) })
        }
    });
    let plan = statement_plan(vec![statement_lane(0, vec![], Some(0))]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(1),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("a direct let lane must publish its projected export");

    assert!(error.to_string().contains("without export slot 0"));
    assert_eq!(starts.load(Ordering::Relaxed), 1);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_rejects_duplicate_slot_import_before_consumer_start() {
    let starts = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let starts = starts.clone();
        move |lane, mut state| {
            starts.lock().unwrap().push(lane.source_order());
            boxed_lane(async move {
                state
                    .env_mut()
                    .declare_binding(
                        "same-slot",
                        Some(0),
                        RuntimeValue::Number(lane.source_order() as f64),
                    )
                    .unwrap();
                LaneCompletion::normal(state)
            })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], Some(0)),
        statement_lane(1, vec![], Some(0)),
        statement_lane(2, vec![0, 1], None),
    ]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(1),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("two dependencies cannot import the same destination slot");

    assert!(error.to_string().contains("repeats destination slot 0"));
    assert_eq!(*starts.lock().unwrap(), vec![0, 1]);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_rejects_stale_cross_heap_export_handle() {
    let starts = Arc::new(Mutex::new(Vec::new()));
    let executor = TestExecutor::new({
        let starts = starts.clone();
        move |lane, mut state| {
            starts.lock().unwrap().push(lane.source_order());
            boxed_lane(async move {
                if lane.source_order() == 0 {
                    let checkpoint = state.heap().checkpoint();
                    let stale = state
                        .heap_mut()
                        .alloc_array(vec![RuntimeValue::Null])
                        .unwrap();
                    state.heap_mut().rollback_to_checkpoint(checkpoint);
                    state
                        .env_mut()
                        .declare_binding("stale", Some(0), RuntimeValue::Heap(stale))
                        .unwrap();
                }
                LaneCompletion::normal(state)
            })
        }
    });
    let plan = statement_plan(vec![
        statement_lane(0, vec![], Some(0)),
        statement_lane(1, vec![0], None),
    ]);
    let outer = TestOuter::new();
    let mut parent_heap = RequestHeap::default();

    let error = run_concurrent_scheduler(
        &plan,
        &env_with_slots(1),
        &mut parent_heap,
        &outer,
        &executor,
    )
    .await
    .expect_err("stale cross-heap carrier must fail closed");

    assert!(error.to_string().contains("carrier clone failed"));
    assert_eq!(*starts.lock().unwrap(), vec![0]);
    assert_clean_scope(&outer);
}

#[tokio::test]
async fn concurrent_scheduler_malformed_projection_starts_zero_lanes() {
    let starts = Arc::new(AtomicUsize::new(0));
    let executor = TestExecutor::new({
        let starts = starts.clone();
        move |_lane, state| {
            starts.fetch_add(1, Ordering::Relaxed);
            boxed_lane(async move { LaneCompletion::normal(state) })
        }
    });
    let malformed = [
        statement_plan(vec![statement_lane(0, vec![0], None)]),
        statement_plan(vec![tail_lane(0, vec![])]),
    ];

    for plan in malformed {
        let outer = TestOuter::new();
        let mut parent_heap = RequestHeap::default();
        let error = run_concurrent_scheduler(
            &plan,
            &env_with_slots(0),
            &mut parent_heap,
            &outer,
            &executor,
        )
        .await
        .expect_err("dependency and tail shapes fail before lane start");

        assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
        assert_clean_scope(&outer);
    }
    assert_eq!(starts.load(Ordering::Relaxed), 0);
}

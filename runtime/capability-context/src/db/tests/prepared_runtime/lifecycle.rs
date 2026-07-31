use serde_json::json;
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};
use tokio::sync::oneshot;

use super::fake_store::{prepared_store, runtime_context, wait_until_started};
use crate::db::{DbKey, DbOneSelector};

#[tokio::test]
async fn prepared_db_pending_wait_releases_caller_heap_until_finalize() {
    let (gate_tx, gate_rx) = oneshot::channel();
    let (store, state) = prepared_store(Some(gate_rx));
    let mut heap = RequestHeap::default();
    let input_handle = heap
        .alloc_array(vec![RuntimeValue::String("input".to_string())])
        .expect("input allocation");
    let input = RuntimeValue::Heap(input_handle);

    let prepared = store
        .prepare_create_runtime("Item", &input, &mut heap, runtime_context())
        .expect("prepare create");
    let independent_handle = heap
        .alloc_array(vec![RuntimeValue::String("independent".to_string())])
        .expect("caller heap must be independently mutable after prepare");
    let checkpoint_before_wait = heap.checkpoint();
    let stats_before_wait = heap.stats();
    let len_before_wait = heap.len();

    let wait_task = tokio::spawn(prepared.into_wait());
    wait_until_started(&state, 1).await;
    assert_eq!(heap.checkpoint(), checkpoint_before_wait);
    assert_eq!(heap.stats(), stats_before_wait);
    assert_eq!(heap.len(), len_before_wait);
    heap.get(input_handle).expect("input node remains");
    heap.get(independent_handle)
        .expect("independent caller mutation remains");

    gate_tx.send(()).expect("release prepared wait");
    let completion = wait_task
        .await
        .expect("wait task joins")
        .expect("prepared wait succeeds");
    assert_eq!(state.wait_starts(), 1);
    assert_eq!(heap.checkpoint(), checkpoint_before_wait);
    assert_eq!(heap.stats(), stats_before_wait);
    assert_eq!(heap.len(), len_before_wait);

    let value = completion.finalize(&mut heap).expect("finalize create");
    assert!(matches!(value, RuntimeValue::Heap(_)));
    assert_eq!(heap.len(), len_before_wait + 1);
    assert_eq!(state.finalize_calls(), 1);
}

#[tokio::test]
async fn prepared_db_ready_and_pending_waits_start_once() {
    let (ready_store, ready_state) = prepared_store(None);
    let mut ready_heap = RequestHeap::default();
    let ready = ready_store
        .prepare_find_one_by_key_runtime(
            "Item",
            DbKey::new(json!("one")),
            None,
            &mut ready_heap,
            runtime_context(),
        )
        .expect("prepare ready find");
    let ready_completion = ready.into_wait().await.expect("ready wait");
    assert_eq!(ready_state.wait_starts(), 1);
    let ready_value = ready_completion
        .finalize(&mut ready_heap)
        .expect("ready finalize");
    assert_eq!(ready_value, Some(RuntimeValue::String("key".to_string())));

    let (gate_tx, gate_rx) = oneshot::channel();
    let (pending_store, pending_state) = prepared_store(Some(gate_rx));
    let mut pending_heap = RequestHeap::default();
    let pending = pending_store
        .prepare_create_runtime(
            "Item",
            &RuntimeValue::Null,
            &mut pending_heap,
            runtime_context(),
        )
        .expect("prepare pending create");
    let pending_task = tokio::spawn(pending.into_wait());
    wait_until_started(&pending_state, 1).await;
    gate_tx.send(()).expect("release pending wait");
    pending_task
        .await
        .expect("pending task joins")
        .expect("pending wait succeeds")
        .finalize(&mut pending_heap)
        .expect("pending finalize");
    assert_eq!(pending_state.wait_starts(), 1);
}

#[tokio::test]
async fn prepared_db_drop_and_error_do_not_restart_wait_or_finalize() {
    let (gate_tx, gate_rx) = oneshot::channel();
    let (drop_store, drop_state) = prepared_store(Some(gate_rx));
    let mut drop_heap = RequestHeap::default();
    let prepared = drop_store
        .prepare_create_runtime(
            "Item",
            &RuntimeValue::Null,
            &mut drop_heap,
            runtime_context(),
        )
        .expect("prepare dropped wait");
    let wait_task = tokio::spawn(prepared.into_wait());
    wait_until_started(&drop_state, 1).await;
    wait_task.abort();
    let _ = wait_task.await;
    assert!(gate_tx.send(()).is_err(), "dropped wait owns its gate");
    assert_eq!(drop_state.wait_starts(), 1);
    assert_eq!(drop_state.finalize_calls(), 0);

    let (error_store, error_state) = prepared_store(None);
    error_state.set_replace_wait_fails(true);
    let mut error_heap = RequestHeap::default();
    let error = error_store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("one")),
            &RuntimeValue::Null,
            &mut error_heap,
            runtime_context(),
        )
        .expect("prepare failing replace")
        .into_wait()
        .await
        .err()
        .expect("wait must fail");
    assert_eq!(error.to_string(), "prepared replace failed");
    assert_eq!(error_state.wait_starts(), 1);
    assert_eq!(error_state.finalize_calls(), 0);

    error_state.set_replace_wait_fails(false);
    let completion = error_store
        .prepare_replace_one_runtime(
            "Item",
            DbOneSelector::key(json!("two")),
            &RuntimeValue::Null,
            &mut error_heap,
            runtime_context(),
        )
        .expect("prepare replace")
        .into_wait()
        .await
        .expect("replace wait");
    let finalize_count = error_state.finalize_calls();
    drop(completion);
    assert_eq!(
        error_state.finalize_calls(),
        finalize_count,
        "dropping the one-shot completion must not run it"
    );
    assert_eq!(error_state.wait_starts(), 2);
}

#[tokio::test]
async fn prepared_db_finalize_resource_failure_rolls_back_partial_allocations() {
    let (store, state) = prepared_store(None);
    state.set_create_finalize_fails(true);
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 2,
        ..RequestHeapLimits::default()
    });
    let baseline_handle = heap
        .alloc_array(vec![RuntimeValue::String("baseline".to_string())])
        .expect("baseline allocation");
    let baseline = RuntimeValue::Heap(baseline_handle);
    let completion = store
        .prepare_create_runtime("Item", &baseline, &mut heap, runtime_context())
        .expect("prepare create")
        .into_wait()
        .await
        .expect("wait succeeds before finalization");
    let checkpoint = heap.checkpoint();
    let stats = heap.stats();

    let error = completion
        .finalize(&mut heap)
        .expect_err("second finalizer allocation must exceed max_nodes");
    assert!(error.to_string().contains("max heap nodes"), "{error}");
    assert_eq!(heap.checkpoint(), checkpoint);
    assert_eq!(heap.stats(), stats);
    heap.get(baseline_handle)
        .expect("pre-existing node must survive rollback");
    assert_eq!(state.finalize_calls(), 1);
}

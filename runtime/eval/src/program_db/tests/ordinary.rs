use crate::heap_access::HeapAccess;
use std::{pin::pin, task::Poll};

use serde_json::json;
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_model::runtime_value::{RuntimeObject, RuntimeValue};

use super::fixture::{
    db_error, first_poll, DbActorFixture, DbPhase, FakeDbState, OperationMetrics, PreparedFinalize,
};
use crate::env::Env;

fn raw_document(id: &str) -> DbDocument {
    DbDocument::new(json!({ "id": id }))
}

fn prepared_heap_value() -> PreparedFinalize {
    PreparedFinalize::new(|heap| {
        let handle = heap
            .alloc_object(RuntimeObject::unshaped(Default::default()))
            .map_err(|error| db_error(error.to_string()))?;
        Ok(RuntimeValue::Heap(handle))
    })
}

fn assert_ready_once(state: &FakeDbState, phase: DbPhase) {
    state.assert_completed_once(phase);
}

fn assert_pending_then_ready_once(state: &FakeDbState, phase: DbPhase) {
    assert_eq!(
        state.metrics(phase),
        OperationMetrics {
            constructed: 1,
            polls: 3,
            pending_returns: 2,
            ready_returns: 1,
            dropped_before_terminal: 0,
            dropped_after_terminal: 1,
        }
    );
}

fn assert_dropped_pending_once(state: &FakeDbState, phase: DbPhase) {
    assert_eq!(
        state.metrics(phase),
        OperationMetrics {
            constructed: 1,
            polls: 2,
            pending_returns: 2,
            ready_returns: 0,
            dropped_before_terminal: 1,
            dropped_after_terminal: 0,
        }
    );
}

async fn assert_actor_held(fixture: &DbActorFixture) {
    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));
}

#[tokio::test]
async fn db_actor_ordinary_query_ready_keeps_segment_and_does_not_touch_store() {
    let state = FakeDbState::new();
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_query_value(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.query_target,
            &fixture.linked.query,
            None,
        )
        .await
        .expect("pure query must evaluate");

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_eq!(state.context_require_calls(), 0);
    assert!(state.phases().is_empty());
}

#[tokio::test]
async fn db_actor_ordinary_raw_create_ready_once_keeps_segment() {
    let state = FakeDbState::new();
    state.raw_create.push_ready(Ok(raw_document("raw-ready")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.raw_create,
        )
        .await
        .expect("raw create must succeed");

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::RawCreate);
    assert_eq!(state.context_require_calls(), 1);
}

#[tokio::test]
async fn db_actor_ordinary_raw_create_pending_releases_and_reacquires_segment() {
    let state = FakeDbState::new();
    let gate = state
        .raw_create
        .push_pending(Ok(raw_document("raw-pending")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut competing = Box::pin(fixture.actor.competing_acquire());
    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.raw_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    let mut competing_lease = competing
        .await
        .expect("pending DB operation must release the Actor segment");
    let pending_heap_len = competing_lease.heap_mut().len();
    assert!(!gate.is_released());
    drop(competing_lease);
    gate.release();
    let result = eval.as_mut().await.expect("raw create must resume");
    drop(eval);

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert!(
        heap.len() > pending_heap_len,
        "raw result must materialize only after the pending operation resumes"
    );
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::RawCreate);
}

#[tokio::test]
async fn db_actor_ordinary_raw_create_ready_error_is_not_rebuilt() {
    let state = FakeDbState::new();
    state
        .raw_create
        .push_ready(Err(db_error("raw ready failure")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.raw_create,
        )
        .await;

    assert!(result.is_err());
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::RawCreate);
}

#[tokio::test]
async fn db_actor_ordinary_raw_create_pending_error_is_not_rebuilt() {
    let state = FakeDbState::new();
    let gate = state
        .raw_create
        .push_pending(Err(db_error("raw pending failure")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.raw_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    let competing_lease = fixture.actor.competing_acquire().await.unwrap();
    drop(competing_lease);
    gate.release();
    let result = eval.as_mut().await;
    drop(eval);

    assert!(result.is_err());
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::RawCreate);
}

#[tokio::test]
async fn db_actor_ordinary_raw_create_pending_drop_drops_only_same_future() {
    let state = FakeDbState::new();
    let gate = state
        .raw_create
        .push_pending(Ok(raw_document("must-not-materialize")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.raw_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    let competing_lease = fixture.actor.competing_acquire().await.unwrap();
    drop(eval);

    assert!(!gate.is_released());
    assert_dropped_pending_once(&state, DbPhase::RawCreate);
    drop(competing_lease);
    drop(frame);
    drop(heap);
}

#[tokio::test]
async fn db_actor_ordinary_prepared_create_ready_wait_and_finalizer_once() {
    let state = FakeDbState::new();
    state.prepared_create.push_ready(Ok(prepared_heap_value()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.prepared_create,
        )
        .await
        .expect("prepared create must succeed");

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::PreparedCreateWait);
    assert_ready_once(&state, DbPhase::PreparedCreateFinalize);
    assert_eq!(state.legacy_runtime_calls(), 0);
}

#[tokio::test]
async fn db_actor_ordinary_prepared_create_pending_finalizes_only_after_resume() {
    let state = FakeDbState::new();
    let gate = state
        .prepared_create
        .push_pending(Ok(prepared_heap_value()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut competing = Box::pin(fixture.actor.competing_acquire());
    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.prepared_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    assert_eq!(
        state.metrics(DbPhase::PreparedCreateFinalize),
        OperationMetrics::default()
    );
    let mut competing_lease = competing.await.unwrap();
    let pending_heap_len = competing_lease.heap_mut().len();
    drop(competing_lease);
    gate.release();
    let result = eval.as_mut().await.expect("prepared create must resume");
    drop(eval);

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert!(
        heap.len() > pending_heap_len,
        "prepared finalizer must materialize only after the wait resumes"
    );
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::PreparedCreateWait);
    assert_ready_once(&state, DbPhase::PreparedCreateFinalize);
}

#[tokio::test]
async fn db_actor_ordinary_prepared_wait_ready_error_is_not_replayed() {
    let state = FakeDbState::new();
    state
        .prepared_create
        .push_ready(Err(db_error("prepared ready failure")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.prepared_create,
        )
        .await;

    assert!(result.is_err());
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::PreparedCreateWait);
    assert_eq!(
        state.metrics(DbPhase::PreparedCreateFinalize),
        OperationMetrics::default()
    );
}

#[tokio::test]
async fn db_actor_ordinary_prepared_wait_pending_error_is_not_replayed() {
    let state = FakeDbState::new();
    let gate = state
        .prepared_create
        .push_pending(Err(db_error("prepared pending failure")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.prepared_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    let competing_lease = fixture.actor.competing_acquire().await.unwrap();
    drop(competing_lease);
    gate.release();
    let result = eval.as_mut().await;
    drop(eval);

    assert!(result.is_err());
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::PreparedCreateWait);
    assert_eq!(
        state.metrics(DbPhase::PreparedCreateFinalize),
        OperationMetrics::default()
    );
}

#[tokio::test]
async fn db_actor_ordinary_prepared_finalizer_error_is_not_replayed() {
    let state = FakeDbState::new();
    state
        .prepared_create
        .push_ready(Ok(PreparedFinalize::error("finalizer failure")));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let result = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            fixture.context(frame.clone()),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.prepared_create,
        )
        .await;

    assert!(result.is_err());
    assert_actor_held(&fixture).await;
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::PreparedCreateWait);
    assert_ready_once(&state, DbPhase::PreparedCreateFinalize);
}

#[tokio::test]
async fn db_actor_ordinary_prepared_pending_drop_does_not_finalize_or_rebuild() {
    let state = FakeDbState::new();
    let gate = state
        .prepared_create
        .push_pending(Ok(prepared_heap_value()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());
    let mut env = Env::new();
    let mut access = HeapAccess::Exclusive(&mut heap);
    let mut eval = Box::pin(fixture.linked.interpreter.eval_program_db_operation(
        context,
        &mut access,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.prepared_create,
    ));
    assert!(matches!(first_poll(eval.as_mut()), Poll::Pending));
    let competing_lease = fixture.actor.competing_acquire().await.unwrap();
    drop(eval);

    assert!(!gate.is_released());
    assert_dropped_pending_once(&state, DbPhase::PreparedCreateWait);
    assert_eq!(
        state.metrics(DbPhase::PreparedCreateFinalize),
        OperationMetrics::default()
    );
    drop(competing_lease);
    drop(frame);
    drop(heap);
}

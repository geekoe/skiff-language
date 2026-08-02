use crate::heap_access::HeapAccess;
use std::fmt;

use serde_json::json;
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_linked_program::{CallIr, DbTransactionIr, ExprRefIr};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapCheckpoint},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
};

use super::fixture::{
    db_error, DbActorFixture, DbEventKind, DbPhase, FakeDbState, OperationMetrics,
    BODY_CREATE_BLOCK_LABEL,
};
use crate::{
    env::Env,
    error::{Result, RuntimeError},
    runtime_ops::runtime_member_access_carrier,
};

#[derive(Clone, Copy, Debug)]
enum TransactionSource {
    Legacy,
    Explicit,
}

impl fmt::Display for TransactionSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Legacy => formatter.write_str("legacy"),
            Self::Explicit => formatter.write_str("explicit"),
        }
    }
}

const SOURCES: [TransactionSource; 2] = [TransactionSource::Legacy, TransactionSource::Explicit];

fn body_document(case: &str) -> DbDocument {
    DbDocument::new(json!({ "id": case }))
}

fn raw_create_expression(fixture: &DbActorFixture) -> ExprRefIr {
    let expression = fixture
        .linked
        .executable()
        .body
        .expressions
        .iter()
        .position(|expression| {
            matches!(
                expression,
                skiff_runtime_linked_program::LinkedExprIr::DbOperation { operation }
                    if operation == &fixture.linked.raw_create
            )
        })
        .expect("shared fixture raw-create expression");
    ExprRefIr {
        expression: u32::try_from(expression).expect("fixture expression index"),
    }
}

fn legacy_create_call(fixture: &DbActorFixture) -> CallIr {
    let mut call = fixture.linked.legacy_transaction.clone();
    call.args = vec![raw_create_expression(fixture)];
    call
}

fn explicit_create_transaction(fixture: &DbActorFixture) -> DbTransactionIr {
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.body = BODY_CREATE_BLOCK_LABEL.to_string();
    transaction
}

async fn evaluate_transaction(
    source: TransactionSource,
    fixture: &DbActorFixture,
    heap: &mut RequestHeap,
    env: &mut Env,
) -> Result<RuntimeValue> {
    let context = fixture.ordinary_context();
    match source {
        TransactionSource::Legacy => {
            let db_context = context.db_context();
            fixture
                .linked
                .interpreter
                .eval_program_db_transaction(
                    &db_context,
                    context,
                    &mut HeapAccess::Exclusive(&mut *heap),
                    env,
                    &fixture.linked.addr,
                    &fixture.linked.file,
                    fixture.linked.executable(),
                    &legacy_create_call(fixture),
                )
                .await
        }
        TransactionSource::Explicit => fixture
            .linked
            .interpreter
            .eval_program_explicit_db_transaction(
                context,
                &mut HeapAccess::Exclusive(&mut *heap),
                env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &explicit_create_transaction(fixture),
            )
            .await
            .map(RuntimeValueCarrier::into_value),
    }
}

fn assert_success_value(source: TransactionSource, value: RuntimeValue) {
    match source {
        TransactionSource::Legacy => {
            assert!(
                matches!(value, RuntimeValue::Heap(_)),
                "legacy value={value:?}"
            );
        }
        TransactionSource::Explicit => {
            assert_eq!(value, RuntimeValue::Null, "explicit value");
        }
    }
}

fn assert_db_error(result: Result<RuntimeValue>, expected_message: &str) {
    match result.expect_err("transaction DB case must fail") {
        RuntimeError::DbDecode { target, message } => {
            assert_eq!(target, "std.db");
            assert_eq!(message, expected_message);
        }
        error => panic!("expected preserved DB decode error, got {error:?}"),
    }
}

fn phase_event_kinds(state: &FakeDbState, phase: DbPhase) -> Vec<DbEventKind> {
    state
        .events()
        .into_iter()
        .filter_map(|event| (event.phase == phase).then_some(event.kind))
        .collect()
}

fn assert_ready_phase(state: &FakeDbState, phase: DbPhase) {
    assert_eq!(
        state.metrics(phase),
        OperationMetrics {
            constructed: 1,
            polls: 1,
            pending_returns: 0,
            ready_returns: 1,
            dropped_before_terminal: 0,
            dropped_after_terminal: 1,
        },
        "unexpected metrics for {phase:?}"
    );
    assert_eq!(
        phase_event_kinds(state, phase),
        vec![
            DbEventKind::Constructed,
            DbEventKind::Poll,
            DbEventKind::Ready,
            DbEventKind::DropAfterTerminal,
        ],
        "unexpected trace for {phase:?}"
    );
}

fn assert_transaction_trace(state: &FakeDbState, expected: &[DbPhase]) {
    assert_eq!(
        state.phases(),
        expected.to_vec(),
        "transaction construction order"
    );
    for phase in [
        DbPhase::Begin,
        DbPhase::BodyCreate,
        DbPhase::Commit,
        DbPhase::Abort,
    ] {
        if expected.contains(&phase) {
            assert_ready_phase(state, phase);
        } else {
            assert_eq!(
                state.metrics(phase),
                OperationMetrics::default(),
                "forbidden {phase:?} metrics"
            );
            assert!(
                phase_event_kinds(state, phase).is_empty(),
                "forbidden {phase:?} trace"
            );
        }
    }
}

fn assert_heap_rolled_back(heap: &RequestHeap, checkpoint: RequestHeapCheckpoint, case: &str) {
    assert_eq!(heap.checkpoint(), checkpoint, "{case} heap rollback");
}

fn assert_heap_retained_body(heap: &RequestHeap, checkpoint: RequestHeapCheckpoint, case: &str) {
    assert_ne!(
        heap.checkpoint(),
        checkpoint,
        "{case} must retain the successful body allocation"
    );
}

#[tokio::test]
async fn ordinary_transaction_ready_success_matrix() {
    for source in SOURCES {
        let state = FakeDbState::new();
        state.begin.push_ready(Ok(()));
        state
            .body_create
            .push_ready(Ok(body_document("ready-success")));
        state.commit.push_ready(Ok(()));
        let fixture = DbActorFixture::new(state.clone());
        let (_, mut heap) = fixture.actor.execution_frame().await;
        let checkpoint = heap.checkpoint();
        let mut env = Env::new();

        let result = evaluate_transaction(
            source,
            &fixture,
            &mut HeapAccess::Exclusive(&mut heap),
            &mut env,
        )
        .await
        .expect("Ready transaction must succeed");

        assert_success_value(source, result);
        assert_heap_retained_body(&heap, checkpoint, "Ready success");
        assert_transaction_trace(
            &state,
            &[DbPhase::Begin, DbPhase::BodyCreate, DbPhase::Commit],
        );
    }
}

#[tokio::test]
async fn ordinary_transaction_body_error_aborts_once_and_rolls_back() {
    for source in SOURCES {
        let state = FakeDbState::new();
        let error_message = format!("{source}-body-error");
        state.begin.push_ready(Ok(()));
        state.body_create.push_ready(Err(db_error(&error_message)));
        state.abort.push_ready(Ok(()));
        let fixture = DbActorFixture::new(state.clone());
        let (_, mut heap) = fixture.actor.execution_frame().await;
        let checkpoint = heap.checkpoint();
        let mut env = Env::new();

        let result = evaluate_transaction(
            source,
            &fixture,
            &mut HeapAccess::Exclusive(&mut heap),
            &mut env,
        )
        .await;

        assert_db_error(result, &error_message);
        assert_heap_rolled_back(&heap, checkpoint, "Body error");
        assert_transaction_trace(
            &state,
            &[DbPhase::Begin, DbPhase::BodyCreate, DbPhase::Abort],
        );
    }
}

#[tokio::test]
async fn ordinary_transaction_commit_error_aborts_once_and_rolls_back() {
    for source in SOURCES {
        let state = FakeDbState::new();
        let error_message = format!("{source}-commit-error");
        state.begin.push_ready(Ok(()));
        state
            .body_create
            .push_ready(Ok(body_document("commit-error-body")));
        state.commit.push_ready(Err(db_error(&error_message)));
        state.abort.push_ready(Ok(()));
        let fixture = DbActorFixture::new(state.clone());
        let (_, mut heap) = fixture.actor.execution_frame().await;
        let checkpoint = heap.checkpoint();
        let mut env = Env::new();

        let result = evaluate_transaction(
            source,
            &fixture,
            &mut HeapAccess::Exclusive(&mut heap),
            &mut env,
        )
        .await;

        assert_db_error(result, &error_message);
        assert_heap_rolled_back(&heap, checkpoint, "Commit error");
        assert_transaction_trace(
            &state,
            &[
                DbPhase::Begin,
                DbPhase::BodyCreate,
                DbPhase::Commit,
                DbPhase::Abort,
            ],
        );
    }
}

#[tokio::test]
async fn ordinary_transaction_rollback_preserves_nested_nominal_throw_for_outer_catch() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (_, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let initial_heap_len = heap.len();
    let mut env = Env::new();

    let exception = fixture
        .linked
        .interpreter
        .eval_program_expr_ref(
            fixture.ordinary_context_with_trace("trace:transaction-rollback-catch"),
            &mut HeapAccess::Exclusive(&mut heap),
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            fixture.linked.rollback_catch_exception,
        )
        .await
        .expect("outer catch must expose the preserved request-local exception");
    let payload = runtime_member_access_carrier(&exception, "error", &heap)
        .expect("caught exception payload remains readable");
    let nested = runtime_member_access_carrier(&payload, "nested", &heap)
        .expect("caught nominal payload retains its nested field");
    let value = runtime_member_access_carrier(&nested, "message", &heap)
        .expect("nested nominal payload remains readable after rollback");

    assert_eq!(
        value.into_value(),
        RuntimeValue::String("nested-survives-rollback".to_string())
    );
    assert_eq!(
        heap.len(),
        initial_heap_len + 4,
        "rollback keeps only the two reachable payload objects plus CatchResult and Exception; \
         the transaction-local array must be removed"
    );
    assert_ne!(
        heap.checkpoint(),
        checkpoint,
        "the outer catch result remains request-local after rollback"
    );
    assert_transaction_trace(&state, &[DbPhase::Begin, DbPhase::Abort]);
}

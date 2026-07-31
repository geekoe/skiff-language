use std::{fmt, pin::pin, task::Poll};

use serde_json::json;
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_linked_program::{CallIr, DbTransactionIr, ExprRefIr, LinkedExprIr};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapCheckpoint},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
};

use super::fixture::{
    db_error, first_poll, DbActorFixture, DbEventKind, DbPhase, FakeDbState, OperationMetrics,
    BODY_CREATE_BLOCK_LABEL, ILLEGAL_FLOW_BLOCK_LABEL, TAIL_CALL_BARRIER_BLOCK_LABEL,
};
use crate::{
    actor_executor::ActorExecutionFrame,
    env::Env,
    error::{Result, RuntimeError},
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

#[derive(Clone, Copy, Debug)]
enum TransactionBody {
    Create,
    IllegalFlow,
}

#[derive(Clone, Copy, Debug)]
enum PhaseExpectation {
    Ready,
    PendingThenReady,
    PendingThenDrop,
}

const SOURCES: [TransactionSource; 2] = [TransactionSource::Legacy, TransactionSource::Explicit];
const TRANSACTION_PHASES: [DbPhase; 4] = [
    DbPhase::Begin,
    DbPhase::BodyCreate,
    DbPhase::Commit,
    DbPhase::Abort,
];

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
                LinkedExprIr::DbOperation { operation }
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

fn explicit_transaction(fixture: &DbActorFixture, body: TransactionBody) -> DbTransactionIr {
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.body = match body {
        TransactionBody::Create => BODY_CREATE_BLOCK_LABEL,
        TransactionBody::IllegalFlow => ILLEGAL_FLOW_BLOCK_LABEL,
    }
    .to_string();
    transaction
}

async fn evaluate_transaction(
    source: TransactionSource,
    body: TransactionBody,
    fixture: &DbActorFixture,
    frame: ActorExecutionFrame,
    heap: &mut RequestHeap,
    env: &mut Env,
) -> Result<RuntimeValue> {
    let context = fixture.context(frame);
    match source {
        TransactionSource::Legacy => {
            assert!(
                matches!(body, TransactionBody::Create),
                "legacy fixture only supports the raw-create transaction body"
            );
            let db_context = context.db_context();
            fixture
                .linked
                .interpreter
                .eval_program_db_transaction(
                    &db_context,
                    context,
                    heap,
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
                heap,
                env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &explicit_transaction(fixture, body),
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

fn assert_error_text(result: Result<RuntimeValue>, expected: &str) {
    let error = result.expect_err("transaction case must fail");
    assert_eq!(error.to_string(), expected);
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
        "unexpected first-Ready metrics for {phase:?}"
    );
    assert_eq!(
        phase_event_kinds(state, phase),
        vec![
            DbEventKind::Constructed,
            DbEventKind::Poll,
            DbEventKind::Ready,
            DbEventKind::DropAfterTerminal,
        ],
        "unexpected first-Ready trace for {phase:?}"
    );
}

fn assert_pending_completed_phase(state: &FakeDbState, phase: DbPhase) {
    let metrics = state.metrics(phase);
    assert_eq!(metrics.constructed, 1, "{phase:?} constructed");
    assert!(metrics.pending_returns > 0, "{phase:?} must return Pending");
    assert_eq!(metrics.ready_returns, 1, "{phase:?} Ready terminal");
    assert_eq!(metrics.dropped_before_terminal, 0, "{phase:?} early drop");
    assert_eq!(metrics.dropped_after_terminal, 1, "{phase:?} terminal drop");
    assert_eq!(
        metrics.polls,
        metrics.pending_returns + metrics.ready_returns,
        "{phase:?} poll accounting"
    );

    let trace = phase_event_kinds(state, phase);
    assert_eq!(trace.first(), Some(&DbEventKind::Constructed));
    assert_eq!(trace.last(), Some(&DbEventKind::DropAfterTerminal));
    let polls = &trace[1..trace.len() - 1];
    assert!(
        polls.len() >= 4,
        "{phase:?} trace must contain Pending then Ready: {trace:?}"
    );
    assert_eq!(
        &polls[polls.len() - 2..],
        &[DbEventKind::Poll, DbEventKind::Ready],
        "{phase:?} terminal trace"
    );
    let pending = &polls[..polls.len() - 2];
    assert!(!pending.is_empty(), "{phase:?} Pending trace");
    assert_eq!(pending.len() % 2, 0, "{phase:?} Pending pairs");
    for pair in pending.chunks_exact(2) {
        assert_eq!(
            pair,
            &[DbEventKind::Poll, DbEventKind::Pending],
            "{phase:?} Pending trace"
        );
    }
}

fn assert_pending_dropped_phase(state: &FakeDbState, phase: DbPhase) {
    let metrics = state.metrics(phase);
    assert_eq!(metrics.constructed, 1, "{phase:?} constructed");
    assert!(metrics.pending_returns > 0, "{phase:?} must return Pending");
    assert_eq!(metrics.ready_returns, 0, "{phase:?} Ready terminal");
    assert_eq!(metrics.dropped_before_terminal, 1, "{phase:?} early drop");
    assert_eq!(metrics.dropped_after_terminal, 0, "{phase:?} terminal drop");
    assert_eq!(metrics.polls, metrics.pending_returns, "{phase:?} polls");

    let trace = phase_event_kinds(state, phase);
    assert_eq!(trace.first(), Some(&DbEventKind::Constructed));
    assert_eq!(trace.last(), Some(&DbEventKind::DropBeforeTerminal));
    let pending = &trace[1..trace.len() - 1];
    assert!(!pending.is_empty(), "{phase:?} Pending trace");
    assert_eq!(pending.len() % 2, 0, "{phase:?} Pending pairs");
    for pair in pending.chunks_exact(2) {
        assert_eq!(
            pair,
            &[DbEventKind::Poll, DbEventKind::Pending],
            "{phase:?} Pending trace"
        );
    }
}

fn assert_transaction_trace(state: &FakeDbState, expected: &[(DbPhase, PhaseExpectation)]) {
    assert_eq!(
        state.phases(),
        expected.iter().map(|(phase, _)| *phase).collect::<Vec<_>>(),
        "transaction construction order"
    );
    for phase in TRANSACTION_PHASES {
        match expected
            .iter()
            .find_map(|(candidate, expectation)| (*candidate == phase).then_some(*expectation))
        {
            Some(PhaseExpectation::Ready) => assert_ready_phase(state, phase),
            Some(PhaseExpectation::PendingThenReady) => {
                assert_pending_completed_phase(state, phase);
            }
            Some(PhaseExpectation::PendingThenDrop) => {
                assert_pending_dropped_phase(state, phase);
            }
            None => {
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
}

async fn assert_actor_segment_held_then_finish(
    fixture: &DbActorFixture,
    frame: &ActorExecutionFrame,
    heap: RequestHeap,
) {
    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(
        matches!(first_poll(competing.as_mut()), Poll::Pending),
        "terminal transaction must hold its resumed Actor segment"
    );
    frame.finish(heap).expect("Actor frame must finish");
    let lease = competing
        .await
        .expect("competing Actor acquire must complete after finish");
    drop(lease);
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

#[test]
fn db_actor_transaction_fixture_exposes_explicit_illegal_flow_case() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let executable = fixture.linked.executable();
    let statement_backed_blocks = executable
        .body
        .blocks
        .iter()
        .filter(|block| !block.statements.is_empty())
        .map(|block| block.label.as_str())
        .collect::<Vec<_>>();

    assert!(
        !statement_backed_blocks.is_empty(),
        "the frozen transaction fixture exposes no statement-backed block for the required \
         explicit illegal-flow case; blocks={:?}; statements={:?}",
        executable
            .body
            .blocks
            .iter()
            .map(|block| (&block.label, &block.statements))
            .collect::<Vec<_>>(),
        executable.body.statements,
    );
}

#[tokio::test]
async fn db_actor_transaction_explicit_body_actual_pending_releases_actor_segment() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    let _gate = state.body_create.push_pending(Ok(DbDocument::new(json!({
        "id": "body-pending"
    }))));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.result = Some(ExprRefIr { expression: 2 });
    let mut env = Env::new();
    let mut eval = pin!(fixture
        .linked
        .interpreter
        .eval_program_explicit_db_transaction(
            fixture.context(frame),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &transaction,
        ));

    match first_poll(eval.as_mut()) {
        Poll::Pending => {}
        Poll::Ready(result) => panic!(
            "an actual-Pending body DB operation must suspend the transaction evaluator; \
             got {result:?}; phases={:?}; body={:?}; abort={:?}",
            state.phases(),
            state.metrics(super::fixture::DbPhase::BodyCreate),
            state.metrics(super::fixture::DbPhase::Abort),
        ),
    }
}

async fn run_ready_success_case(source: TransactionSource) {
    eprintln!("transaction case source={source} phase=ready-success");
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state
        .body_create
        .push_ready(Ok(body_document("ready-success")));
    state.commit.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();

    let result = evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    )
    .await
    .expect("Ready transaction must succeed");

    assert_success_value(source, result);
    assert_heap_retained_body(&heap, checkpoint, "Ready success");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Commit, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn db_actor_transaction_ready_success_matrix() {
    for source in SOURCES {
        run_ready_success_case(source).await;
    }
}

async fn run_actual_pending_phase_case(source: TransactionSource, pending_phase: DbPhase) {
    eprintln!("transaction case source={source} phase={pending_phase:?}-actual-pending");
    let state = FakeDbState::new();
    let error_message = format!("{source}-{pending_phase:?}-body-error");
    let gate = match pending_phase {
        DbPhase::Begin => {
            let gate = state.begin.push_pending(Ok(()));
            state
                .body_create
                .push_ready(Ok(body_document("pending-begin")));
            state.commit.push_ready(Ok(()));
            gate
        }
        DbPhase::BodyCreate => {
            state.begin.push_ready(Ok(()));
            let gate = state
                .body_create
                .push_pending(Ok(body_document("pending-body")));
            state.commit.push_ready(Ok(()));
            gate
        }
        DbPhase::Commit => {
            state.begin.push_ready(Ok(()));
            state
                .body_create
                .push_ready(Ok(body_document("pending-commit")));
            state.commit.push_pending(Ok(()))
        }
        DbPhase::Abort => {
            state.begin.push_ready(Ok(()));
            state.body_create.push_ready(Err(db_error(&error_message)));
            state.abort.push_pending(Ok(()))
        }
        phase => panic!("unsupported transaction Pending phase {phase:?}"),
    };
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();
    let mut evaluation = Box::pin(evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    ));

    match first_poll(evaluation.as_mut()) {
        Poll::Pending => {}
        Poll::Ready(result) => {
            panic!("{source}/{pending_phase:?} must be actual-Pending, got {result:?}")
        }
    }
    assert!(!gate.is_released(), "gate starts closed");
    let competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("actual-Pending phase must release the Actor segment");
    assert!(
        state.metrics(pending_phase).pending_returns > 0,
        "selected phase must have returned Pending before gate release"
    );
    assert_eq!(
        state.metrics(pending_phase).constructed,
        1,
        "selected operation is constructed once"
    );

    gate.release();
    assert!(
        matches!(first_poll(evaluation.as_mut()), Poll::Pending),
        "the same DB future may finish, but the Actor segment remains owned by the competitor"
    );
    assert_eq!(
        state.metrics(pending_phase).ready_returns,
        1,
        "gate release completes the same future"
    );
    assert_eq!(
        state.metrics(pending_phase).constructed,
        1,
        "Pending must not reconstruct the operation"
    );
    drop(competing);
    let result = evaluation.as_mut().await;
    drop(evaluation);

    let expected = match pending_phase {
        DbPhase::Abort => vec![
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::PendingThenReady),
        ],
        _ => vec![
            (
                DbPhase::Begin,
                if pending_phase == DbPhase::Begin {
                    PhaseExpectation::PendingThenReady
                } else {
                    PhaseExpectation::Ready
                },
            ),
            (
                DbPhase::BodyCreate,
                if pending_phase == DbPhase::BodyCreate {
                    PhaseExpectation::PendingThenReady
                } else {
                    PhaseExpectation::Ready
                },
            ),
            (
                DbPhase::Commit,
                if pending_phase == DbPhase::Commit {
                    PhaseExpectation::PendingThenReady
                } else {
                    PhaseExpectation::Ready
                },
            ),
        ],
    };
    assert_transaction_trace(&state, &expected);

    if pending_phase == DbPhase::Abort {
        assert_db_error(result, &error_message);
        assert_heap_rolled_back(&heap, checkpoint, "Pending Abort");
    } else {
        assert_success_value(
            source,
            result.expect("Pending success case must reach the terminal value"),
        );
        assert_heap_retained_body(&heap, checkpoint, "Pending success");
    }
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn db_actor_transaction_each_phase_actual_pending_matrix() {
    for source in SOURCES {
        for phase in TRANSACTION_PHASES {
            run_actual_pending_phase_case(source, phase).await;
        }
    }
}

async fn run_begin_error_case(source: TransactionSource, actual_pending: bool) {
    let timing = if actual_pending {
        "pending-then-error"
    } else {
        "ready-error"
    };
    eprintln!("transaction case source={source} phase=Begin-{timing}");
    let state = FakeDbState::new();
    let error_message = format!("{source}-begin-{timing}");
    let gate = if actual_pending {
        Some(state.begin.push_pending(Err(db_error(&error_message))))
    } else {
        state.begin.push_ready(Err(db_error(&error_message)));
        None
    };
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();
    let mut evaluation = Box::pin(evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    ));

    let result = if let Some(gate) = gate {
        assert!(
            matches!(first_poll(evaluation.as_mut()), Poll::Pending),
            "{source} Begin must expose actual-Pending"
        );
        let competing = fixture
            .actor
            .competing_acquire()
            .await
            .expect("Pending Begin must release the Actor segment");
        assert_eq!(state.metrics(DbPhase::Begin).constructed, 1);
        assert!(state.metrics(DbPhase::Begin).pending_returns > 0);
        gate.release();
        assert!(
            matches!(first_poll(evaluation.as_mut()), Poll::Pending),
            "terminal Begin error still waits to regain the Actor segment"
        );
        assert_eq!(state.metrics(DbPhase::Begin).ready_returns, 1);
        assert_eq!(state.metrics(DbPhase::Begin).constructed, 1);
        drop(competing);
        evaluation.as_mut().await
    } else {
        evaluation.as_mut().await
    };
    drop(evaluation);

    assert_db_error(result, &error_message);
    assert_heap_rolled_back(&heap, checkpoint, "Begin error");
    assert_transaction_trace(
        &state,
        &[(
            DbPhase::Begin,
            if actual_pending {
                PhaseExpectation::PendingThenReady
            } else {
                PhaseExpectation::Ready
            },
        )],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn db_actor_transaction_begin_error_matrix_never_aborts() {
    for source in SOURCES {
        run_begin_error_case(source, false).await;
        run_begin_error_case(source, true).await;
    }
}

async fn run_body_error_case(source: TransactionSource) {
    eprintln!("transaction case source={source} phase=BodyCreate-ready-error");
    let state = FakeDbState::new();
    let error_message = format!("{source}-body-error");
    state.begin.push_ready(Ok(()));
    state.body_create.push_ready(Err(db_error(&error_message)));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();

    let result = evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    )
    .await;

    assert_db_error(result, &error_message);
    assert_heap_rolled_back(&heap, checkpoint, "Body error");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

async fn run_explicit_illegal_flow_case() {
    let source = TransactionSource::Explicit;
    eprintln!("transaction case source={source} phase=illegal-flow");
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();

    let result = evaluate_transaction(
        source,
        TransactionBody::IllegalFlow,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    )
    .await;

    assert_error_text(result, "return is not allowed inside db transaction blocks");
    assert_heap_rolled_back(&heap, checkpoint, "Illegal flow");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn tail_call_negative_db_transaction() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    let abort_gate = state.abort.push_pending(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let initial_heap_len = heap.len();
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.body = TAIL_CALL_BARRIER_BLOCK_LABEL.to_string();
    let mut env = Env::new();
    let mut evaluation = Box::pin(
        fixture
            .linked
            .interpreter
            .eval_program_explicit_db_transaction(
                fixture.context(frame.clone()),
                &mut heap,
                &mut env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &transaction,
            ),
    );

    assert!(
        matches!(first_poll(evaluation.as_mut()), Poll::Pending),
        "ordinary exact local call must reach the selected Abort future"
    );
    let mut competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending Abort must release the Actor segment");
    assert!(
        competing.heap_mut().len() > initial_heap_len,
        "the ordinary exact local call must materialize its structured result before Abort"
    );
    drop(competing);
    abort_gate.release();
    let ordinary_result = evaluation.as_mut().await;
    drop(evaluation);

    assert_error_text(
        ordinary_result.map(RuntimeValueCarrier::into_value),
        "return is not allowed inside db transaction blocks",
    );
    assert_heap_rolled_back(
        &heap,
        checkpoint,
        "ordinary exact local call transaction barrier",
    );
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::PendingThenReady),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;

    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.body = TAIL_CALL_BARRIER_BLOCK_LABEL.to_string();
    let mut env = Env::new();
    let error = fixture
        .linked
        .interpreter
        .eval_program_explicit_db_transaction(
            fixture
                .context(frame.clone())
                .with_program_call_depth_for_test(32),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &transaction,
        )
        .await
        .expect_err("seeded barrier call must retain ordinary program-call depth");

    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded {
            ref resource,
            limit: 32,
            current: 32,
            requested_delta: 1,
            ..
        } if resource == "programCallDepth"
    ));
    assert_heap_rolled_back(
        &heap,
        checkpoint,
        "seeded exact local call transaction barrier",
    );
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn db_actor_transaction_body_error_and_illegal_flow_abort_once() {
    for source in SOURCES {
        run_body_error_case(source).await;
    }
    run_explicit_illegal_flow_case().await;
}

async fn run_commit_error_case(source: TransactionSource) {
    eprintln!("transaction case source={source} phase=Commit-ready-error");
    let state = FakeDbState::new();
    let error_message = format!("{source}-commit-error");
    state.begin.push_ready(Ok(()));
    state
        .body_create
        .push_ready(Ok(body_document("commit-error-body")));
    state.commit.push_ready(Err(db_error(&error_message)));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();

    let result = evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    )
    .await;

    assert_db_error(result, &error_message);
    assert_heap_rolled_back(&heap, checkpoint, "Commit error");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Commit, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn db_actor_transaction_commit_error_aborts_once_and_preserves_error() {
    for source in SOURCES {
        run_commit_error_case(source).await;
    }
}

async fn run_pending_drop_case(source: TransactionSource, drop_phase: DbPhase) {
    eprintln!("transaction case source={source} phase={drop_phase:?}-pending-drop");
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    let gate = match drop_phase {
        DbPhase::BodyCreate => state
            .body_create
            .push_pending(Ok(body_document("drop-body"))),
        DbPhase::Commit => {
            state
                .body_create
                .push_ready(Ok(body_document("drop-commit")));
            state.commit.push_pending(Ok(()))
        }
        phase => panic!("unsupported transaction drop phase {phase:?}"),
    };
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();
    let mut evaluation = Box::pin(evaluate_transaction(
        source,
        TransactionBody::Create,
        &fixture,
        frame,
        &mut heap,
        &mut env,
    ));

    assert!(
        matches!(first_poll(evaluation.as_mut()), Poll::Pending),
        "{source}/{drop_phase:?} must be actual-Pending before outer drop"
    );
    let competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("actual-Pending drop case must release the Actor segment");
    assert_eq!(state.metrics(drop_phase).constructed, 1);
    assert!(state.metrics(drop_phase).pending_returns > 0);
    assert!(!gate.is_released(), "drop gate must remain closed");

    drop(evaluation);
    assert_eq!(state.metrics(drop_phase).ready_returns, 0);
    assert_eq!(state.metrics(drop_phase).dropped_before_terminal, 1);
    assert_eq!(state.metrics(drop_phase).constructed, 1);
    assert!(
        !gate.is_released(),
        "outer drop must not release the DB gate"
    );

    let expected = match drop_phase {
        DbPhase::BodyCreate => vec![
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::PendingThenDrop),
        ],
        DbPhase::Commit => vec![
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Commit, PhaseExpectation::PendingThenDrop),
        ],
        _ => unreachable!("validated above"),
    };
    assert_transaction_trace(&state, &expected);
    assert_eq!(
        state.metrics(DbPhase::PreparedCreateFinalize),
        OperationMetrics::default(),
        "drop must not start a detached finalizer"
    );
    if drop_phase == DbPhase::Commit {
        assert_heap_retained_body(&heap, checkpoint, "Pending Commit drop");
    } else {
        assert_heap_rolled_back(&heap, checkpoint, "Pending body drop");
    }
    drop(competing);
}

#[tokio::test]
async fn db_actor_transaction_commit_actual_pending_drop_has_no_terminal_action() {
    for source in SOURCES {
        run_pending_drop_case(source, DbPhase::Commit).await;
    }
}

#[tokio::test]
async fn db_actor_transaction_body_actual_pending_drop_has_no_terminal_action() {
    for source in SOURCES {
        run_pending_drop_case(source, DbPhase::BodyCreate).await;
    }
}

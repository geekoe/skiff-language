use std::{fmt, pin::pin, sync::atomic::Ordering, task::Poll};

use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_linked_program::{CallIr, DbTransactionIr, ExprRefIr, LinkedExprIr};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    request_heap::{RequestHeap, RequestHeapCheckpoint},
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, RequestException,
    },
};

use super::fixture::{
    db_error, first_poll, DbActorFixture, DbEventKind, DbPhase, FakeDbState, OperationMetrics,
    BODY_CREATE_BLOCK_LABEL, ILLEGAL_FLOW_BLOCK_LABEL, TAIL_CALL_BARRIER_BLOCK_LABEL,
};
use crate::{
    actor_executor::ActorExecutionFrame,
    actor_instance::{ActorInstanceExecutionSnapshot, ActorInstanceStoreError},
    env::Env,
    error::{runtime_error_request_heap_root, Result, RuntimeError, UserException},
    runtime_ops::runtime_member_access_carrier,
};

use super::super::rollback::{rollback_transaction_live_roots, TransactionRollbackCheckpoint};

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
    state.commit.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let exact_call_expression = fixture
        .linked
        .executable()
        .body
        .expressions
        .iter()
        .position(|expression| {
            matches!(
                expression,
                LinkedExprIr::Call { call } if call == &fixture.linked.exact_local_call
            )
        })
        .expect("shared fixture exact local call expression");
    let mut transaction = fixture.linked.explicit_transaction.clone();
    transaction.result = Some(ExprRefIr {
        expression: u32::try_from(exact_call_expression).expect("exact local call expression"),
    });
    let mut env = Env::new();
    let value = fixture
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
        )
        .await
        .expect("transaction result exact call must return through Commit");

    assert!(
        matches!(value.value(), RuntimeValue::Heap(_)),
        "the exact local call must materialize its structured array result"
    );
    assert_heap_retained_body(&heap, checkpoint, "exact local call commit");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Commit, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;

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
    assert_eq!(
        competing.heap_mut().len(),
        initial_heap_len,
        "Pending Abort must publish only persistent Actor fields, not transaction-local results"
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
                .with_program_call_depth_for_test(crate::program_execution::MAX_PROGRAM_CALL_DEPTH),
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
            limit: crate::program_execution::MAX_PROGRAM_CALL_DEPTH,
            current: crate::program_execution::MAX_PROGRAM_CALL_DEPTH,
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

#[tokio::test]
async fn explicit_transaction_rollback_preserves_nested_nominal_throw_for_outer_catch() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let initial_heap_len = heap.len();
    let mut env = Env::new();

    let exception = fixture
        .linked
        .interpreter
        .eval_program_expr_ref(
            fixture.context_with_trace(frame.clone(), "trace:transaction-rollback-catch"),
            &mut heap,
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
async fn explicit_transaction_abort_resume_failure_rolls_back_and_wins_precedence() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    let abort_gate = state.abort.push_pending(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();
    let mut evaluation = Box::pin(
        fixture
            .linked
            .interpreter
            .eval_program_explicit_db_transaction(
                fixture.context_with_trace(frame.clone(), "trace:transaction-abort-resume-failure"),
                &mut heap,
                &mut env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &fixture.linked.rollback_throw_transaction,
            ),
    );

    assert!(
        matches!(first_poll(evaluation.as_mut()), Poll::Pending),
        "the scripted Abort must expose a real Actor suspension"
    );
    assert!(
        fixture
            .actor
            .store
            .begin_upgrade_exact(&fixture.actor.handle),
        "the suspended incarnation must accept the replacement fence"
    );
    abort_gate.release();
    let error = evaluation
        .as_mut()
        .await
        .expect_err("Actor resume failure must replace the original transaction throw");
    drop(evaluation);

    assert!(
        matches!(
            error,
            RuntimeError::ActorInstance(ActorInstanceStoreError::InstanceReplaced)
        ),
        "Abort failure keeps its existing precedence, got {error:?}"
    );
    assert_heap_rolled_back(&heap, checkpoint, "Abort resume failure");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::PendingThenReady),
        ],
    );
    assert!(
        frame
            .with_transaction_live_fields(|fields| Ok(fields.is_none()))
            .expect("missing lease inspection"),
        "resume failure leaves no lease; rollback must not synthesize one"
    );
}

#[tokio::test]
async fn explicit_transaction_abort_cancelled_resume_keeps_cancelled_precedence_without_lease() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    let abort_gate = state.abort.push_pending(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let checkpoint = heap.checkpoint();
    let mut env = Env::new();
    let context =
        fixture.context_with_trace(frame.clone(), "trace:transaction-abort-cancelled-resume");
    let cancel = context.execution().cancel_flag();
    let mut evaluation = Box::pin(
        fixture
            .linked
            .interpreter
            .eval_program_explicit_db_transaction(
                context,
                &mut heap,
                &mut env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &fixture.linked.rollback_throw_transaction,
            ),
    );

    assert!(
        matches!(first_poll(evaluation.as_mut()), Poll::Pending),
        "the scripted Abort must suspend the Actor before cancellation"
    );
    cancel.store(true, Ordering::Release);
    abort_gate.release();
    let error = evaluation
        .as_mut()
        .await
        .expect_err("cancelled resume must replace the original transaction throw");
    drop(evaluation);

    assert!(
        matches!(error, RuntimeError::Cancelled),
        "cancelled Abort resume keeps its exact terminal precedence, got {error:?}"
    );
    assert_heap_rolled_back(&heap, checkpoint, "Abort cancelled resume");
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::Abort, PhaseExpectation::PendingThenReady),
        ],
    );
    assert!(
        frame
            .with_transaction_live_fields(|fields| Ok(fields.is_none()))
            .expect("missing lease inspection"),
        "cancelled resume leaves no lease; rollback must not synthesize one"
    );
}

#[tokio::test]
async fn transaction_rollback_rebases_error_env_and_two_actor_aliases_after_competing_pending() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("entry-live outer slot");
    let checkpoint_len = heap.len();
    let checkpoint = TransactionRollbackCheckpoint::capture(&heap, &env);

    let execution = fixture.context(frame.clone()).execution();
    let (release, pending) = tokio::sync::oneshot::channel::<()>();
    let mut suspended = Box::pin(frame.await_if_pending(&mut heap, &execution, async move {
        pending.await.expect("Pending gate sender")
    }));
    assert!(matches!(first_poll(suspended.as_mut()), Poll::Pending));

    let mut competitor = fixture
        .actor
        .competing_acquire()
        .await
        .expect("actual Pending releases the Actor scheduler");
    let competitor_heap = competitor.take_heap();
    {
        let fields = competitor.fields();
        let mut fields = fields.lock().expect("competitor fields");
        for field in fields
            .iter_mut()
            .filter(|field| field.name == "count" || field.name == "mirror")
        {
            field.value = RuntimeValue::Number(9.0);
            field.assigned = true;
        }
    }
    let competitor_snapshot = ActorInstanceExecutionSnapshot::new(
        competitor
            .fields()
            .lock()
            .expect("competitor fields")
            .clone(),
        competitor_heap,
    );
    fixture
        .actor
        .store
        .commit_execution(&fixture.actor.handle, competitor, competitor_snapshot)
        .expect("competing method publishes its latest fields");
    release.send(()).expect("release Pending operation");
    suspended
        .as_mut()
        .await
        .expect("original continuation resumes after competitor");
    drop(suspended);
    assert_eq!(
        frame.read_field("count").expect("latest count"),
        RuntimeValue::Number(9.0)
    );
    assert_eq!(
        frame.read_field("mirror").expect("latest mirror"),
        RuntimeValue::Number(9.0)
    );

    heap.alloc_bytes(vec![0xde, 0xad])
        .expect("dead transaction-local node");
    let nested = heap
        .alloc_array(vec![RuntimeValue::from("shared")])
        .expect("shared nested node");
    let shared = heap
        .alloc_array(vec![RuntimeValue::Heap(nested)])
        .expect("four-owner shared root");
    env.assign_binding("outer", Some(0), RuntimeValue::Heap(shared))
        .expect("outer slot escapes transaction graph");
    frame
        .with_transaction_live_fields(|fields| {
            let fields = fields.expect("resumed Actor lease");
            for field in fields
                .iter_mut()
                .filter(|field| field.name == "count" || field.name == "mirror")
            {
                field.value = RuntimeValue::Heap(shared);
            }
            Ok(())
        })
        .expect("seed two Actor aliases");

    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 3,
            },
            type_arguments: Vec::new(),
        },
    ));
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    let correlation = ErrorCorrelation {
        trace_id: "trace:four-root-alias".to_string(),
        error_id: "trace:four-root-alias:local-error:1".to_string(),
    };
    let request = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::Heap(shared), identity.clone()),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        correlation.clone(),
    )
    .expect("typed local transaction error");
    let selected = RuntimeError::UserException(UserException::new(request))
        .with_source(71, json!({ "source": "transaction" }))
        .with_diagnostic_frame(json!({ "operation": "db.transaction" }));

    let preserved = rollback_transaction_live_roots(
        &fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        checkpoint,
        selected,
    )
    .expect("valid live roots rollback");

    let error_root = runtime_error_request_heap_root(&preserved)
        .expect("preserved typed error root")
        .value()
        .clone();
    let env_root = env.get_slot(0).expect("preserved Env root").into_value();
    let count_root = frame.read_field("count").expect("preserved count root");
    let mirror_root = frame.read_field("mirror").expect("preserved mirror root");
    assert_eq!(error_root, env_root);
    assert_eq!(env_root, count_root);
    assert_eq!(count_root, mirror_root, "all four owners retain one alias");
    assert_ne!(
        error_root,
        RuntimeValue::Heap(shared),
        "suffix root is rebased"
    );
    assert_eq!(
        runtime_error_request_heap_root(&preserved)
            .expect("typed root")
            .catch_identity(),
        Some(&identity)
    );
    let RuntimeError::WithDiagnosticFrame { error, .. } = &preserved else {
        panic!("diagnostic wrapper must remain outermost")
    };
    let RuntimeError::WithSource { error, .. } = error.as_ref() else {
        panic!("source wrapper must remain nested")
    };
    let RuntimeError::UserException(exception) = error.as_ref() else {
        panic!("typed exception leaf")
    };
    assert_eq!(exception.request().correlation(), &correlation);
    assert_eq!(
        heap.len(),
        checkpoint_len + 2,
        "dead transaction node is collected while shared graph survives"
    );
    frame.finish(heap).expect("finish Actor frame");
}

#[tokio::test]
async fn transaction_rollback_resource_limit_keeps_original_owners_and_actor_lease() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("entry-live outer slot");
    let checkpoint = TransactionRollbackCheckpoint::capture(&heap, &env);

    let mut root = RuntimeValue::from("limit-root");
    for _ in 0..=(heap.limits().max_clone_depth + 1) {
        root = RuntimeValue::Heap(
            heap.alloc_array(vec![root])
                .expect("deep candidate graph stays within node limit"),
        );
    }
    env.assign_binding("outer", Some(0), root.clone())
        .expect("deep Env root");
    frame
        .with_transaction_live_fields(|fields| {
            let fields = fields.expect("active Actor lease");
            for field in fields
                .iter_mut()
                .filter(|field| field.name == "count" || field.name == "mirror")
            {
                field.value = root.clone();
            }
            Ok(())
        })
        .expect("seed deep Actor roots");
    let before = heap.checkpoint();
    let selected = RuntimeError::DbDecode {
        target: "std.db".to_string(),
        message: "commit-selected".to_string(),
    };

    let error = rollback_transaction_live_roots(
        &fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        checkpoint,
        selected,
    )
    .expect("resource-only rollback failure keeps selected error");
    assert!(matches!(
        error,
        RuntimeError::DbDecode { ref message, .. } if message == "commit-selected"
    ));
    assert_eq!(heap.checkpoint(), before, "heap owner is unchanged");
    assert_eq!(
        env.get_slot(0).expect("Env owner").into_value(),
        root,
        "Env owner is unchanged"
    );
    assert_eq!(frame.read_field("count").expect("Actor owner"), root);
    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(
        matches!(first_poll(competing.as_mut()), Poll::Pending),
        "skipped compaction retains the original Actor lease"
    );
    let finish_error = frame
        .finish(heap)
        .expect_err("main Actor persistence gate must reject the over-depth field graph");
    assert!(matches!(
        finish_error,
        RuntimeError::ResourceLimitExceeded { ref reason, .. }
            if reason == "max persistent Actor graph depth"
    ));
    drop(competing.await.expect("Actor lease releases after finish"));
}

#[tokio::test]
async fn transaction_rollback_invalid_source_overrides_business_error_without_partial_publish() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("entry-live outer slot");
    let checkpoint = TransactionRollbackCheckpoint::capture(&heap, &env);
    let corrupt = RuntimeValue::Heap(skiff_runtime_model::runtime_value::HeapHandle::new(
        u32::MAX - 1,
        0,
    ));
    env.assign_binding("outer", Some(0), corrupt.clone())
        .expect("corrupt root carrier");
    let before = heap.checkpoint();

    let error = rollback_transaction_live_roots(
        &fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        checkpoint,
        RuntimeError::DbDecode {
            target: "std.db".to_string(),
            message: "must-not-win".to_string(),
        },
    )
    .expect_err("invalid source graph is a hard invariant failure");
    assert!(
        matches!(error, RuntimeError::Decode(ref message) if message.contains("invalid heap handle")),
        "invalid source must override the business error, got {error:?}"
    );
    assert_eq!(heap.checkpoint(), before, "heap is not partially published");
    assert_eq!(
        env.get_slot(0).expect("unchanged Env owner").into_value(),
        corrupt,
        "Env is not partially published"
    );
    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));
    frame.finish(heap).expect("finish Actor frame");
    drop(competing.await.expect("Actor lease releases after finish"));
}

#[tokio::test]
async fn transaction_rollback_rejects_same_epoch_replacement_before_publishing_owners() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("entry-live outer slot");
    let checkpoint = TransactionRollbackCheckpoint::capture(&heap, &env);
    let shared = heap
        .alloc_array(vec![RuntimeValue::from("stale-owner")])
        .expect("stale owner graph");
    let shared = RuntimeValue::Heap(shared);
    env.assign_binding("outer", Some(0), shared.clone())
        .expect("stale Env owner");
    frame
        .with_transaction_live_fields(|fields| {
            let count = fields
                .expect("active Actor lease")
                .iter_mut()
                .find(|field| field.name == "count")
                .expect("count field");
            count.value = shared.clone();
            Ok(())
        })
        .expect("seed stale Actor owner");
    let before_heap = heap.checkpoint();
    fixture
        .actor
        .replace_same_epoch_while_leased(&fixture.linked);

    let error = rollback_transaction_live_roots(
        &fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        checkpoint,
        RuntimeError::DbDecode {
            target: "std.db".to_string(),
            message: "must-not-publish".to_string(),
        },
    )
    .expect_err("stale same-epoch lease must fail before rollback publication");

    assert!(matches!(
        error,
        RuntimeError::ActorInstance(ActorInstanceStoreError::InstanceReplaced)
    ));
    assert_eq!(heap.checkpoint(), before_heap, "heap is not published");
    assert_eq!(
        env.get_slot(0).expect("unchanged Env owner").into_value(),
        shared,
        "Env is not published"
    );
    assert_eq!(
        frame
            .read_field("count")
            .expect("unchanged stale Actor owner"),
        shared,
        "stale Actor fields are not published"
    );
    drop(frame);
}

#[tokio::test]
async fn explicit_commit_error_after_actual_pending_preserves_latest_actor_and_env_root() {
    let state = FakeDbState::new();
    state.begin.push_ready(Ok(()));
    state
        .body_create
        .push_ready(Ok(body_document("pending-commit-error")));
    let commit_gate = state
        .commit
        .push_pending(Err(db_error("pending-commit-selected")));
    state.abort.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let env_handle = heap
        .alloc_array(vec![RuntimeValue::from("outer-env")])
        .expect("outer Env graph");
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Heap(env_handle))
        .expect("entry-live Env root");
    let mut evaluation = Box::pin(evaluate_transaction(
        TransactionSource::Explicit,
        TransactionBody::Create,
        &fixture,
        frame.clone(),
        &mut heap,
        &mut env,
    ));
    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));

    let mut competitor = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending Commit releases Actor scheduler");
    let competitor_heap = competitor.take_heap();
    {
        let fields = competitor.fields();
        let mut fields = fields.lock().expect("competitor fields");
        for field in fields
            .iter_mut()
            .filter(|field| field.name == "count" || field.name == "mirror")
        {
            field.value = RuntimeValue::Number(12.0);
        }
    }
    let competitor_snapshot = ActorInstanceExecutionSnapshot::new(
        competitor
            .fields()
            .lock()
            .expect("competitor fields")
            .clone(),
        competitor_heap,
    );
    fixture
        .actor
        .store
        .commit_execution(&fixture.actor.handle, competitor, competitor_snapshot)
        .expect("competing method publishes latest fields");
    commit_gate.release();
    let result = evaluation.as_mut().await;
    drop(evaluation);

    assert_db_error(result, "pending-commit-selected");
    assert_eq!(
        frame.read_field("count").expect("latest count"),
        RuntimeValue::Number(12.0)
    );
    assert_eq!(
        frame.read_field("mirror").expect("latest mirror"),
        RuntimeValue::Number(12.0)
    );
    let RuntimeValue::Heap(preserved_env) = env.get_slot(0).expect("preserved Env").into_value()
    else {
        panic!("Env root must remain heap-backed")
    };
    assert_eq!(
        heap.array_item_carrier(preserved_env, 0)
            .expect("read Env root")
            .expect("Env item")
            .into_value(),
        RuntimeValue::from("outer-env")
    );
    assert_transaction_trace(
        &state,
        &[
            (DbPhase::Begin, PhaseExpectation::Ready),
            (DbPhase::BodyCreate, PhaseExpectation::Ready),
            (DbPhase::Commit, PhaseExpectation::PendingThenReady),
            (DbPhase::Abort, PhaseExpectation::Ready),
        ],
    );
    assert_actor_segment_held_then_finish(&fixture, &frame, heap).await;
}

#[tokio::test]
async fn transaction_rollback_keeps_distinct_error_root_out_of_env_actor_alias_class() {
    let fixture = DbActorFixture::new(FakeDbState::new());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = Env::for_program_executable(fixture.linked.executable(), None, 0)
        .expect("fixture environment");
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("entry-live outer slot");
    let checkpoint = TransactionRollbackCheckpoint::capture(&heap, &env);
    let shared = heap
        .alloc_array(vec![RuntimeValue::from("env-actor")])
        .expect("Env/Actor root");
    let error_only = heap
        .alloc_array(vec![RuntimeValue::from("error-only")])
        .expect("distinct error root");
    env.assign_binding("outer", Some(0), RuntimeValue::Heap(shared))
        .expect("Env root");
    frame
        .with_transaction_live_fields(|fields| {
            for field in fields
                .expect("active Actor lease")
                .iter_mut()
                .filter(|field| field.name == "count" || field.name == "mirror")
            {
                field.value = RuntimeValue::Heap(shared);
            }
            Ok(())
        })
        .expect("Actor roots");
    let identity = CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 3,
            },
            type_arguments: Vec::new(),
        },
    ));
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    let request = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::Heap(error_only), identity),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        ErrorCorrelation {
            trace_id: "trace:distinct-error".to_string(),
            error_id: "trace:distinct-error:local-error:1".to_string(),
        },
    )
    .expect("typed error");

    let preserved = rollback_transaction_live_roots(
        &fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        checkpoint,
        RuntimeError::UserException(UserException::new(request)),
    )
    .expect("distinct roots rollback");
    let error_root = runtime_error_request_heap_root(&preserved)
        .expect("error root")
        .value()
        .clone();
    let env_root = env.get_slot(0).expect("Env root").into_value();
    assert_ne!(
        error_root, env_root,
        "distinct transaction error must not silently collide with another root"
    );
    assert_eq!(frame.read_field("count").expect("Actor root"), env_root);
    assert_eq!(frame.read_field("mirror").expect("Actor root"), env_root);
    frame.finish(heap).expect("finish Actor frame");
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

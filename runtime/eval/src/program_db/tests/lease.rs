use std::{
    future::Future,
    pin::{pin, Pin},
    sync::Arc,
    task::Poll,
    time::Duration,
};

use serde_json::json;
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_linked_program::{ExprRefIr, LinkedExprIr};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use crate::{env::Env, error::RuntimeError};

use super::fixture::{
    db_error, first_poll, test_lease_handle, DbActorFixture, DbPhase, FakeDbState,
    OperationMetrics, BODY_CREATE_BLOCK_LABEL, ILLEGAL_FLOW_BLOCK_LABEL,
    TAIL_CALL_BARRIER_BLOCK_LABEL,
};

fn assert_phase_not_started(state: &FakeDbState, phase: DbPhase) {
    assert_eq!(state.metrics(phase).constructed, 0);
}

fn lease_env(fixture: &DbActorFixture) -> Env {
    Env::for_program_executable(
        fixture.linked.executable(),
        Some(fixture.linked.file.module_path.clone()),
        0,
    )
    .expect("lease fixture executable must expose its frozen slot layout")
}

fn assert_no_lease_binding(env: &Env) {
    assert!(
        env.get_slot(0).is_err(),
        "lease binding must stay uninitialized until claim success"
    );
}

fn assert_lease_binding_visible(env: &Env) {
    assert!(
        matches!(
            env.get_slot(0)
                .expect("successful claim must import the frozen binding slot")
                .into_value(),
            RuntimeValue::Heap(_)
        ),
        "lease binding must materialize the claimed document"
    );
}

fn assert_actor_segment_held(fixture: &DbActorFixture) {
    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(
        matches!(first_poll(competing.as_mut()), Poll::Pending),
        "first-Ready DB phases must keep the Actor segment"
    );
}

fn assert_ready_once(state: &FakeDbState, phase: DbPhase) {
    state.assert_completed_once(phase);
}

fn assert_pending_before_terminal(state: &FakeDbState, phase: DbPhase) {
    let metrics = state.metrics(phase);
    assert_eq!(metrics.constructed, 1, "{phase:?} must construct once");
    assert!(
        metrics.pending_returns > 0,
        "{phase:?} must return actual Pending before its gate"
    );
    assert_eq!(metrics.ready_returns, 0, "{phase:?} terminal is gated");
    assert_eq!(metrics.dropped_before_terminal, 0);
}

fn assert_pending_then_ready_once(state: &FakeDbState, phase: DbPhase) {
    let metrics = state.metrics(phase);
    assert_eq!(metrics.constructed, 1, "{phase:?} must construct once");
    assert!(metrics.pending_returns > 0, "{phase:?} must be Pending");
    assert_eq!(metrics.ready_returns, 1, "{phase:?} must return once");
    assert_eq!(metrics.dropped_before_terminal, 0);
    assert_eq!(metrics.dropped_after_terminal, 1);
}

fn assert_pending_dropped_once(state: &FakeDbState, phase: DbPhase) {
    let metrics = state.metrics(phase);
    assert_eq!(metrics.constructed, 1, "{phase:?} must construct once");
    assert!(metrics.pending_returns > 0, "{phase:?} must be Pending");
    assert_eq!(metrics.ready_returns, 0, "{phase:?} must not return late");
    assert_eq!(
        metrics.dropped_before_terminal, 1,
        "{phase:?} must drop the same pending future"
    );
    assert_eq!(metrics.dropped_after_terminal, 0);
}

async fn wait_until_dropped(state: &FakeDbState, phase: DbPhase) {
    for _ in 0..64 {
        if state.metrics(phase).dropped_before_terminal == 1 {
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("{phase:?} did not observe pending-future drop");
}

async fn poll_until_phase_pending<F>(mut future: Pin<&mut F>, state: &FakeDbState, phase: DbPhase)
where
    F: Future,
{
    for _ in 0..64 {
        assert!(
            matches!(first_poll(future.as_mut()), Poll::Pending),
            "evaluation returned before gated {phase:?}"
        );
        if state.metrics(phase).constructed == 1 {
            assert_pending_before_terminal(state, phase);
            return;
        }
        tokio::task::yield_now().await;
    }
    panic!("evaluation did not reach gated {phase:?}");
}

async fn wait_for_real_renew_poll(state: &Arc<FakeDbState>) {
    let probe = state.probe(DbPhase::Renew);
    tokio::time::sleep(Duration::from_millis(5)).await;
    probe.wait_until_polled().await;
}

#[test]
fn db_actor_lease_fixture_exposes_required_binding_variant() {
    eprintln!("phase=fixture variant=frozen-binding");
    let fixture = DbActorFixture::new(FakeDbState::new());
    let claim = fixture
        .linked
        .executable()
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::DbLeaseClaim { claim } => Some(claim),
            _ => None,
        })
        .expect("shared fixture claim expression");

    assert!(
        claim.binding_slot.is_some(),
        "TASK_NOT_EXECUTABLE: frozen expression fixture has no lease binding variant"
    );
}

#[tokio::test]
async fn db_actor_lease_claim_pending_uses_one_actor_segment() {
    eprintln!("phase=claim variant=none-actual-pending");
    let state = FakeDbState::new();
    let claim_gate = state.claim.push_pending(Ok(None));
    let fixture = DbActorFixture::new(state.clone());
    let claim_expression = fixture
        .linked
        .executable()
        .body
        .expressions
        .iter()
        .position(|expression| {
            matches!(
                expression,
                LinkedExprIr::DbLeaseClaim { claim } if claim == &fixture.linked.claim
            )
        })
        .expect("shared fixture claim expression");
    let claim_expression = ExprRefIr {
        expression: u32::try_from(claim_expression).expect("fixture expression index"),
    };
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());
    let mut env = lease_env(&fixture);
    assert_no_lease_binding(&env);
    let value = {
        let mut evaluation = pin!(fixture.linked.interpreter.eval_program_expr_ref(
            context,
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            claim_expression,
        ));

        match first_poll(evaluation.as_mut()) {
            Poll::Pending => {}
            Poll::Ready(result) => {
                panic!("actual-Pending claim must cut one Actor segment, got {result:?}")
            }
        }
        let competing = fixture
            .actor
            .competing_acquire()
            .await
            .expect("Pending claim must release the Actor segment");
        claim_gate.release();
        assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
        assert_eq!(state.metrics(DbPhase::Claim).constructed, 1);
        drop(competing);
        evaluation
            .as_mut()
            .await
            .expect("claim must resume through the real expression entry")
    };

    assert!(matches!(value.into_value(), RuntimeValue::Bool(false)));
    assert_no_lease_binding(&env);
    frame.finish(heap).expect("Actor frame must finish");
    assert_eq!(
        state.metrics(DbPhase::Claim),
        OperationMetrics {
            constructed: 1,
            polls: 3,
            pending_returns: 2,
            ready_returns: 1,
            dropped_before_terminal: 0,
            dropped_after_terminal: 1,
        }
    );
    assert_eq!(state.phases(), vec![DbPhase::Claim]);
    assert_phase_not_started(&state, DbPhase::Renew);
    assert_phase_not_started(&state, DbPhase::LeaseLost);
    assert_phase_not_started(&state, DbPhase::Release);
}

#[tokio::test]
async fn db_actor_lease_claim_none_ready_has_no_binding_or_terminal_phases() {
    eprintln!("phase=claim variant=none-ready");
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(None));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    assert_no_lease_binding(&env);

    let result = fixture
        .linked
        .interpreter
        .eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.claim,
        )
        .await
        .expect("Ready None claim must succeed");

    assert!(matches!(result, RuntimeValue::Bool(false)));
    assert_no_lease_binding(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::Claim);
    assert_eq!(state.phases(), vec![DbPhase::Claim]);
    assert_phase_not_started(&state, DbPhase::Renew);
    assert_phase_not_started(&state, DbPhase::LeaseLost);
    assert_phase_not_started(&state, DbPhase::Release);
}

#[tokio::test]
async fn db_actor_lease_claim_success_ready_imports_binding_once() {
    eprintln!("phase=claim variant=success-ready");
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        1,
        json!({ "owner": "ready" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    state.release.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    assert_no_lease_binding(&env);

    let result = fixture
        .linked
        .interpreter
        .eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.claim,
        )
        .await
        .expect("Ready successful claim must complete");

    assert!(matches!(result, RuntimeValue::Bool(true)));
    assert_lease_binding_visible(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::Claim);
    assert_ready_once(&state, DbPhase::LeaseLost);
    assert_ready_once(&state, DbPhase::Release);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release]
    );
    assert_phase_not_started(&state, DbPhase::Renew);
}

#[tokio::test]
async fn db_actor_lease_claim_success_pending_imports_only_after_same_future_resumes() {
    eprintln!("phase=claim variant=success-actual-pending");
    let state = FakeDbState::new();
    let claim_gate = state.claim.push_pending(Ok(Some(test_lease_handle(
        2,
        json!({ "owner": "pending" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    state.release.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    assert_no_lease_binding(&env);
    let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_claim(
        fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.claim,
    ));

    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    assert_pending_before_terminal(&state, DbPhase::Claim);
    assert_phase_not_started(&state, DbPhase::Renew);
    assert_phase_not_started(&state, DbPhase::LeaseLost);
    assert_phase_not_started(&state, DbPhase::Release);
    let competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending claim must release the Actor segment");
    claim_gate.release();
    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    drop(competing);
    let result = evaluation
        .as_mut()
        .await
        .expect("the same pending claim future must complete");
    drop(evaluation);

    assert!(matches!(result, RuntimeValue::Bool(true)));
    assert_lease_binding_visible(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::Claim);
    assert_ready_once(&state, DbPhase::LeaseLost);
    assert_ready_once(&state, DbPhase::Release);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release]
    );
    assert_phase_not_started(&state, DbPhase::Renew);
}

#[tokio::test]
async fn db_actor_lease_body_pending_cleanup_stops_renew_before_terminals() {
    for (variant, body_terminal, expect_error) in [
        (
            "normal-success",
            Ok(DbDocument::new(json!({ "id": "body-success" }))),
            None,
        ),
        (
            "body-error",
            Err(db_error("lease body failure")),
            Some("lease body failure"),
        ),
    ] {
        eprintln!("phase=body variant={variant}");
        let state = FakeDbState::new();
        state.claim.push_ready(Ok(Some(test_lease_handle(
            10,
            json!({ "owner": variant }),
            3,
        ))));
        let body_gate = state.body_create.push_pending(body_terminal);
        let renew_gate = state.renew.push_pending(Ok(true));
        state.lease_lost.push_ready(Ok(false));
        state.release.push_ready(Ok(()));
        let fixture = DbActorFixture::new(state.clone());
        let (frame, mut heap) = fixture.actor.execution_frame().await;
        let mut claim = fixture.linked.claim.clone();
        claim.body = BODY_CREATE_BLOCK_LABEL.to_string();
        let mut env = lease_env(&fixture);
        assert_no_lease_binding(&env);
        let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &claim,
        ));

        assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
        assert_ready_once(&state, DbPhase::Claim);
        assert_pending_before_terminal(&state, DbPhase::BodyCreate);
        let mut competing = fixture
            .actor
            .competing_acquire()
            .await
            .expect("Pending body DB call must release the Actor segment");
        let heap_len_while_pending = competing.heap_mut().len();
        wait_for_real_renew_poll(&state).await;
        assert_pending_before_terminal(&state, DbPhase::Renew);
        assert_phase_not_started(&state, DbPhase::LeaseLost);
        assert_phase_not_started(&state, DbPhase::Release);

        drop(competing);
        body_gate.release();
        let result = tokio::time::timeout(Duration::from_secs(1), evaluation.as_mut())
            .await
            .unwrap_or_else(|_| {
                panic!(
                    "claim cleanup did not terminate for {variant}; phases={:?}; body={:?}; \
                     renew={:?}; lost={:?}; release={:?}",
                    state.phases(),
                    state.metrics(DbPhase::BodyCreate),
                    state.metrics(DbPhase::Renew),
                    state.metrics(DbPhase::LeaseLost),
                    state.metrics(DbPhase::Release),
                )
            });
        drop(evaluation);

        match expect_error {
            Some(message) => assert!(format!(
                "{:?}",
                result.expect_err("body error must survive cleanup")
            )
            .contains(message)),
            None => {
                assert!(matches!(
                    result.expect("normal body must succeed"),
                    RuntimeValue::Bool(true)
                ));
                assert!(
                    heap.len() > heap_len_while_pending,
                    "body result must materialize only after its gate"
                );
            }
        }
        assert_lease_binding_visible(&env);
        assert_actor_segment_held(&fixture);
        frame.finish(heap).expect("Actor frame must finish");
        assert_pending_then_ready_once(&state, DbPhase::BodyCreate);
        assert_pending_dropped_once(&state, DbPhase::Renew);
        assert_ready_once(&state, DbPhase::LeaseLost);
        assert_ready_once(&state, DbPhase::Release);
        assert_eq!(
            state.phases(),
            vec![
                DbPhase::Claim,
                DbPhase::BodyCreate,
                DbPhase::Renew,
                DbPhase::LeaseLost,
                DbPhase::Release,
            ],
            "Renew must stop/join before terminal phases for {variant}"
        );
        let renew_metrics = state.metrics(DbPhase::Renew);
        renew_gate.release();
        tokio::task::yield_now().await;
        assert_eq!(
            state.metrics(DbPhase::Renew),
            renew_metrics,
            "late Renew sender must not revive the dropped future"
        );
    }
}

#[tokio::test]
async fn db_actor_lease_illegal_flow_still_runs_lost_and_release() {
    eprintln!("phase=body variant=explicit-illegal-flow");
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        20,
        json!({ "owner": "illegal-flow" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    state.release.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut claim = fixture.linked.claim.clone();
    claim.body = ILLEGAL_FLOW_BLOCK_LABEL.to_string();
    let mut env = lease_env(&fixture);
    let error = fixture
        .linked
        .interpreter
        .eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &claim,
        )
        .await
        .expect_err("return inside claim body must be rejected after cleanup");

    assert!(
        format!("{error:?}").contains("return is not allowed inside db claim blocks"),
        "unexpected illegal-flow error: {error:?}"
    );
    assert_lease_binding_visible(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::Claim);
    assert_ready_once(&state, DbPhase::LeaseLost);
    assert_ready_once(&state, DbPhase::Release);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release]
    );
    assert_phase_not_started(&state, DbPhase::Renew);
}

#[tokio::test]
async fn tail_call_negative_db_lease() {
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        21,
        json!({ "owner": "ordinary-structured" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    state.release.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let initial_heap_len = heap.len();
    let mut claim = fixture.linked.claim.clone();
    claim.binding_slot = None;
    claim.body = TAIL_CALL_BARRIER_BLOCK_LABEL.to_string();
    let mut env = lease_env(&fixture);
    let error = fixture
        .linked
        .interpreter
        .eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &claim,
        )
        .await
        .expect_err("ordinary exact local call must remain inside the claim barrier");

    assert!(
        format!("{error:?}").contains("return is not allowed inside db claim blocks"),
        "ordinary structured result must complete before claim-flow validation: {error:?}"
    );
    assert!(
        heap.len() > initial_heap_len,
        "ordinary exact local call must materialize its structured result"
    );
    assert_no_lease_binding(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::Claim);
    assert_ready_once(&state, DbPhase::LeaseLost);
    assert_ready_once(&state, DbPhase::Release);
    assert_phase_not_started(&state, DbPhase::Renew);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release],
        "LeaseRenewOwner must stop before the held lease is released"
    );

    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        22,
        json!({ "owner": "seeded-depth" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    state.release.push_ready(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let initial_heap_len = heap.len();
    let mut claim = fixture.linked.claim.clone();
    claim.binding_slot = None;
    claim.body = TAIL_CALL_BARRIER_BLOCK_LABEL.to_string();
    let mut env = lease_env(&fixture);
    let error = fixture
        .linked
        .interpreter
        .eval_program_db_lease_claim(
            fixture
                .context(frame.clone())
                .with_program_call_depth_for_test(crate::program_execution::MAX_PROGRAM_CALL_DEPTH),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &claim,
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
    assert_eq!(
        heap.len(),
        initial_heap_len,
        "depth rejection must not materialize the structured callee result"
    );
    assert_no_lease_binding(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_ready_once(&state, DbPhase::Claim);
    assert_ready_once(&state, DbPhase::LeaseLost);
    assert_ready_once(&state, DbPhase::Release);
    assert_phase_not_started(&state, DbPhase::Renew);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release],
        "seeded depth error must still release the held lease exactly once"
    );
}

#[tokio::test]
async fn db_actor_lease_lost_and_release_error_priority_matrix() {
    for (variant, lease_lost, pending_lost, illegal_flow, expected) in [
        ("lost-ready", true, false, false, "db lease was lost"),
        (
            "lost-actual-pending-beats-release",
            true,
            true,
            false,
            "db lease was lost",
        ),
        (
            "release-beats-illegal-flow",
            false,
            false,
            true,
            "release failure",
        ),
    ] {
        eprintln!("phase=terminal variant={variant}");
        let state = FakeDbState::new();
        state.claim.push_ready(Ok(Some(test_lease_handle(
            30,
            json!({ "owner": variant }),
            30_000,
        ))));
        let lost_gate = pending_lost.then(|| state.lease_lost.push_pending(Ok(lease_lost)));
        if !pending_lost {
            state.lease_lost.push_ready(Ok(lease_lost));
        }
        state.release.push_ready(Err(db_error("release failure")));
        let fixture = DbActorFixture::new(state.clone());
        let (frame, mut heap) = fixture.actor.execution_frame().await;
        let mut claim = fixture.linked.claim.clone();
        if illegal_flow {
            claim.body = ILLEGAL_FLOW_BLOCK_LABEL.to_string();
        }
        let mut env = lease_env(&fixture);
        let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_claim(
            fixture.context(frame.clone()),
            &mut heap,
            &mut env,
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &claim,
        ));

        let result = if let Some(gate) = lost_gate {
            poll_until_phase_pending(evaluation.as_mut(), &state, DbPhase::LeaseLost).await;
            assert_phase_not_started(&state, DbPhase::Release);
            let competing = fixture
                .actor
                .competing_acquire()
                .await
                .expect("Pending LeaseLost must release the Actor segment");
            gate.release();
            assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
            drop(competing);
            evaluation.as_mut().await
        } else {
            evaluation.as_mut().await
        };
        drop(evaluation);

        let error = result.expect_err("terminal matrix case must fail");
        assert!(
            format!("{error:?}").contains(expected),
            "unexpected terminal priority for {variant}: {error:?}"
        );
        if lease_lost {
            assert!(
                !format!("{error:?}").contains("release failure"),
                "LeaseLost must outrank Release error"
            );
        }
        if illegal_flow {
            assert!(
                !format!("{error:?}").contains("return is not allowed"),
                "Release error must outrank body flow error"
            );
        }
        assert_lease_binding_visible(&env);
        assert_actor_segment_held(&fixture);
        frame.finish(heap).expect("Actor frame must finish");
        assert_ready_once(&state, DbPhase::Claim);
        if pending_lost {
            assert_pending_then_ready_once(&state, DbPhase::LeaseLost);
        } else {
            assert_ready_once(&state, DbPhase::LeaseLost);
        }
        assert_ready_once(&state, DbPhase::Release);
        assert_eq!(
            state.phases(),
            vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release]
        );
        assert_phase_not_started(&state, DbPhase::Renew);
    }
}

#[tokio::test]
async fn db_actor_lease_body_pending_drop_aborts_real_renew_future() {
    eprintln!("phase=body variant=actual-pending-outer-drop");
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        40,
        json!({ "owner": "drop" }),
        3,
    ))));
    let body_gate = state.body_create.push_pending(Ok(DbDocument::new(json!({
        "id": "must-not-materialize"
    }))));
    let renew_gate = state.renew.push_pending(Ok(true));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut claim = fixture.linked.claim.clone();
    claim.body = BODY_CREATE_BLOCK_LABEL.to_string();
    let mut env = lease_env(&fixture);
    let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_claim(
        fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &claim,
    ));

    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    assert_pending_before_terminal(&state, DbPhase::BodyCreate);
    let mut competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending body must release the Actor segment");
    let pending_heap_len = competing.heap_mut().len();
    wait_for_real_renew_poll(&state).await;
    assert_pending_before_terminal(&state, DbPhase::Renew);
    let renew_polls_before_drop = state.metrics(DbPhase::Renew).polls;
    drop(evaluation);
    wait_until_dropped(&state, DbPhase::Renew).await;

    assert_lease_binding_visible(&env);
    assert_pending_dropped_once(&state, DbPhase::BodyCreate);
    assert_pending_dropped_once(&state, DbPhase::Renew);
    assert_phase_not_started(&state, DbPhase::LeaseLost);
    assert_phase_not_started(&state, DbPhase::Release);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::BodyCreate, DbPhase::Renew]
    );
    let body_metrics = state.metrics(DbPhase::BodyCreate);
    let renew_metrics = state.metrics(DbPhase::Renew);
    body_gate.release();
    renew_gate.release();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(state.metrics(DbPhase::BodyCreate), body_metrics);
    assert_eq!(state.metrics(DbPhase::Renew), renew_metrics);
    assert_eq!(
        state.metrics(DbPhase::Renew).polls,
        renew_polls_before_drop,
        "aborted Renew future must never poll again"
    );
    assert_eq!(
        competing.heap_mut().len(),
        pending_heap_len,
        "late body sender must not materialize its document"
    );
    drop(competing);
    drop(frame);
    drop(heap);
}

#[tokio::test]
async fn db_actor_lease_release_pending_drop_has_no_late_terminal() {
    eprintln!("phase=release variant=actual-pending-outer-drop");
    let state = FakeDbState::new();
    state.claim.push_ready(Ok(Some(test_lease_handle(
        50,
        json!({ "owner": "release-drop" }),
        30_000,
    ))));
    state.lease_lost.push_ready(Ok(false));
    let release_gate = state.release.push_pending(Ok(()));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_claim(
        fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.claim,
    ));

    poll_until_phase_pending(evaluation.as_mut(), &state, DbPhase::Release).await;
    let mut competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending Release must release the Actor segment");
    let pending_heap_len = competing.heap_mut().len();
    drop(evaluation);

    assert_lease_binding_visible(&env);
    assert_pending_dropped_once(&state, DbPhase::Release);
    assert_eq!(
        state.phases(),
        vec![DbPhase::Claim, DbPhase::LeaseLost, DbPhase::Release]
    );
    let release_metrics = state.metrics(DbPhase::Release);
    release_gate.release();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        state.metrics(DbPhase::Release),
        release_metrics,
        "late Release sender must not return or reconstruct an operation"
    );
    assert_eq!(competing.heap_mut().len(), pending_heap_len);
    assert_phase_not_started(&state, DbPhase::Renew);
    drop(competing);
    drop(frame);
    drop(heap);
}

#[tokio::test]
async fn db_actor_lease_read_ready_none_and_store_error_matrix() {
    enum Expected {
        Heap,
        Null,
        Error(&'static str),
    }

    for (variant, terminal, expected) in [
        (
            "ready-object",
            Ok(Some(json!({ "owner": "reader" }))),
            Expected::Heap,
        ),
        ("ready-none", Ok(None), Expected::Null),
        (
            "ready-store-error",
            Err(db_error("lease read failure")),
            Expected::Error("lease read failure"),
        ),
    ] {
        eprintln!("phase=read variant={variant}");
        let state = FakeDbState::new();
        state.read.push_ready(terminal);
        let fixture = DbActorFixture::new(state.clone());
        let (frame, mut heap) = fixture.actor.execution_frame().await;
        let mut env = lease_env(&fixture);
        let result = fixture
            .linked
            .interpreter
            .eval_program_db_lease_read(
                fixture.context(frame.clone()),
                &mut heap,
                &mut env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &fixture.linked.read,
            )
            .await;

        match expected {
            Expected::Heap => {
                assert!(matches!(
                    result.expect("read must succeed"),
                    RuntimeValue::Heap(_)
                ));
            }
            Expected::Null => {
                assert!(matches!(
                    result.expect("None read must succeed"),
                    RuntimeValue::Null
                ));
            }
            Expected::Error(message) => assert!(format!(
                "{:?}",
                result.expect_err("store error must propagate")
            )
            .contains(message)),
        }
        assert_no_lease_binding(&env);
        assert_actor_segment_held(&fixture);
        frame.finish(heap).expect("Actor frame must finish");
        assert_ready_once(&state, DbPhase::Read);
        assert_eq!(state.phases(), vec![DbPhase::Read]);
        for phase in [
            DbPhase::Claim,
            DbPhase::Renew,
            DbPhase::LeaseLost,
            DbPhase::Release,
        ] {
            assert_phase_not_started(&state, phase);
        }
    }
}

#[tokio::test]
async fn db_actor_lease_read_pending_resumes_same_future_and_materializes_after_gate() {
    eprintln!("phase=read variant=actual-pending-array");
    let state = FakeDbState::new();
    let read_gate = state
        .read
        .push_pending(Ok(Some(json!(["owner", "pending"]))));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_read(
        fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.read,
    ));

    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    assert_pending_before_terminal(&state, DbPhase::Read);
    let mut competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending Read must release the Actor segment");
    let pending_heap_len = competing.heap_mut().len();
    read_gate.release();
    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    drop(competing);
    let result = evaluation
        .as_mut()
        .await
        .expect("the same Read future must resume");
    drop(evaluation);

    assert!(matches!(result, RuntimeValue::Heap(_)));
    assert!(
        heap.len() > pending_heap_len,
        "Read result must materialize only after the gate"
    );
    assert_no_lease_binding(&env);
    assert_actor_segment_held(&fixture);
    frame.finish(heap).expect("Actor frame must finish");
    assert_pending_then_ready_once(&state, DbPhase::Read);
    assert_eq!(state.phases(), vec![DbPhase::Read]);
}

#[tokio::test]
async fn db_actor_lease_read_limited_heap_rejects_object_and_array_decode() {
    for (variant, value) in [
        ("decode-object", json!({ "owner": "too-large" })),
        ("decode-array", json!(["too-large"])),
    ] {
        eprintln!("phase=read variant={variant}");
        let state = FakeDbState::new();
        state.read.push_ready(Ok(Some(value)));
        let fixture = DbActorFixture::new(state.clone());
        let (frame, actor_heap) = fixture.actor.execution_frame().await;
        drop(actor_heap);
        let mut heap = RequestHeap::new(RequestHeapLimits {
            max_nodes: 0,
            ..RequestHeapLimits::default()
        });
        let mut env = lease_env(&fixture);
        let result = fixture
            .linked
            .interpreter
            .eval_program_db_lease_read(
                fixture.context(frame.clone()),
                &mut heap,
                &mut env,
                &fixture.linked.addr,
                &fixture.linked.file,
                fixture.linked.executable(),
                &fixture.linked.read,
            )
            .await;

        assert!(
            result.is_err(),
            "restricted RequestHeapLimits must reject {variant}"
        );
        assert_eq!(
            heap.len(),
            0,
            "failed decode must not materialize a heap node"
        );
        assert_no_lease_binding(&env);
        assert_actor_segment_held(&fixture);
        frame.finish(heap).expect("Actor frame must finish");
        assert_ready_once(&state, DbPhase::Read);
        assert_eq!(state.phases(), vec![DbPhase::Read]);
    }
}

#[tokio::test]
async fn db_actor_lease_read_pending_drop_does_not_rebuild_or_materialize() {
    eprintln!("phase=read variant=actual-pending-outer-drop");
    let state = FakeDbState::new();
    let read_gate = state
        .read
        .push_pending(Ok(Some(json!({ "owner": "late" }))));
    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let mut env = lease_env(&fixture);
    let mut evaluation = Box::pin(fixture.linked.interpreter.eval_program_db_lease_read(
        fixture.context(frame.clone()),
        &mut heap,
        &mut env,
        &fixture.linked.addr,
        &fixture.linked.file,
        fixture.linked.executable(),
        &fixture.linked.read,
    ));

    assert!(matches!(first_poll(evaluation.as_mut()), Poll::Pending));
    assert_pending_before_terminal(&state, DbPhase::Read);
    let mut competing = fixture
        .actor
        .competing_acquire()
        .await
        .expect("Pending Read must release the Actor segment");
    let pending_heap_len = competing.heap_mut().len();
    drop(evaluation);

    assert_pending_dropped_once(&state, DbPhase::Read);
    assert_no_lease_binding(&env);
    assert_eq!(state.phases(), vec![DbPhase::Read]);
    let read_metrics = state.metrics(DbPhase::Read);
    read_gate.release();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        state.metrics(DbPhase::Read),
        read_metrics,
        "late Read sender must not return or rebuild"
    );
    assert_eq!(
        competing.heap_mut().len(),
        pending_heap_len,
        "late Read result must not materialize"
    );
    for phase in [
        DbPhase::Claim,
        DbPhase::Renew,
        DbPhase::LeaseLost,
        DbPhase::Release,
    ] {
        assert_phase_not_started(&state, phase);
    }
    drop(competing);
    drop(frame);
    drop(heap);
}

use std::{pin::pin, task::Poll};

use skiff_runtime_linked_program::{ExprRefIr, LinkedExprIr};
use skiff_runtime_model::runtime_value::RuntimeValue;

use crate::env::Env;

use super::fixture::{first_poll, DbActorFixture, DbPhase, FakeDbState};

#[tokio::test]
async fn db_actor_lease_claim_pending_uses_one_actor_segment() {
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
    let mut env = Env::new();
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
    frame.finish(heap).expect("Actor frame must finish");
    state.assert_completed_once(DbPhase::Claim);
    assert_eq!(state.metrics(DbPhase::Renew).constructed, 0);
    assert_eq!(state.metrics(DbPhase::LeaseLost).constructed, 0);
    assert_eq!(state.metrics(DbPhase::Release).constructed, 0);
}

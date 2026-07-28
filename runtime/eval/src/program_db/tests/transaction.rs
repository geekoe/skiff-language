use std::{pin::pin, task::Poll};

use serde_json::json;
use skiff_runtime_capability_context::DbDocument;
use skiff_runtime_linked_program::ExprRefIr;

use super::fixture::{first_poll, DbActorFixture, FakeDbState};
use crate::env::Env;

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

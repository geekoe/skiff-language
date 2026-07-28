mod actor;
mod program;
mod state;
mod store;

pub(super) use actor::*;
pub(super) use program::*;
pub(super) use state::*;
pub(super) use store::*;

use std::{pin::pin, task::Poll};

use serde_json::json;
use skiff_runtime_model::runtime_value::{RuntimeObject, RuntimeValue};

use crate::env::Env;

#[tokio::test]
async fn db_actor_fixture_checkpoint() {
    let state = FakeDbState::new();
    state
        .raw_create
        .push_ready(Ok(skiff_runtime_capability_context::DbDocument::new(
            json!({ "id": "raw-1" }),
        )));
    state
        .prepared_create
        .push_ready(Ok(PreparedFinalize::new(|heap| {
            let handle = heap
                .alloc_object(RuntimeObject::unshaped(Default::default()))
                .map_err(|error| db_error(error.to_string()))?;
            Ok(RuntimeValue::Heap(handle))
        })));

    let fixture = DbActorFixture::new(state.clone());
    let (frame, mut heap) = fixture.actor.execution_frame().await;
    let context = fixture.context(frame.clone());

    let mut competing = pin!(fixture.actor.competing_acquire());
    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));

    let raw = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            context.clone(),
            &mut heap,
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.raw_create,
        )
        .await
        .expect("raw create must execute through the DB evaluator");
    assert!(matches!(raw, RuntimeValue::Heap(_)));

    let prepared = fixture
        .linked
        .interpreter
        .eval_program_db_operation(
            context,
            &mut heap,
            &mut Env::new(),
            &fixture.linked.addr,
            &fixture.linked.file,
            fixture.linked.executable(),
            &fixture.linked.prepared_create,
        )
        .await
        .expect("prepared create must execute through the DB evaluator");
    assert!(matches!(prepared, RuntimeValue::Heap(_)));

    assert!(matches!(first_poll(competing.as_mut()), Poll::Pending));
    frame.finish(heap).expect("Actor frame must finish");
    let competing_lease = competing
        .await
        .expect("competing Actor acquire must complete after finish");
    drop(competing_lease);

    state.assert_completed_once(DbPhase::RawCreate);
    state.assert_completed_once(DbPhase::PreparedCreateWait);
    state.assert_completed_once(DbPhase::PreparedCreateFinalize);
    assert_eq!(state.context_require_calls(), 2);
    assert_eq!(state.legacy_runtime_calls(), 0);
    assert_eq!(
        state.phases(),
        vec![
            DbPhase::RawCreate,
            DbPhase::PreparedCreateWait,
            DbPhase::PreparedCreateFinalize,
        ]
    );
}

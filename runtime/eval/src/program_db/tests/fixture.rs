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
use skiff_runtime_linked_program::{LinkedExprIr, LinkedStmtIr};
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
    let executable = fixture.linked.executable();
    assert_eq!(fixture.linked.program.service_files.len(), 1);
    assert!(fixture.linked.program.packages.is_empty());
    assert!(fixture.linked.program.package_files.is_empty());
    assert_eq!(fixture.linked.file.executables.len(), 1);

    let binding_slot = fixture
        .linked
        .claim
        .binding_slot
        .expect("canonical lease claim binding slot");
    assert!(
        usize::try_from(binding_slot).expect("binding slot index") < executable.slots.frame_size
    );
    assert!(executable
        .slots
        .slots
        .iter()
        .any(|slot| slot.index == binding_slot as usize));

    let body_create = executable
        .body
        .blocks
        .iter()
        .find(|block| block.label == BODY_CREATE_BLOCK_LABEL)
        .expect("body-create block");
    let body_create_statement = body_create
        .statements
        .first()
        .expect("body-create statement");
    let LinkedStmtIr::Expr { value } =
        &executable.body.statements[body_create_statement.statement as usize]
    else {
        panic!("body-create block must execute an expression statement");
    };
    assert!(matches!(
        &executable.body.expressions[value.expression as usize],
        LinkedExprIr::DbOperation { operation } if operation == &fixture.linked.raw_create
    ));

    let illegal_flow = executable
        .body
        .blocks
        .iter()
        .find(|block| block.label == ILLEGAL_FLOW_BLOCK_LABEL)
        .expect("illegal-flow block");
    let illegal_flow_statement = illegal_flow
        .statements
        .first()
        .expect("illegal-flow statement");
    assert!(matches!(
        &executable.body.statements[illegal_flow_statement.statement as usize],
        LinkedStmtIr::Return { .. }
    ));

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

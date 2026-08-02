use super::actor_dispatch::{actor_return, RecordingActor};
use super::*;

fn self_dispatch_executor_file() -> Arc<LinkedFileUnit> {
    let mut file = (*actor_file(integer(), true)).clone();
    let executable = &mut file.executables[0];
    executable.self_type = Some(LinkedTypeRef::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: "actors".to_string(),
            symbol: "Counter".to_string(),
        },
    });
    executable.params[0].slot = 1;
    executable.slots = SlotLayoutIr {
        slots: vec![
            SlotIr {
                index: 0,
                name: "self".to_string(),
                kind: "selfValue".to_string(),
            },
            SlotIr {
                index: 1,
                name: "value".to_string(),
                kind: "param".to_string(),
            },
            SlotIr {
                index: 2,
                name: "nested".to_string(),
                kind: "local".to_string(),
            },
        ],
        frame_size: 3,
    };
    executable.body = LinkedExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![
                StmtRefIr { statement: 0 },
                StmtRefIr { statement: 1 },
                StmtRefIr { statement: 2 },
            ],
        }],
        statements: vec![
            LinkedStmtIr::Let {
                slot: 2,
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Assign {
                target: AssignTargetIr::ActorSelfField {
                    field: "count".to_string(),
                    field_type: integer(),
                },
                value: ExprRefIr { expression: 5 },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 6 }),
            },
        ],
        expressions: vec![
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::LoadSlot { slot: 1 },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActorDispatch {
                        plan: skiff_runtime_linked_program::LinkedActorMethodDispatchPlan {
                            declaration_owner: owner(),
                            actor_abi_identity: abi(),
                            actor_implementation_identity: implementation(),
                            method_identity: method_identity(),
                        },
                    },
                    vec![0, 1],
                ),
            },
            LinkedExprIr::ActorSelfField {
                field: "count".to_string(),
                field_type: integer(),
            },
            LinkedExprIr::LoadSlot { slot: 2 },
            LinkedExprIr::Binary {
                op: skiff_runtime_linked_program::BinaryOpIr::Add,
                left: ExprRefIr { expression: 3 },
                right: ExprRefIr { expression: 4 },
            },
            LinkedExprIr::ActorSelfField {
                field: "count".to_string(),
                field_type: integer(),
            },
        ],
    };
    Arc::new(file)
}

async fn execute_self_dispatch(
    fixture: &Fixture,
    interpreter: &Interpreter,
    actor: RecordingActor,
    value: i64,
) -> Result<Vec<u8>, ActorMethodExecutorError> {
    let arguments_payload = canonical_json_bytes(&json!([value])).expect("self-dispatch args");
    ActorMethodExecutor::new(&fixture.store)
        .execute(
            interpreter,
            ActorMethodExecutionRequest {
                instance: &fixture.handle,
                method_identity: &fixture.method,
                arguments_payload: &arguments_payload,
                context: program_context_with(
                    interpreter,
                    ActorCapabilityContext::new(actor),
                    test_runtime::request_context(),
                    test_runtime::file_context(),
                    DbCapabilityContext::unavailable(),
                ),
            },
        )
        .await
}

#[tokio::test]
async fn actor_method_executor_binds_pinned_self_for_ready_self_dispatch() {
    let fixture = fixture(integer(), true);
    let (interpreter, _) = interpreter_for(self_dispatch_executor_file());
    let actor = RecordingActor::ready(Ok(actor_return(5)));

    assert_eq!(
        execute_self_dispatch(&fixture, &interpreter, actor.clone(), 7)
            .await
            .expect("self dispatch must execute"),
        b"6"
    );

    let requests = actor.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.actor_ref.service_id(), "skiff.run/counter");
    assert_eq!(request.actor_ref.actor_type_identity(), "actors.Counter");
    assert_eq!(request.actor_ref.epoch(), Some(1));
    assert_eq!(request.identity.expected_epoch, 1);
    assert_eq!(request.arguments_payload, b"[7]");
}

#[tokio::test]
async fn actor_method_executor_self_dispatch_pending_releases_and_resumes_latest_fields() {
    let fixture = fixture(integer(), true);
    let (interpreter, _) = interpreter_for(self_dispatch_executor_file());
    let (pending_actor, release) = RecordingActor::pending();
    let ready_actor = RecordingActor::ready(Ok(actor_return(17)));

    let outer = execute_self_dispatch(&fixture, &interpreter, pending_actor.clone(), 12);
    let concurrent = async {
        while pending_actor.starts() == 0 {
            tokio::task::yield_now().await;
        }
        let result = execute_self_dispatch(&fixture, &interpreter, ready_actor, 4)
            .await
            .expect("concurrent actor segment");
        assert_eq!(result, b"18");
        release
            .send(Ok(actor_return(12)))
            .expect("release pending self dispatch");
    };
    let (outer_result, ()) = tokio::join!(outer, concurrent);

    assert_eq!(outer_result.expect("resumed self dispatch"), b"30");
    let count = fixture
        .store
        .with_fields_for_executor(
            &ActorExecutorAuthority::new(),
            &fixture.handle,
            |fields, _| fields[1].value.clone(),
        )
        .expect("read committed count");
    assert_eq!(count, RuntimeValue::Number(30.0));
}

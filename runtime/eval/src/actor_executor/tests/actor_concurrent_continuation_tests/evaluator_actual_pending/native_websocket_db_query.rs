use super::*;

#[tokio::test]
async fn f445h_e4r_spine_native_ready_first_poll_keeps_actor_segment() {
    let fixture = native_executable(
        native_target("std.time", "sleep", "std.time.sleep"),
        vec![LiteralIr::Number {
            value: serde_json::Number::from(0),
        }],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let flow = fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("zero sleep");

    assert!(matches!(flow, crate::env::Flow::Return(_)));
    assert!(
        frame.has_execution_lease(),
        "first-Ready native wait must keep the current segment"
    );
    frame.finish(heap).expect("finish ready native frame");
}

#[tokio::test]
async fn f445h_e4r_spine_native_pending_releases_and_reacquires_actor_segment() {
    let fixture = native_executable(
        native_target("std.time", "sleep", "std.time.sleep"),
        vec![LiteralIr::Number {
            value: serde_json::Number::from(50),
        }],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(
        !frame.has_execution_lease(),
        "first-Pending native wait must release the current segment"
    );
    tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("sleep completes")
        .expect("pending native call succeeds");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "native completion must reacquire before finalize"
    );
    frame.finish(heap).expect("finish pending native frame");
}

#[tokio::test]
async fn f445h_e4r_spine_websocket_send_sync_error_keeps_actor_segment() {
    let fixture = native_executable(
        native_target(
            "std.websocket",
            "sendTextToConnection",
            "std.websocket.sendTextToConnection",
        ),
        vec![
            LiteralIr::String {
                value: "connection".to_string(),
            },
            LiteralIr::String {
                value: "hello".to_string(),
            },
        ],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect_err("test websocket capability is deliberately unavailable");

    assert!(
        frame.has_execution_lease(),
        "synchronous WebSocket send must not cut the Actor segment"
    );
    frame.finish(heap).expect("finish websocket frame");
}

#[tokio::test]
async fn f445h_e4r_spine_db_query_is_first_poll_ready_and_keeps_actor_segment() {
    let fixture = EvaluatorFixture::new(
        vec![LinkedExprIr::DbQuery {
            target: DbTargetIr {
                type_ref: string_type(),
                type_name: "Thread".to_string(),
            },
            query: DbQueryIr::default(),
            projection: None,
            result_type: None,
        }],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let mut eval = fixture.eval_context_with(
        default_program_context(&fixture.interpreter),
        frame.clone(),
        &mut heap,
        &mut env,
        &addr,
    );
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(
        first_poll(execution.as_mut()),
        Poll::Ready(Ok(crate::env::Flow::Return(_)))
    ));
    drop(execution);
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "DbQuery only materializes query IR and must stay synchronous"
    );
    frame.finish(heap).expect("finish DbQuery frame");
}

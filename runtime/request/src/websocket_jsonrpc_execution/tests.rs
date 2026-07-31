use super::*;

#[test]
fn websocket_jsonrpc_execution_keeps_cancelled_outside_response_outcome() {
    let terminal = RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled;
    assert!(matches!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
    ));
    let failure = RuntimeWebSocketJsonRpcExecutionOutcome::InternalError;
    assert_eq!(failure.payload(), None);
}

#[test]
fn websocket_jsonrpc_execution_finalization_prefers_cancel_then_deadline() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let terminal = finalize_execution_terminal(
        RuntimeWebSocketJsonRpcExecutionTerminal::Response(
            RuntimeWebSocketJsonRpcExecutionOutcome::Success {
                payload: b"\"late\"".to_vec(),
            },
        ),
        Err(RuntimeError::Decode(
            "private-finalization-message".to_string(),
        )),
        &cancellation,
        Some(Instant::now()),
    );
    assert_eq!(
        terminal,
        RuntimeWebSocketJsonRpcExecutionTerminal::Cancelled
    );
    assert!(!format!("{terminal:?}").contains("private-finalization-message"));
}

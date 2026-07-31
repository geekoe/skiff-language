use super::*;

#[test]
fn websocket_jsonrpc_outcomes_preserve_success_null_and_omit_failure_payloads() {
    let (outcome, payload) =
        websocket_jsonrpc_response_parts(RuntimeWebSocketJsonRpcExecutionOutcome::Success {
            payload: b"null".to_vec(),
        });
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(payload, b"null");

    for terminal in [
        RuntimeWebSocketJsonRpcExecutionOutcome::InvalidParams,
        RuntimeWebSocketJsonRpcExecutionOutcome::InternalError,
        RuntimeWebSocketJsonRpcExecutionOutcome::DeadlineExceeded,
    ] {
        let (_, payload) = websocket_jsonrpc_response_parts(terminal);
        assert!(payload.is_empty());
    }
}

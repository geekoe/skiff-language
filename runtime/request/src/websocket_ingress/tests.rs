use std::num::NonZeroU32;

use skiff_runtime_capability_context::{
    WebSocketConnectionPolicyControl, WebSocketConnectionPolicyOverflowControl,
};
use skiff_runtime_eval::invocation::{
    EvalWebSocketAdapterResult, EvalWebSocketConnectResponse, EvalWebSocketConnectResult,
    EvalWebSocketContextCodec,
};

use super::*;
use crate::ResponseEvent;

fn boundary_end(response: BoundaryResponse) -> (Vec<u8>, Option<WebSocketConnectResponse>) {
    match response {
        BoundaryResponse::Event(ResponseEvent::End {
            payload,
            http_response,
            websocket_connect,
        }) => {
            assert_eq!(http_response, None);
            (payload, websocket_connect)
        }
        other => panic!("expected end response, got {other:?}"),
    }
}

#[test]
fn maps_accept_eval_result_to_request_boundary_response() {
    let response = EvalWebSocketAdapterResult {
        payload: vec![1, 2, 3],
        response: Some(EvalWebSocketConnectResponse {
            result: EvalWebSocketConnectResult::Accept,
            business_identity: Some("host-1".to_string()),
            connection_policy: Some(WebSocketConnectionPolicyControl {
                max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
                overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                close_code: None,
                close_reason: None,
            }),
            context_codec: Some(EvalWebSocketContextCodec {
                operation_abi_id: "abi.connect".to_string(),
                context_type_identity: "pkg.Context".to_string(),
            }),
            context_payload_present: true,
            code: None,
            reason: None,
        }),
    };

    let (payload, websocket_connect) = boundary_end(
        boundary_response_from_eval_websocket_adapter_result(response),
    );
    let websocket_connect = websocket_connect.expect("connect response should be present");

    assert_eq!(payload, vec![1, 2, 3]);
    assert_eq!(websocket_connect.result, "accept");
    assert_eq!(
        websocket_connect.business_identity,
        Some("host-1".to_string())
    );
    assert_eq!(
        websocket_connect.connection_policy,
        Some(WebSocketConnectionPolicyControl {
            max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
            overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
            close_code: None,
            close_reason: None,
        })
    );
    assert_eq!(
        websocket_connect.context_codec,
        Some(WebSocketContextCodec {
            operation_abi_id: "abi.connect".to_string(),
            context_type_identity: "pkg.Context".to_string(),
        })
    );
    assert!(websocket_connect.context_payload_present);
    assert_eq!(websocket_connect.code, None);
    assert_eq!(websocket_connect.reason, None);
}

#[test]
fn maps_reject_eval_result_to_request_boundary_response() {
    let response = EvalWebSocketAdapterResult {
        payload: Vec::new(),
        response: Some(EvalWebSocketConnectResponse {
            result: EvalWebSocketConnectResult::Reject,
            business_identity: None,
            connection_policy: None,
            context_codec: None,
            context_payload_present: false,
            code: Some(1008),
            reason: Some("policy".to_string()),
        }),
    };

    let (payload, websocket_connect) = boundary_end(
        boundary_response_from_eval_websocket_adapter_result(response),
    );
    let websocket_connect = websocket_connect.expect("connect response should be present");

    assert!(payload.is_empty());
    assert_eq!(websocket_connect.result, "reject");
    assert_eq!(websocket_connect.business_identity, None);
    assert_eq!(websocket_connect.connection_policy, None);
    assert_eq!(websocket_connect.context_codec, None);
    assert!(!websocket_connect.context_payload_present);
    assert_eq!(websocket_connect.code, Some(1008));
    assert_eq!(websocket_connect.reason, Some("policy".to_string()));
}

#[test]
fn maps_receive_eval_result_without_connect_response() {
    let response = EvalWebSocketAdapterResult {
        payload: vec![4, 5],
        response: None,
    };

    let (payload, websocket_connect) = boundary_end(
        boundary_response_from_eval_websocket_adapter_result(response),
    );

    assert_eq!(payload, vec![4, 5]);
    assert_eq!(websocket_connect, None);
}

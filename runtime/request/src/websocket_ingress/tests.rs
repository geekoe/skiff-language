use std::num::NonZeroU32;

use skiff_runtime_capability_context::{
    WebSocketConnectionPolicyControl, WebSocketConnectionPolicyOverflowControl,
};
use skiff_runtime_eval::{
    EvalRequestWebSocketAdapterResult, EvalRequestWebSocketConnectAccept,
    EvalRequestWebSocketConnectContext, EvalRequestWebSocketConnectReject,
    EvalRequestWebSocketContextCodec,
};

use super::*;
use crate::{ResponseEnd, ResponseEvent};

fn websocket_end(response: BoundaryResponse) -> WebSocketResponse {
    match response {
        BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::WebSocket(response))) => response,
        other => panic!("expected typed WebSocket response.end, got {other:?}"),
    }
}

#[test]
fn websocket_response_boundary_maps_typed_accept_with_zero_byte_context() {
    let policy = WebSocketConnectionPolicyControl {
        max_connections: NonZeroU32::new(1).expect("non-zero fixture"),
        overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
        close_code: None,
        close_reason: None,
    };
    let response =
        EvalRequestWebSocketAdapterResult::ConnectAccept(EvalRequestWebSocketConnectAccept {
            business_identity: Some("host-1".to_string()),
            connection_policy: Some(policy.clone()),
            context: EvalRequestWebSocketConnectContext::Typed {
                payload: Vec::new(),
                codec: EvalRequestWebSocketContextCodec {
                    operation_abi_id: "abi.connect".to_string(),
                    context_type_identity: "pkg.Context".to_string(),
                },
            },
        });

    let mapped = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Connect,
        response,
    )
    .expect("connect accept must map");
    assert_eq!(
        websocket_end(mapped),
        WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
            business_identity: Some("host-1".to_string()),
            connection_policy: Some(policy),
            context: WebSocketConnectContext::Typed {
                payload: Vec::new(),
                codec: WebSocketContextCodec {
                    operation_abi_id: "abi.connect".to_string(),
                    context_type_identity: "pkg.Context".to_string(),
                },
            },
        })
    );
}

#[test]
fn websocket_response_boundary_maps_null_accept_reject_and_receive() {
    let accept = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Connect,
        EvalRequestWebSocketAdapterResult::ConnectAccept(EvalRequestWebSocketConnectAccept {
            business_identity: None,
            connection_policy: None,
            context: EvalRequestWebSocketConnectContext::Null,
        }),
    )
    .expect("null Context accept must map");
    assert!(matches!(
        websocket_end(accept),
        WebSocketResponse::ConnectAccept(WebSocketConnectAccept {
            context: WebSocketConnectContext::Null,
            ..
        })
    ));

    let reject = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Connect,
        EvalRequestWebSocketAdapterResult::ConnectReject(EvalRequestWebSocketConnectReject {
            code: 1008,
            reason: "policy".to_string(),
        }),
    )
    .expect("connect reject must map");
    assert_eq!(
        websocket_end(reject),
        WebSocketResponse::ConnectReject(WebSocketConnectReject {
            code: 1008,
            reason: "policy".to_string(),
        })
    );

    let receive = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Receive,
        EvalRequestWebSocketAdapterResult::Receive,
    )
    .expect("receive must map");
    assert_eq!(websocket_end(receive), WebSocketResponse::Receive);
}

#[test]
fn websocket_response_boundary_rejects_phase_variant_mismatch() {
    let connect_error = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Connect,
        EvalRequestWebSocketAdapterResult::Receive,
    )
    .expect_err("connect may not emit a receive response");
    assert!(connect_error.to_string().contains("admitted request phase"));

    let receive_error = boundary_response_from_eval_websocket_adapter_result(
        WebSocketAdapterKind::Receive,
        EvalRequestWebSocketAdapterResult::ConnectReject(EvalRequestWebSocketConnectReject {
            code: 1008,
            reason: "policy".to_string(),
        }),
    )
    .expect_err("receive may not emit a connect response");
    assert!(receive_error.to_string().contains("admitted request phase"));
}

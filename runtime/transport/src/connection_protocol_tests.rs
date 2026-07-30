use serde_json::json;
use skiff_artifact_model::WebSocketEntryId;

use crate::{
    connection_protocol::{
        decode_connection_request_cancel_frame, decode_connection_request_frame,
        decode_connection_response_frame, encode_connection_request_cancel_frame,
        encode_connection_request_frame, encode_connection_response_frame,
        ConnectionRemoteErrorFrameHeader, ConnectionRequestCancelFrameHeader,
        ConnectionRequestFrameHeader, ConnectionResponseFrameHeader, ConnectionResponseOutcome,
        WebSocketRpcProfile,
    },
    protocol::{RuntimeDeadlineFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION},
};

fn websocket_entry_id() -> WebSocketEntryId {
    WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "a".repeat(64)
    ))
    .expect("canonical WebSocket entry id")
}

fn request_header() -> ConnectionRequestFrameHeader {
    ConnectionRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request".to_string(),
        request_id: "connection-request-1".to_string(),
        service_id: "example.com/chat".to_string(),
        websocket_entry_id: websocket_entry_id(),
        connection_id: "connection-1".to_string(),
        profile: WebSocketRpcProfile::JsonRpc2_0Text,
        method: "chat.send".to_string(),
        deadline: Some(RuntimeDeadlineFrameHeader {
            timeout_ms: 1000,
            expires_at: "2030-01-02T03:04:05Z".to_string(),
        }),
    }
}

#[test]
fn connection_request_strict_frame_requires_utf8_object_or_array_payload() {
    let header = request_header();
    for payload in [br#"{"message":"hi"}"#.as_slice(), br#"[1,2]"#.as_slice()] {
        let frame =
            encode_connection_request_frame(&header, payload).expect("request frame encodes");
        assert_eq!(
            decode_connection_request_frame(&frame).expect("request frame decodes"),
            (header.clone(), payload.to_vec())
        );
    }

    for payload in [
        b"".as_slice(),
        b"null".as_slice(),
        b"true".as_slice(),
        b"\"scalar\"".as_slice(),
        b"\xff".as_slice(),
    ] {
        assert!(
            encode_connection_request_frame(&header, payload).is_err(),
            "payload {payload:?} must fail closed"
        );
    }

    let mut unknown = serde_json::to_value(&header).expect("header value");
    unknown
        .as_object_mut()
        .expect("object")
        .insert("unknown".to_string(), json!(true));
    let frame = crate::protocol::encode_binary_frame(&unknown, br#"{}"#)
        .expect("untyped malformed frame encodes");
    assert!(decode_connection_request_frame(&frame).is_err());

    for expires_at in [
        "2030-02-30T03:04:05Z",
        "2030-01-02T03:04:05suffixZ",
        "2030-01-02T24:04:05Z",
        "2030-01-02T03:04:05+24:00",
    ] {
        let mut malformed = header.clone();
        malformed.deadline.as_mut().expect("deadline").expires_at = expires_at.to_string();
        assert!(
            encode_connection_request_frame(&malformed, br#"{}"#).is_err(),
            "invalid RFC3339 deadline {expires_at} must fail closed"
        );
    }
    let mut unsafe_timeout = header.clone();
    unsafe_timeout
        .deadline
        .as_mut()
        .expect("deadline")
        .timeout_ms = 9_007_199_254_740_992;
    assert!(
        encode_connection_request_frame(&unsafe_timeout, br#"{}"#).is_err(),
        "deadline timeoutMs must fit the JavaScript safe-integer domain"
    );
}

#[test]
fn connection_request_cancel_is_dedicated_and_payloadless() {
    let header = ConnectionRequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request.cancel".to_string(),
        request_id: "connection-request-1".to_string(),
        reason: crate::cancel_reason::RequestCancelReason::CallerCancel,
    };
    let frame = encode_connection_request_cancel_frame(&header).expect("cancel frame encodes");
    assert_eq!(
        decode_connection_request_cancel_frame(&frame).expect("cancel frame decodes"),
        header
    );

    let frame = crate::protocol::encode_binary_frame(&header, b"forbidden")
        .expect("untyped malformed frame encodes");
    assert!(decode_connection_request_cancel_frame(&frame).is_err());
}

#[test]
fn connection_response_payload_presence_matches_exact_outcome() {
    let success = ConnectionResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.response".to_string(),
        request_id: "connection-request-1".to_string(),
        outcome: ConnectionResponseOutcome::Success,
        remote: None,
    };
    let frame = encode_connection_response_frame(&success, b"null").expect("success frame encodes");
    assert_eq!(
        decode_connection_response_frame(&frame).expect("success frame decodes"),
        (success, b"null".to_vec())
    );

    let remote = ConnectionResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.response".to_string(),
        request_id: "connection-request-2".to_string(),
        outcome: ConnectionResponseOutcome::Remote,
        remote: Some(ConnectionRemoteErrorFrameHeader {
            code: -32603,
            message: " peer failed ".to_string(),
            data_present: true,
        }),
    };
    let frame = encode_connection_response_frame(&remote, b"null").expect("remote frame encodes");
    assert_eq!(
        decode_connection_response_frame(&frame).expect("remote frame decodes"),
        (remote, b"null".to_vec())
    );

    for malformed in [
        (
            ConnectionResponseFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "connection.response".to_string(),
                request_id: "bad-success".to_string(),
                outcome: ConnectionResponseOutcome::Success,
                remote: None,
            },
            b"".as_slice(),
        ),
        (
            ConnectionResponseFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "connection.response".to_string(),
                request_id: "bad-protocol".to_string(),
                outcome: ConnectionResponseOutcome::ProtocolError,
                remote: None,
            },
            b"payload".as_slice(),
        ),
        (
            ConnectionResponseFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "connection.response".to_string(),
                request_id: "bad-remote".to_string(),
                outcome: ConnectionResponseOutcome::Remote,
                remote: Some(ConnectionRemoteErrorFrameHeader {
                    code: 1,
                    message: "missing data".to_string(),
                    data_present: true,
                }),
            },
            b"".as_slice(),
        ),
    ] {
        assert!(encode_connection_response_frame(&malformed.0, malformed.1).is_err());
    }
}

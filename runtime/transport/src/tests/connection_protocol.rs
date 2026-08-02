use serde_json::json;
use skiff_artifact_model::WebSocketEntryId;

use crate::{
    connection_protocol::{
        classify_jsonrpc_20_text_frame, decode_connection_request_cancel_frame,
        decode_connection_request_frame, decode_connection_response_frame,
        encode_connection_request_cancel_frame, encode_connection_request_frame,
        encode_connection_response_frame, ClientSocketGeneration, ConnectionRemoteErrorFrameHeader,
        ConnectionRequestCancelFrameHeader, ConnectionRequestFrameHeader,
        ConnectionResponseFrameHeader, ConnectionResponseOutcome, JsonRpcPlatformErrorKind,
        OpaquePeerId, ProfileAction, WebSocketRpcProfile,
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

#[test]
fn client_socket_generation_newtype_requires_canonical_connection_id() {
    let generation = ClientSocketGeneration::new("connection-1", 7)
        .expect("canonical connection id must construct");
    assert_eq!(generation.connection_id, "connection-1");
    assert_eq!(generation.generation, 7);

    for invalid in [
        "",
        " connection",
        "connection ",
        "conn\u{0000}ection",
        &"c".repeat(1025),
    ] {
        assert!(
            ClientSocketGeneration::new(invalid, 0).is_err(),
            "connectionId {invalid:?} must fail closed"
        );
    }
}

#[test]
fn jsonrpc_text_classifier_canonicalizes_numeric_ids_without_float_roundtrip() {
    let request =
        |id: &str| format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"status.get","params":[]}}"#);
    let action = classify_jsonrpc_20_text_frame(request("1e0").as_bytes());
    assert_eq!(
        action,
        ProfileAction::Request {
            id: OpaquePeerId::SafeInteger(1),
            method: "status.get".to_string(),
        }
    );
    let ProfileAction::Request { id, .. } =
        classify_jsonrpc_20_text_frame(request("1e0").as_bytes())
    else {
        panic!("1e0 must classify as a request");
    };
    assert_eq!(id.canonical_key(), "n:1");

    for (lexeme, canonical) in [
        ("-0", "0"),
        ("-0.0e+3", "0"),
        ("1.000e2", "100"),
        ("1E+2", "100"),
        ("9007199254740991", "9007199254740991"),
        ("-9007199254740991", "-9007199254740991"),
    ] {
        let action = classify_jsonrpc_20_text_frame(request(lexeme).as_bytes());
        let ProfileAction::Request { id, .. } = action else {
            panic!("{lexeme} must classify as a request");
        };
        assert_eq!(id.canonical_key(), format!("n:{canonical}"), "{lexeme}");
    }

    for lexeme in [
        "1.5",
        "-0.5",
        "9007199254740992",
        "1e-324",
        "1.0000000000000000001",
        "1e21",
    ] {
        let action = classify_jsonrpc_20_text_frame(request(lexeme).as_bytes());
        assert_eq!(
            action,
            ProfileAction::PlatformError {
                kind: JsonRpcPlatformErrorKind::InvalidRequest
            },
            "{lexeme} must be rejected"
        );
    }
}

#[test]
fn jsonrpc_text_classifier_follows_frozen_profile_contract() {
    // String id request with object params.
    let action = classify_jsonrpc_20_text_frame(
        br#"{"jsonrpc":"2.0","id":"peer-1","method":"chat.send","params":{"n":1}}"#,
    );
    assert_eq!(
        action,
        ProfileAction::Request {
            id: OpaquePeerId::String("peer-1".to_string()),
            method: "chat.send".to_string(),
        }
    );

    // Notification (no id) is classified without terminal semantics.
    let action =
        classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","method":"chat.event","params":{}}"#);
    assert_eq!(
        action,
        ProfileAction::Notification {
            method: "chat.event".to_string()
        }
    );

    // Response with string id is accepted; numeric response id closes 1002.
    let action =
        classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","id":"peer-a","result":null}"#);
    assert_eq!(
        action,
        ProfileAction::Response {
            id: "peer-a".to_string()
        }
    );
    let action = classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","id":1,"result":null}"#);
    assert_eq!(action, ProfileAction::Close { code: 1002 });
    let action =
        classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","id":"a","result":null,"extra":true}"#);
    assert_eq!(action, ProfileAction::Close { code: 1002 });

    // Array/scalar/duplicate members are invalidRequest; leading-zero number
    // is a parse error; missing params is invalidParams.
    assert_eq!(
        classify_jsonrpc_20_text_frame(b"[1]"),
        ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidRequest
        }
    );
    assert_eq!(
        classify_jsonrpc_20_text_frame(b"null"),
        ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidRequest
        }
    );
    assert_eq!(
        classify_jsonrpc_20_text_frame(
            br#"{"jsonrpc":"2.0","id":"a","id":"b","method":"m","params":[]}"#
        ),
        ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidRequest
        }
    );
    assert_eq!(
        classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","id":01,"method":"m","params":[]}"#),
        ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::Parse
        }
    );
    assert_eq!(
        classify_jsonrpc_20_text_frame(br#"{"jsonrpc":"2.0","id":"a","method":"m"}"#),
        ProfileAction::PlatformError {
            kind: JsonRpcPlatformErrorKind::InvalidParams
        }
    );

    // Non-UTF-8 and oversize frames close 1009.
    assert_eq!(
        classify_jsonrpc_20_text_frame(b"\xff"),
        ProfileAction::Close { code: 1009 }
    );
    let oversize = vec![b' '; crate::connection_protocol::WEBSOCKET_JSONRPC_MAX_TEXT_BYTES + 1];
    assert_eq!(
        classify_jsonrpc_20_text_frame(&oversize),
        ProfileAction::Close { code: 1009 }
    );
}

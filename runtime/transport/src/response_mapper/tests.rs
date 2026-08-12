use serde_json::{json, Value};

use super::{
    response_event_into_frame, response_stream_event_into_frame,
    bytecode_websocket_jsonrpc_response_into_frame, validate_response_end_frame,
    OrdinaryResponseEvent, ResponseEndPhase,
};
use crate::protocol::{
    decode_binary_frame, decode_response_chunk_frame, decode_response_end_frame,
    decode_response_error_frame, decode_response_start_frame, decode_typed_binary_frame,
    ResponseEndFrameHeader, ResponseErrorFrameHeader, ValidatedResponseErrorFrame,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use crate::protocol::{
    decode_bytecode_websocket_jsonrpc_response_end_frame,
    BytecodeWebSocketJsonRpcResponseFrameHeader,
    BytecodeWebSocketJsonRpcResponseOutcome,
};
use skiff_runtime_request_contract::{
    FixedServiceResponseFailure, HttpResponseMetadata, OpaqueServiceError,
    OrdinaryResponseErrorSource, PlatformErrorProjectionPayload, ResponseEnd, ResponseError,
    ResponseEvent, ResponseStreamEvent, ServiceErrorEnvelope,
};

struct TestOrdinaryError(ResponseError);

impl OrdinaryResponseErrorSource for TestOrdinaryError {
    fn ordinary_response_error(&self) -> Option<ResponseError> {
        Some(self.0.clone())
    }
}

struct TestCancellationTerminal;

impl OrdinaryResponseErrorSource for TestCancellationTerminal {
    fn ordinary_response_error(&self) -> Option<ResponseError> {
        None
    }
}

fn ordinary_error(error: ResponseError) -> OrdinaryResponseEvent {
    OrdinaryResponseEvent::try_error(&TestOrdinaryError(error))
        .expect("test source is an ordinary failure")
}

#[test]
fn response_boundary_rejects_http_and_payload_phase_confusion() {
    let http = response_event_into_frame(
        "request-http".to_string(),
        OrdinaryResponseEvent::End(ResponseEnd::Http {
            payload: Vec::new(),
            metadata: HttpResponseMetadata::new(204, Vec::new()),
        }),
    )
    .expect("HTTP response must encode");
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&http).expect("HTTP response.end must decode");
    assert!(validate_response_end_frame(&header, &payload, ResponseEndPhase::Http).is_ok());
    assert!(validate_response_end_frame(&header, &payload, ResponseEndPhase::Payload).is_err());
}

#[test]
fn canonical_stream_response_mapper_round_trips_start_chunk_and_end() {
    let request_id = "request-stream".to_string();

    let start = response_stream_event_into_frame(
        &request_id,
        ResponseStreamEvent::Start {
            http_response: HttpResponseMetadata::new(200, Vec::new()),
        },
    )
    .expect("stream start must encode");
    let start_header = decode_response_start_frame(&start).expect("stream start must decode");
    assert_eq!(start_header.request_id, request_id);
    assert_eq!(start_header.http_response.status, 200);

    let chunk = response_stream_event_into_frame(
        &request_id,
        ResponseStreamEvent::Chunk {
            seq: 7,
            payload: b"body".to_vec(),
        },
    )
    .expect("stream chunk must encode");
    let (chunk_header, chunk_payload) =
        decode_response_chunk_frame(&chunk).expect("stream chunk must decode");
    assert_eq!(chunk_header.request_id, request_id);
    assert_eq!(chunk_header.seq, 7);
    assert_eq!(chunk_payload, b"body");

    let end = response_stream_event_into_frame(&request_id, ResponseStreamEvent::End)
        .expect("stream end must encode");
    let (end_header, end_payload) =
        decode_response_end_frame(&end).expect("stream end must decode");
    assert_eq!(end_header.request_id, request_id);
    assert!(!end_header.payload_present);
    assert!(end_payload.is_empty());
}

#[test]
fn service_error_response_v2_mapper_round_trip_preserves_fixed_payload_bytes() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../testdata/service-error-response-v2.json"
    ))
    .expect("service error response v2 corpus must decode");
    for test_case in corpus["validCases"]
        .as_array()
        .expect("validCases must be an array")
        .iter()
        .filter(|test_case| test_case["expected"]["kind"] != "control")
    {
        let payload = test_case["payloadUtf8"]
            .as_str()
            .expect("fixture payload")
            .as_bytes()
            .to_vec();
        let error = OpaqueServiceError::decode(payload.clone()).expect("fixture fixed error");
        let request_id = test_case["header"]["requestId"]
            .as_str()
            .expect("fixture request id");

        let encoded = response_event_into_frame(
            request_id.to_string(),
            OrdinaryResponseEvent::FixedServiceFailure(FixedServiceResponseFailure::new(error)),
        )
        .expect("fixed service response must encode");
        let (header, decoded_body) =
            decode_response_error_frame(&encoded).expect("fixed service response must decode");
        let decoded = match decoded_body {
            ValidatedResponseErrorFrame::FixedService(decoded) => decoded,
            ValidatedResponseErrorFrame::Control(_) => {
                panic!("{} must stay a fixed service error", test_case["name"])
            }
        };
        assert_eq!(decoded.encoded_bytes(), payload);

        if let ServiceErrorEnvelope::PlatformError {
            encoded_payload, ..
        } = decoded.envelope()
        {
            let known = test_case["expected"]["known"]
                .as_bool()
                .expect("platform fixture known flag");
            if known {
                let evidence = decoded
                    .known_platform_projection()
                    .expect("exact-known platform error must expose typed evidence");
                assert!(matches!(
                    evidence.payload(),
                    PlatformErrorProjectionPayload::StdCollectionMapKeyNotFoundError(_)
                ));
                assert_eq!(encoded_payload.as_slice(), b"{}");
            } else {
                assert!(decoded.known_platform_projection().is_none());
                assert!(
                    serde_json::from_slice::<Value>(encoded_payload).is_err(),
                    "unknown payload fixture must prove transport never requires JSON"
                );
            }
        }

        let raw = decode_binary_frame(&encoded).expect("fixed service binary frame");
        assert_eq!(raw.payload_bytes, payload);
        assert!(matches!(
            header,
            ResponseErrorFrameHeader::FixedService { .. }
        ));
    }
}

#[test]
fn service_error_response_v2_mapper_keeps_matching_generic_control_untyped() {
    let encoded = response_event_into_frame(
        "request-control-1".to_string(),
        ordinary_error(ResponseError {
            code: "InternalError".to_string(),
            message: "The service could not complete the request.".to_string(),
            status: Some(500),
            details: Some(json!({ "traceId": "trace-control-only" })),
        }),
    )
    .expect("control response must encode");
    let (_header, decoded_body) =
        decode_response_error_frame(&encoded).expect("control response must decode");
    assert!(matches!(
        decoded_body,
        ValidatedResponseErrorFrame::Control(ref error)
            if error.code == "InternalError"
    ));
}

#[test]
fn cancellation_terminal_cannot_be_encoded_as_response_error_but_ordinary_failures_can() {
    let cancelled = OrdinaryResponseEvent::try_error(&TestCancellationTerminal);
    assert!(
        cancelled.is_none(),
        "internal cancellation must fail closed before response.error encoding"
    );
    let unproven = OrdinaryResponseEvent::try_from_non_error(ResponseEvent::Error(ResponseError {
        code: "UnprovenError".to_string(),
        message: "unproven raw error".to_string(),
        status: None,
        details: None,
    }));
    assert!(
        unproven.is_err(),
        "raw response.error must also fail closed"
    );

    for (request_id, code) in [
        ("request-timeout", "TimeoutError"),
        (
            "request-provider-unavailable",
            "std.service.ProviderUnavailableError",
        ),
    ] {
        let encoded = response_event_into_frame(
            request_id.to_string(),
            ordinary_error(ResponseError {
                code: code.to_string(),
                message: format!("{code} message"),
                status: None,
                details: None,
            }),
        )
        .expect("ordinary failure must encode");
        let (header, decoded_body) =
            decode_response_error_frame(&encoded).expect("ordinary response.error must decode");
        assert!(matches!(
            decoded_body,
            ValidatedResponseErrorFrame::Control(ref error) if error.code == code
        ));
        assert_eq!(header.request_id(), request_id);
    }
}

#[test]
fn websocket_response_wire_raw_optional_bag_shapes_are_rejected() {
    let legacy = json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "response.end",
        "requestId": "legacy-optional-bag",
        "payloadPresent": false,
        "websocketConnect": {
            "result": "accept",
            "contextPayloadPresent": false,
            "code": 1008,
            "reason": "illegal reject fields on accept"
        }
    });
    assert!(serde_json::from_value::<ResponseEndFrameHeader>(legacy).is_err());
}

#[test]
fn bytecode_websocket_jsonrpc_mapper_round_trips_opaque_success_payload() {
    let payload = b"null".to_vec();
    let encoded = bytecode_websocket_jsonrpc_response_into_frame(
        "request-websocket-jsonrpc-mapper".to_string(),
        BytecodeWebSocketJsonRpcResponseFrameHeader {
            outcome: BytecodeWebSocketJsonRpcResponseOutcome::Success,
        },
        payload.clone(),
    )
    .expect("success must encode");
    let (decoded, decoded_payload) =
        decode_bytecode_websocket_jsonrpc_response_end_frame(&encoded)
            .expect("mapped response must decode");

    assert_eq!(decoded.request_id, "request-websocket-jsonrpc-mapper");
    assert_eq!(
        decoded.websocket_json_rpc.outcome,
        BytecodeWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(decoded_payload, payload);
}

#[test]
fn bytecode_websocket_jsonrpc_mapper_rejects_outcome_payload_mismatch() {
    assert!(bytecode_websocket_jsonrpc_response_into_frame(
        "request-success-without-payload".to_string(),
        BytecodeWebSocketJsonRpcResponseFrameHeader {
            outcome: BytecodeWebSocketJsonRpcResponseOutcome::Success,
        },
        Vec::new(),
    )
    .is_err());
    assert!(bytecode_websocket_jsonrpc_response_into_frame(
        "request-error-with-payload".to_string(),
        BytecodeWebSocketJsonRpcResponseFrameHeader {
            outcome: BytecodeWebSocketJsonRpcResponseOutcome::InternalError,
        },
        b"null".to_vec(),
    )
    .is_err());
    assert!(bytecode_websocket_jsonrpc_response_into_frame(
        " invalid-request-id ".to_string(),
        BytecodeWebSocketJsonRpcResponseFrameHeader {
            outcome: BytecodeWebSocketJsonRpcResponseOutcome::InternalError,
        },
        Vec::new(),
    )
    .is_err());
}

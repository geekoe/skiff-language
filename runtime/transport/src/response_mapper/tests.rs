use serde_json::{json, Value};

use super::{
    response_end_to_outbound, response_error_to_outbound, response_event_into_frame,
    validate_response_end_frame, OrdinaryResponseErrorSource, OrdinaryResponseEvent,
    ResponseEndPhase,
};
use crate::protocol::{
    decode_binary_frame, decode_response_error_frame, decode_typed_binary_frame,
    ResponseEndFrameHeader, ResponseEndFrameMetadata, ResponseErrorFrameHeader,
    ValidatedResponseErrorFrame, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_model::service_error::OpaqueServiceError;
use skiff_runtime_request_contract::{
    FixedServiceResponseFailure, HttpResponseMetadata, OutboundResponse, ResponseEnd,
    ResponseError, ResponseEvent,
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

    let inbound = response_end_to_outbound(
        &ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "response.end".to_string(),
            request_id: "request-payload-mismatch".to_string(),
            payload_present: false,
            metadata: ResponseEndFrameMetadata::None,
        },
        vec![1],
    );
    assert!(matches!(
        inbound,
        OutboundResponse::Error(error) if error.code == "RuntimeProtocolViolation"
    ));
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
        .take(3)
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
        assert!(matches!(
            decoded_body,
            ValidatedResponseErrorFrame::FixedService(ref decoded)
                if decoded.encoded_bytes() == payload
        ));

        let raw = decode_binary_frame(&encoded).expect("fixed service binary frame");
        assert_eq!(raw.payload_bytes, payload);
        let typed_header: ResponseErrorFrameHeader =
            serde_json::from_value(raw.header).expect("fixed service header");
        let outbound = response_error_to_outbound(&typed_header, raw.payload_bytes);
        assert!(matches!(
            outbound,
            OutboundResponse::FixedServiceFailure(failure)
                if failure.error().encoded_bytes() == payload
        ));
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
    let (header, decoded_body) =
        decode_response_error_frame(&encoded).expect("control response must decode");
    assert!(matches!(
        decoded_body,
        ValidatedResponseErrorFrame::Control(ref error)
            if error.code == "InternalError"
    ));

    let outbound = response_error_to_outbound(&header, Vec::new());
    assert!(matches!(
        outbound,
        OutboundResponse::Error(error)
            if error.code == "InternalError"
                && error.message == "The service could not complete the request."
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

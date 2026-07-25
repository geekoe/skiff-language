use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    response_end_to_outbound, response_error_to_outbound, response_event_into_frame,
    validate_response_end_frame, ResponseEndPhase,
};
use crate::protocol::{
    decode_binary_frame, decode_response_error_frame, decode_typed_binary_frame,
    encode_binary_frame, ResponseEndFrameHeader, ResponseEndFrameMetadata,
    ResponseErrorFrameHeader, RuntimeWebSocketConnectContextFrameHeader,
    RuntimeWebSocketResponseFrameHeader, ValidatedResponseErrorFrame, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_model::service_error::OpaqueServiceError;
use skiff_runtime_request_contract::{
    FixedServiceResponseFailure, HttpResponseMetadata, OutboundResponse, ResponseEnd,
    ResponseError, ResponseEvent, WebSocketConnectAccept, WebSocketConnectContext,
    WebSocketConnectReject, WebSocketContextCodec, WebSocketResponse,
};

#[test]
fn websocket_response_boundary_preserves_nominal_zero_byte_context() {
    let frame = response_event_into_frame(
        "request-zero-context".to_string(),
        ResponseEvent::End(ResponseEnd::WebSocket(WebSocketResponse::ConnectAccept(
            WebSocketConnectAccept {
                business_identity: Some("business-1".to_string()),
                connection_policy: None,
                context: WebSocketConnectContext::Typed {
                    payload: Vec::new(),
                    codec: WebSocketContextCodec {
                        operation_abi_id: "operation-abi-1".to_string(),
                        context_type_identity: "context-type-1".to_string(),
                    },
                },
            },
        ))),
    )
    .expect("typed zero-byte Context must encode");
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("response.end must decode");

    assert!(payload.is_empty());
    assert!(header.payload_present);
    assert!(matches!(
        header.metadata,
        ResponseEndFrameMetadata::WebSocketConnect(
            RuntimeWebSocketResponseFrameHeader::ConnectAccept(accept)
        ) if matches!(
            &accept.context,
            RuntimeWebSocketConnectContextFrameHeader::Typed(codec)
                if codec.operation_abi_id == "operation-abi-1"
                    && codec.context_type_identity == "context-type-1"
        )
    ));
}

#[test]
fn websocket_response_boundary_keeps_accept_reject_receive_discriminated() {
    let cases = [
        (
            ResponseEvent::End(ResponseEnd::WebSocket(WebSocketResponse::ConnectAccept(
                WebSocketConnectAccept {
                    business_identity: None,
                    connection_policy: None,
                    context: WebSocketConnectContext::Null,
                },
            ))),
            "accept",
        ),
        (
            ResponseEvent::End(ResponseEnd::WebSocket(WebSocketResponse::ConnectReject(
                WebSocketConnectReject {
                    code: 1008,
                    reason: "policy".to_string(),
                },
            ))),
            "reject",
        ),
        (
            ResponseEvent::End(ResponseEnd::WebSocket(WebSocketResponse::Receive)),
            "receive",
        ),
    ];

    for (index, (event, expected)) in cases.into_iter().enumerate() {
        let frame = response_event_into_frame(format!("request-{index}"), event)
            .expect("typed WebSocket response must encode");
        let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("response.end must decode");
        assert!(payload.is_empty());
        match (expected, header.metadata) {
            (
                "accept",
                ResponseEndFrameMetadata::WebSocketConnect(
                    RuntimeWebSocketResponseFrameHeader::ConnectAccept(accept),
                ),
            ) => assert!(matches!(
                accept.context,
                RuntimeWebSocketConnectContextFrameHeader::Null
            )),
            (
                "reject",
                ResponseEndFrameMetadata::WebSocketConnect(
                    RuntimeWebSocketResponseFrameHeader::ConnectReject(reject),
                ),
            ) => {
                assert_eq!(reject.code, 1008);
                assert_eq!(reject.reason, "policy");
            }
            ("receive", ResponseEndFrameMetadata::None) => {}
            other => panic!("unexpected typed response boundary {other:?}"),
        }
    }
}

#[test]
fn websocket_response_boundary_rejects_http_and_payload_phase_confusion() {
    let http = response_event_into_frame(
        "request-http".to_string(),
        ResponseEvent::End(ResponseEnd::Http {
            payload: Vec::new(),
            metadata: HttpResponseMetadata::new(204, Vec::new()),
        }),
    )
    .expect("HTTP response must encode");
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&http).expect("HTTP response.end must decode");
    assert!(validate_response_end_frame(&header, &payload, ResponseEndPhase::Http).is_ok());
    assert!(
        validate_response_end_frame(&header, &payload, ResponseEndPhase::WebSocketConnect).is_err()
    );

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
            ResponseEvent::FixedServiceFailure(FixedServiceResponseFailure::new(error)),
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
        ResponseEvent::Error(ResponseError {
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
fn canonical_response_corpus_accepts_goldens_and_rejects_mutations() {
    let corpus: ResponseCorpus = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json"
    )))
    .expect("canonical response corpus must parse");

    assert_eq!(corpus.response_end_cases.len(), 6);
    assert!(corpus.response_end_mutations.len() >= 18);
    for case in &corpus.response_end_cases {
        let payload = decode_hex(&case.payload_hex);
        let frame = encode_binary_frame(&case.header, &payload)
            .unwrap_or_else(|error| panic!("{} must encode: {error}", case.name));
        let (header, decoded): (ResponseEndFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame)
                .unwrap_or_else(|error| panic!("{} must decode: {error}", case.name));
        assert_eq!(decoded, payload, "{} payload", case.name);
        validate_response_end_frame(&header, &payload, case.phase())
            .unwrap_or_else(|error| panic!("{} must validate: {error}", case.name));
    }

    for mutation in &corpus.response_end_mutations {
        let base = &corpus.response_end_cases[mutation.base_index];
        let mut header = base.header.clone();
        if let Some(path) = &mutation.set_path {
            set_path(&mut header, path, mutation.value.clone());
        }
        if let Some(path) = &mutation.remove_path {
            remove_path(&mut header, path);
        }
        let payload = mutation
            .payload_hex
            .as_deref()
            .map(decode_hex)
            .unwrap_or_else(|| decode_hex(&base.payload_hex));
        let rejected = serde_json::from_value::<ResponseEndFrameHeader>(header)
            .map(|typed| validate_response_end_frame(&typed, &payload, base.phase()).is_err())
            .unwrap_or(true);
        assert!(rejected, "mutation must fail closed: {}", mutation.name);
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseCorpus {
    response_end_cases: Vec<ResponseCase>,
    response_end_mutations: Vec<ResponseMutation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseCase {
    name: String,
    phase: String,
    header: Value,
    payload_hex: String,
}

impl ResponseCase {
    fn phase(&self) -> ResponseEndPhase {
        match self.phase.as_str() {
            "payload" => ResponseEndPhase::Payload,
            "http" => ResponseEndPhase::Http,
            "webSocketConnect" => ResponseEndPhase::WebSocketConnect,
            "webSocketReceive" => ResponseEndPhase::WebSocketReceive,
            phase => panic!("unsupported response corpus phase {phase}"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResponseMutation {
    name: String,
    base_index: usize,
    set_path: Option<String>,
    remove_path: Option<String>,
    #[serde(default)]
    value: Value,
    payload_hex: Option<String>,
}

fn decode_hex(input: &str) -> Vec<u8> {
    assert_eq!(input.len() % 2, 0, "hex fixture length");
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex fixture must be UTF-8");
            u8::from_str_radix(text, 16).expect("hex fixture byte")
        })
        .collect()
}

fn set_path(root: &mut Value, path: &str, value: Value) {
    let mut segments = path.split('.').peekable();
    let mut current = root;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current
                .as_object_mut()
                .expect("mutation parent must be object")
                .insert(segment.to_string(), value);
            return;
        }
        current = current
            .get_mut(segment)
            .unwrap_or_else(|| panic!("mutation path missing segment {segment}"));
    }
}

fn remove_path(root: &mut Value, path: &str) {
    let mut segments = path.split('.').peekable();
    let mut current = root;
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current
                .as_object_mut()
                .expect("mutation parent must be object")
                .remove(segment)
                .unwrap_or_else(|| panic!("mutation path missing field {segment}"));
            return;
        }
        current = current
            .get_mut(segment)
            .unwrap_or_else(|| panic!("mutation path missing segment {segment}"));
    }
}

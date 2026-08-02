//! W-model-request corpus gate.
//!
//! Consumes the frozen C-model-request corpus
//! (`testdata/request-wire/`) through the W-model frame codecs and proves
//! the target request wire bytes roundtrip exactly:
//! `encode(decode(frameHex)) == frameHex`. W-model also flips the
//! `currentEnforced` gaps frozen by the contract pack: `request.cancel`
//! payload must be empty, its reason must be a `CONTRACT_H` wire value, and
//! `response.end` `payloadPresent` must match the payload presence.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::{
    decode_request_cancel_frame, decode_response_chunk_frame, decode_response_end_frame,
    decode_response_start_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_request_cancel_frame, encode_response_chunk_frame, encode_response_end_frame,
    encode_response_start_frame, request_frame_rule, validate_response_error_frame, FrameDirection,
    RequestCancelFrameHeader, RequestFrameKind, RequestFramePayloadPresence,
    ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseErrorFrameHeader,
    ResponseStartFrameHeader, REQUEST_CANCEL_FRAME_TYPE, REQUEST_START_FRAME_TYPE,
    RESPONSE_CHUNK_FRAME_TYPE, RESPONSE_END_FRAME_TYPE, RESPONSE_ERROR_FRAME_TYPE,
    RESPONSE_START_FRAME_TYPE, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameWireHeader,
};

const REQUIRED_FRAMES: [&str; 12] = [
    "start.unary.req1",
    "start.stream.req2",
    "cancel.req1.timeout",
    "response.start.req2",
    "response.start.req1.unexpected",
    "response.chunk.req2.seq0",
    "response.chunk.req2.seq1",
    "response.chunk.req2.seq2",
    "response.end.req1.payload",
    "response.end.req2.empty",
    "response.error.req1.control",
    "response.error.req1.fixed-service",
];

const REQUIRED_SCENARIOS: [&str; 13] = [
    "unary-response-end",
    "unary-response-error-control",
    "unary-response-error-fixed-service",
    "stream-start-chunk-chunk-end",
    "stream-end-before-start-rejected",
    "stream-chunk-before-start-rejected",
    "stream-chunk-seq-gap-rejected",
    "stream-duplicate-start-rejected",
    "stream-start-on-unary-rejected",
    "stream-end-with-payload-rejected",
    "request-cancel-router-to-runtime",
    "request-cancel-runtime-to-router",
    "stale-response-ignored",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadRule")]
    payload_rule: String,
    #[serde(rename = "payloadHex")]
    payload_hex: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    #[allow(dead_code)]
    // consumed by serde (deny_unknown_fields); semantically covered by frameHex.
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameCatalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    #[serde(rename = "sharedCorpus")]
    shared_corpus: String,
    #[serde(rename = "cancelReasons")]
    cancel_reasons: Vec<String>,
    frames: BTreeMap<String, FrameEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RejectCase {
    id: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "expectErrorContains")]
    expect_error_contains: String,
    json: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RejectCatalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    cases: Vec<RejectCase>,
}

fn frame_catalog() -> FrameCatalog {
    serde_json::from_str(include_str!("../testdata/request-wire/frames.json"))
        .expect("request-wire frames.json must decode")
}

fn reject_catalog() -> RejectCatalog {
    serde_json::from_str(include_str!("../testdata/request-wire/reject-cases.json"))
        .expect("request-wire reject-cases.json must decode")
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/request-wire")
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_frame_rules_are_frozen() {
        let catalog = frame_catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "request-wire-v1");
        assert_eq!(
            catalog.shared_corpus,
            "cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json"
        );
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        let contract_reasons: Vec<String> = RequestCancelReason::CONTRACT_H
            .iter()
            .map(|reason| reason.as_str().to_string())
            .collect();
        assert_eq!(catalog.cancel_reasons, contract_reasons);

        for (name, entry) in &catalog.frames {
            let rule = request_frame_rule(&entry.frame_type)
                .unwrap_or_else(|| panic!("{name} frame type {} must classify", entry.frame_type));
            match entry.frame_type.as_str() {
                REQUEST_START_FRAME_TYPE => {
                    assert_eq!(entry.direction, "RouterToRuntime", "{name}");
                    assert_eq!(entry.payload_rule, "optional", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::Start);
                    assert_eq!(rule.direction, FrameDirection::RouterToRuntime);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Optional);
                }
                REQUEST_CANCEL_FRAME_TYPE => {
                    assert_eq!(entry.direction, "Either", "{name}");
                    assert_eq!(entry.payload_rule, "empty", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::Cancel);
                    assert_eq!(rule.direction, FrameDirection::Either);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Empty);
                }
                RESPONSE_START_FRAME_TYPE => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_eq!(entry.payload_rule, "empty", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::ResponseStart);
                    assert_eq!(rule.direction, FrameDirection::RuntimeToRouter);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Empty);
                }
                RESPONSE_CHUNK_FRAME_TYPE => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_eq!(entry.payload_rule, "optional", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::ResponseChunk);
                    assert_eq!(rule.direction, FrameDirection::RuntimeToRouter);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Optional);
                }
                RESPONSE_END_FRAME_TYPE => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::ResponseEnd);
                    assert_eq!(rule.direction, FrameDirection::RuntimeToRouter);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Optional);
                }
                RESPONSE_ERROR_FRAME_TYPE => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_eq!(rule.kind, RequestFrameKind::ResponseError);
                    assert_eq!(rule.direction, FrameDirection::RuntimeToRouter);
                    assert_eq!(rule.payload_presence, RequestFramePayloadPresence::Variant);
                }
                other => panic!("{name} has unknown frame type {other}"),
            }
        }

        for unknown in ["", "request.bogus", "response.bogus", "runtime.health"] {
            assert_eq!(
                request_frame_rule(unknown),
                None,
                "{unknown} must not classify"
            );
        }
    }

    #[test]
    fn frames_round_trip_byte_exact_through_w_model_codecs() {
        let catalog = frame_catalog();
        for (name, entry) in &catalog.frames {
            let expected_bytes = decode_hex(&entry.frame_hex);
            let expected_payload = decode_hex(&entry.payload_hex);
            let reencoded = match entry.decode_as.as_str() {
                "RequestStartHttpUnary" | "RequestStartHttpStream" => {
                    let (header, payload) =
                        decode_runtime_assembly_request_start_frame(&expected_bytes)
                            .unwrap_or_else(|error| panic!("{name} start decode: {error}"));
                    let mode = match &header {
                        RuntimeAssemblyRequestStartFrameWireHeader::Http(http) => {
                            http.mode.as_str()
                        }
                        other => panic!("{name} must decode as HTTP start, got {other:?}"),
                    };
                    let expected_mode = if entry.decode_as == "RequestStartHttpUnary" {
                        "unary"
                    } else {
                        "serverStream"
                    };
                    assert_eq!(mode, expected_mode, "{name} mode");
                    assert_eq!(payload, expected_payload, "{name} payload");
                    encode_binary_frame(&header, &payload).expect("start must encode")
                }
                "RequestCancel" => {
                    let header = decode_request_cancel_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} cancel decode: {error}"));
                    assert_eq!(header.envelope_type, REQUEST_CANCEL_FRAME_TYPE, "{name}");
                    assert_eq!(entry.payload_rule, "empty", "{name}");
                    assert!(
                        RequestCancelReason::from_contract_h_wire(&header.reason).is_some(),
                        "{name} reason {} must be CONTRACT_H",
                        header.reason
                    );
                    encode_request_cancel_frame(&header).expect("cancel must encode")
                }
                "ResponseStart" => {
                    let header = decode_response_start_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.start decode: {error}"));
                    assert_eq!(header.envelope_type, RESPONSE_START_FRAME_TYPE, "{name}");
                    encode_response_start_frame(&header).expect("response.start must encode")
                }
                "ResponseChunk" => {
                    let (header, payload) = decode_response_chunk_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.chunk decode: {error}"));
                    assert_eq!(header.envelope_type, RESPONSE_CHUNK_FRAME_TYPE, "{name}");
                    assert_eq!(payload, expected_payload, "{name} chunk payload");
                    encode_response_chunk_frame(&header, &payload)
                        .expect("response.chunk must encode")
                }
                "ResponseEnd" => {
                    let (header, payload) = decode_response_end_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.end decode: {error}"));
                    assert_eq!(header.envelope_type, RESPONSE_END_FRAME_TYPE, "{name}");
                    assert_eq!(payload, expected_payload, "{name} response.end payload");
                    encode_response_end_frame(&header, &payload).expect("response.end must encode")
                }
                "ResponseErrorControl" | "ResponseErrorFixedService" => {
                    let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&expected_bytes).unwrap_or_else(|error| {
                            panic!("{name} response.error decode: {error}")
                        });
                    let validated = validate_response_error_frame(&header, payload.clone())
                        .unwrap_or_else(|error| panic!("{name} response.error validate: {error}"));
                    let expected_variant = if entry.decode_as == "ResponseErrorControl" {
                        "control"
                    } else {
                        "fixedService"
                    };
                    let actual_variant = match &header {
                        ResponseErrorFrameHeader::FixedService { .. } => "fixedService",
                        ResponseErrorFrameHeader::Control { .. } => "control",
                    };
                    assert_eq!(actual_variant, expected_variant, "{name}");
                    if expected_variant == "fixedService" {
                        match &validated {
                            skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(
                                error,
                            ) => {
                                assert_eq!(error.encoded_bytes(), payload.as_slice(), "{name}");
                            }
                            _ => panic!("{name} must validate as fixedService"),
                        }
                    }
                    assert_eq!(payload, expected_payload, "{name} response.error payload");
                    encode_binary_frame(&header, &payload).expect("response.error must encode")
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            };
            assert_eq!(
                reencoded, expected_bytes,
                "{name} must roundtrip byte-exact through W-model codecs"
            );
        }
    }

    #[test]
    fn reject_cases_fail_closed_with_expected_errors() {
        let catalog = reject_catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "request-wire-v1-reject");
        assert!(!catalog.cases.is_empty());
        for case in &catalog.cases {
            assert_eq!(case.decode_as, "RequestStartHttpUnary", "{}", case.id);
            let frame = encode_binary_frame(&case.json, &[])
                .unwrap_or_else(|error| panic!("{} must encode: {error}", case.id));
            let result = decode_runtime_assembly_request_start_frame(&frame);
            let message = match result {
                Ok((_, _)) => panic!("{} must be rejected", case.id),
                Err(error) => error.to_string(),
            };
            assert!(
                message.contains(&case.expect_error_contains),
                "{} must fail with {:?}, got {message}",
                case.id,
                case.expect_error_contains
            );
        }
    }

    #[test]
    fn w_model_enforces_cancel_and_response_frame_payload_rules() {
        let cancel = RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: REQUEST_CANCEL_FRAME_TYPE.to_string(),
            request_id: "req-1".to_string(),
            reason: "timeout".to_string(),
        };
        let cancel_with_payload =
            encode_binary_frame(&cancel, b"not-empty").expect("cancel with payload must encode");
        let error = decode_request_cancel_frame(&cancel_with_payload)
            .expect_err("request.cancel payload must be enforced")
            .to_string();
        assert!(error.contains("payload must be empty"), "{error}");

        for reason in ["nonsense", "gateway_disconnect", "drain"] {
            let header = RequestCancelFrameHeader {
                reason: reason.to_string(),
                ..cancel.clone()
            };
            let frame = encode_binary_frame(&header, &[]).expect("cancel header must encode");
            let error = decode_request_cancel_frame(&frame)
                .expect_err("non-CONTRACT_H reason must be rejected")
                .to_string();
            assert!(
                error.contains("CONTRACT_H wire reasons"),
                "{reason} must fail with CONTRACT_H error, got {error}"
            );
        }

        let empty_request_id = RequestCancelFrameHeader {
            request_id: "  ".to_string(),
            ..cancel.clone()
        };
        let frame = encode_binary_frame(&empty_request_id, &[])
            .expect("empty requestId header must encode");
        let error = decode_request_cancel_frame(&frame)
            .expect_err("empty requestId must be rejected")
            .to_string();
        assert!(error.contains("requestId must be non-empty"), "{error}");

        let start = ResponseStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: RESPONSE_START_FRAME_TYPE.to_string(),
            request_id: "req-2".to_string(),
            http_response: skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader {
                status: 200,
                headers: Vec::new(),
            },
        };
        let start_with_payload =
            encode_binary_frame(&start, b"x").expect("response.start with payload must encode");
        let error = decode_response_start_frame(&start_with_payload)
            .expect_err("response.start payload must be enforced")
            .to_string();
        assert!(error.contains("payload must be empty"), "{error}");

        let end = ResponseEndFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: RESPONSE_END_FRAME_TYPE.to_string(),
            request_id: "req-1".to_string(),
            payload_present: true,
            metadata: skiff_runtime_transport::protocol::ResponseEndFrameMetadata::None,
        };
        let end_true_empty =
            encode_binary_frame(&end, &[]).expect("response.end with empty payload must encode");
        let error = decode_response_end_frame(&end_true_empty)
            .expect_err("payloadPresent true with empty payload must be rejected")
            .to_string();
        assert!(error.contains("payloadPresent must match"), "{error}");

        let end_false_payload = ResponseEndFrameHeader {
            payload_present: false,
            ..end
        };
        let frame = encode_binary_frame(&end_false_payload, b"x")
            .expect("response.end with payload must encode");
        let error = decode_response_end_frame(&frame)
            .expect_err("payloadPresent false with payload must be rejected")
            .to_string();
        assert!(error.contains("payloadPresent must match"), "{error}");

        let chunk = ResponseChunkFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: RESPONSE_CHUNK_FRAME_TYPE.to_string(),
            request_id: "req-2".to_string(),
            seq: 0,
        };
        for payload in [&b""[..], &b"chunk"[..]] {
            let frame = encode_binary_frame(&chunk, payload)
                .expect("response.chunk with any payload must encode");
            let (decoded, decoded_payload) = decode_response_chunk_frame(&frame)
                .unwrap_or_else(|error| panic!("response.chunk must decode: {error}"));
            assert_eq!(decoded, chunk);
            assert_eq!(decoded_payload, payload);
        }

        let wrong_schema = ResponseEndFrameHeader {
            schema_version: "skiff-runtime-frame-v2".to_string(),
            ..end_false_payload
        };
        let frame =
            encode_binary_frame(&wrong_schema, &[]).expect("wrong schema header must encode");
        let error = decode_response_end_frame(&frame)
            .expect_err("wrong schemaVersion must be rejected")
            .to_string();
        assert!(error.contains("schemaVersion"), "{error}");
    }

    #[test]
    fn scenarios_are_present_and_reference_only_w_model_decodable_frames() {
        let catalog = frame_catalog();
        let scenario_dir = corpus_dir().join("scenarios");
        let mut found = Vec::new();
        for entry in fs::read_dir(&scenario_dir).expect("scenarios dir must be readable") {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            assert_eq!(value["schemaVersion"], 1, "{}", path.display());
            let name = value["scenario"].as_str().expect("scenario name");
            found.push(name.to_string());
            for event in value["events"].as_array().expect("events") {
                if let Some(frame) = event["frame"].as_str() {
                    let entry = catalog
                        .frames
                        .get(frame)
                        .unwrap_or_else(|| panic!("{name} references unknown frame {frame}"));
                    assert_ne!(entry.decode_as, "RequestStartHttpUnary");
                    assert_ne!(entry.decode_as, "RequestStartHttpStream");
                }
            }
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                found.iter().any(|name| name == required),
                "corpus must contain required scenario {required}"
            );
        }
        assert_eq!(found.len(), REQUIRED_SCENARIOS.len());
    }
}

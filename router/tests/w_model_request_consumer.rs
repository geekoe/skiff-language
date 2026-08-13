//! M-request consumer gate: skiff-router consumes the frozen
//! C-model-request corpus directly through the W-model transport codecs.
//!
//! The Router side of ordinary request traffic (C-model-request §2) encodes
//! `request.start` / `request.cancel` (RouterToRuntime) and decodes
//! `response.start` / `response.chunk` / `response.end` / `response.error`
//! (RuntimeToRouter). Every frozen frame must roundtrip byte-exact through
//! the same codecs the W-dispatch node will wire into the demux.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, BytecodeRequestStartFrameWireHeader,
};
use skiff_runtime_transport::protocol::{
    decode_request_cancel_frame, decode_response_chunk_frame, decode_response_end_frame,
    decode_response_start_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_request_cancel_frame, encode_response_chunk_frame, encode_response_end_frame,
    encode_response_start_frame, validate_response_error_frame, ResponseErrorFrameHeader,
    ValidatedResponseErrorFrame,
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
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadHex")]
    payload_hex: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
struct FrameCatalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runtime/transport/testdata/request-wire")
}

fn catalog() -> FrameCatalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("request-wire frames.json must be readable");
    serde_json::from_str(&value).expect("request-wire frames.json must decode")
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn assert_frame_type(name: &str, entry: &FrameEntry, expected: &str) {
    assert_eq!(entry.frame_type, expected, "{name}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn router_consumer_roundtrips_request_wire_corpus_byte_exact() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "request-wire-v1");
        for name in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(name),
                "corpus must contain required frame {name}"
            );
        }

        for (name, entry) in &catalog.frames {
            let bytes = hex_to_bytes(&entry.frame_hex);
            let expected_payload = hex_to_bytes(&entry.payload_hex);
            let reencoded = match entry.decode_as.as_str() {
                "RequestStartHttpUnary" | "RequestStartHttpStream" => {
                    assert_eq!(entry.direction, "RouterToRuntime", "{name}");
                    assert_frame_type(name, entry, "request.start");
                    let (header, payload) = decode_bytecode_request_start_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} start decode: {error}"));
                    let request_id = match &header {
                        BytecodeRequestStartFrameWireHeader::Http(http) => http.request_id.as_str(),
                        other => panic!("{name} must decode as HTTP start, got {other:?}"),
                    };
                    assert_eq!(
                        request_id,
                        entry.header["requestId"].as_str().expect("requestId"),
                        "{name}"
                    );
                    assert_eq!(payload, expected_payload, "{name}");
                    encode_binary_frame(&header, &payload).expect("start must encode")
                }
                "RequestCancel" => {
                    assert_eq!(entry.direction, "Either", "{name}");
                    assert_frame_type(name, entry, "request.cancel");
                    let header = decode_request_cancel_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} cancel decode: {error}"));
                    assert_eq!(
                        header.request_id,
                        entry.header["requestId"].as_str().expect("requestId"),
                        "{name}"
                    );
                    assert!(
                        RequestCancelReason::from_contract_h_wire(&header.reason).is_some(),
                        "{name} reason {} must be CONTRACT_H",
                        header.reason
                    );
                    encode_request_cancel_frame(&header).expect("cancel must encode")
                }
                "ResponseStart" => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_frame_type(name, entry, "response.start");
                    let header = decode_response_start_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} response.start decode: {error}"));
                    assert_eq!(
                        header.request_id,
                        entry.header["requestId"].as_str().expect("requestId"),
                        "{name}"
                    );
                    encode_response_start_frame(&header).expect("response.start must encode")
                }
                "ResponseChunk" => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_frame_type(name, entry, "response.chunk");
                    let (header, payload) = decode_response_chunk_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} response.chunk decode: {error}"));
                    assert_eq!(
                        header.seq,
                        entry.header["seq"].as_u64().expect("chunk seq"),
                        "{name}"
                    );
                    assert_eq!(payload, expected_payload, "{name}");
                    encode_response_chunk_frame(&header, &payload)
                        .expect("response.chunk must encode")
                }
                "ResponseEnd" => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_frame_type(name, entry, "response.end");
                    let (header, payload) = decode_response_end_frame(&bytes)
                        .unwrap_or_else(|error| panic!("{name} response.end decode: {error}"));
                    assert_eq!(
                        header.payload_present,
                        entry.header["payloadPresent"]
                            .as_bool()
                            .expect("payloadPresent"),
                        "{name}"
                    );
                    assert_eq!(payload, expected_payload, "{name}");
                    encode_response_end_frame(&header, &payload).expect("response.end must encode")
                }
                "ResponseErrorControl" | "ResponseErrorFixedService" => {
                    assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
                    assert_frame_type(name, entry, "response.error");
                    let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                        decode_typed_binary_frame(&bytes).unwrap_or_else(|error| {
                            panic!("{name} response.error decode: {error}")
                        });
                    let validated = validate_response_error_frame(&header, payload.clone())
                        .unwrap_or_else(|error| panic!("{name} response.error validate: {error}"));
                    let (decoded_header, decoded_body) =
                        skiff_runtime_transport::protocol::decode_response_error_frame(&bytes)
                            .unwrap_or_else(|error| {
                                panic!("{name} response.error decode: {error}")
                            });
                    assert_eq!(decoded_header, header, "{name}");
                    match (&validated, decoded_body) {
                        (
                            ValidatedResponseErrorFrame::FixedService(expected),
                            ValidatedResponseErrorFrame::FixedService(decoded),
                        ) => {
                            assert_eq!(expected.encoded_bytes(), decoded.encoded_bytes(), "{name}");
                        }
                        (
                            ValidatedResponseErrorFrame::Control(expected),
                            ValidatedResponseErrorFrame::Control(decoded),
                        ) => {
                            assert_eq!(expected.code, decoded.code, "{name}");
                            assert_eq!(expected.message, decoded.message, "{name}");
                        }
                        _ => panic!("{name} validated variant must match decoded variant"),
                    }
                    encode_binary_frame(&header, &payload).expect("response.error must encode")
                }
                other => panic!("{name} has unknown decodeAs {other}"),
            };
            assert_eq!(
                reencoded, bytes,
                "{name} must roundtrip byte-exact in the Router consumer"
            );
        }
    }

    #[test]
    fn router_consumer_sees_all_frozen_request_wire_scenarios() {
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
            found.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
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

//! Byte-exact request-wire corpus verifier for C-model-request
//! (`doc/implementation/router-rust-migration/contracts/router-rust-migration-c-model-request-contract.md`).
//!
//! TEST-ONLY reference model. Not production code and not imported by any
//! production crate. W-model-request/W-dispatch must implement the frozen
//! semantics from the contract doc and consume the same fixtures.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, validate_response_error_frame,
    RequestCancelFrameHeader, ResponseChunkFrameHeader, ResponseEndFrameHeader,
    ResponseErrorFrameHeader, ResponseStartFrameHeader,
};
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, BytecodeRequestStartFrameWireHeader,
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

fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "unary-response-end",
            include_str!("../testdata/request-wire/scenarios/01-unary-response-end.json"),
        ),
        (
            "unary-response-error-control",
            include_str!("../testdata/request-wire/scenarios/02-unary-response-error-control.json"),
        ),
        (
            "unary-response-error-fixed-service",
            include_str!(
                "../testdata/request-wire/scenarios/03-unary-response-error-fixed-service.json"
            ),
        ),
        (
            "stream-start-chunk-chunk-end",
            include_str!("../testdata/request-wire/scenarios/04-stream-start-chunk-chunk-end.json"),
        ),
        (
            "stream-end-before-start-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/05-stream-end-before-start-rejected.json"
            ),
        ),
        (
            "stream-chunk-before-start-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/06-stream-chunk-before-start-rejected.json"
            ),
        ),
        (
            "stream-chunk-seq-gap-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/07-stream-chunk-seq-gap-rejected.json"
            ),
        ),
        (
            "stream-duplicate-start-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/08-stream-duplicate-start-rejected.json"
            ),
        ),
        (
            "stream-start-on-unary-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/09-stream-start-on-unary-rejected.json"
            ),
        ),
        (
            "stream-end-with-payload-rejected",
            include_str!(
                "../testdata/request-wire/scenarios/10-stream-end-with-payload-rejected.json"
            ),
        ),
        (
            "request-cancel-router-to-runtime",
            include_str!(
                "../testdata/request-wire/scenarios/11-request-cancel-router-to-runtime.json"
            ),
        ),
        (
            "request-cancel-runtime-to-router",
            include_str!(
                "../testdata/request-wire/scenarios/12-request-cancel-runtime-to-router.json"
            ),
        ),
        (
            "stale-response-ignored",
            include_str!("../testdata/request-wire/scenarios/13-stale-response-ignored.json"),
        ),
    ]
}

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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    events: Vec<Event>,
    expect: Expect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Event {
    Start {
        #[serde(rename = "requestId")]
        request_id: String,
        mode: String,
    },
    Read {
        #[serde(rename = "requestId")]
        request_id: String,
        frame: String,
        #[serde(rename = "payloadHex")]
        payload_hex: Option<String>,
    },
    Cancel {
        #[serde(rename = "requestId")]
        request_id: String,
        direction: String,
        frame: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expect {
    outcomes: HashMap<String, String>,
    #[serde(rename = "terminalSources")]
    terminal_sources: HashMap<String, String>,
    chunks: HashMap<String, Vec<String>>,
    payload: HashMap<String, Option<String>>,
    #[serde(rename = "protocolErrors")]
    protocol_errors: u64,
    #[serde(rename = "ignoredStale")]
    ignored_stale: u64,
}

fn frame_catalog() -> FrameCatalog {
    serde_json::from_str(include_str!("../testdata/request-wire/frames.json"))
        .expect("request-wire frames.json must decode")
}

fn reject_catalog() -> RejectCatalog {
    serde_json::from_str(include_str!("../testdata/request-wire/reject-cases.json"))
        .expect("request-wire reject-cases.json must decode")
}

fn decode_hex(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn reencode(header: &impl serde::Serialize, payload: &[u8]) -> Vec<u8> {
    encode_binary_frame(header, payload).expect("frame must re-encode")
}

#[test]
fn frame_catalog_is_frozen_and_reasons_match_transport_contract() {
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
            "required frame {name} is missing"
        );
        let entry = &catalog.frames[name];
        assert!(!entry.direction.is_empty(), "{name} direction");
        assert!(!entry.frame_type.is_empty(), "{name} frame type");
        match entry.decode_as.as_str() {
            "RequestStartHttpUnary" | "RequestStartHttpStream" => {
                assert_eq!(entry.direction, "RouterToRuntime", "{name}");
                assert_eq!(entry.frame_type, "request.start", "{name}");
            }
            "RequestCancel" => assert_eq!(entry.direction, "Either", "{name}"),
            "ResponseStart"
            | "ResponseChunk"
            | "ResponseEnd"
            | "ResponseErrorControl"
            | "ResponseErrorFixedService" => {
                assert_eq!(entry.direction, "RuntimeToRouter", "{name}");
            }
            other => panic!("{name} has unknown decodeAs {other}"),
        }
    }

    let contract_reasons: Vec<String> = RequestCancelReason::CONTRACT_H
        .iter()
        .map(|reason| reason.as_str().to_string())
        .collect();
    assert_eq!(catalog.cancel_reasons, contract_reasons);
}

#[test]
fn frames_round_trip_byte_exact_with_real_codec() {
    let catalog = frame_catalog();
    for (name, entry) in &catalog.frames {
        let expected_bytes = decode_hex(&entry.frame_hex);
        let expected_payload = decode_hex(&entry.payload_hex);
        let reencoded = match entry.decode_as.as_str() {
            "RequestStartHttpUnary" | "RequestStartHttpStream" => {
                let (header, payload) =
                    decode_bytecode_request_start_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} start decode: {error}"));
                let mode = match &header {
                    BytecodeRequestStartFrameWireHeader::Http(http) => http.mode.as_str(),
                    other => panic!("{name} must decode as HTTP start, got {other:?}"),
                };
                let expected_mode = if entry.decode_as == "RequestStartHttpUnary" {
                    "unary"
                } else {
                    "serverStream"
                };
                assert_eq!(mode, expected_mode, "{name} mode");
                assert_eq!(payload, expected_payload, "{name} payload");
                reencode(&header, &payload)
            }
            "RequestCancel" => {
                let (header, payload): (RequestCancelFrameHeader, Vec<u8>) =
                    decode_typed_binary_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} cancel decode: {error}"));
                assert_eq!(header.envelope_type, "request.cancel", "{name}");
                assert_eq!(entry.payload_rule, "empty", "{name}");
                assert!(payload.is_empty(), "{name} payload must be empty");
                reencode(&header, &payload)
            }
            "ResponseStart" => {
                let (header, payload): (ResponseStartFrameHeader, Vec<u8>) =
                    decode_typed_binary_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.start decode: {error}"));
                assert_eq!(header.envelope_type, "response.start", "{name}");
                assert!(
                    payload.is_empty(),
                    "{name} response.start payload must be empty"
                );
                reencode(&header, &payload)
            }
            "ResponseChunk" => {
                let (header, payload): (ResponseChunkFrameHeader, Vec<u8>) =
                    decode_typed_binary_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.chunk decode: {error}"));
                assert_eq!(header.envelope_type, "response.chunk", "{name}");
                assert_eq!(payload, expected_payload, "{name} chunk payload");
                reencode(&header, &payload)
            }
            "ResponseEnd" => {
                let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
                    decode_typed_binary_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.end decode: {error}"));
                assert_eq!(header.envelope_type, "response.end", "{name}");
                assert_eq!(payload, expected_payload, "{name} response.end payload");
                reencode(&header, &payload)
            }
            "ResponseErrorControl" | "ResponseErrorFixedService" => {
                let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                    decode_typed_binary_frame(&expected_bytes)
                        .unwrap_or_else(|error| panic!("{name} response.error decode: {error}"));
                validate_response_error_frame(&header, payload.clone())
                    .unwrap_or_else(|error| panic!("{name} response.error validate: {error}"));
                assert_eq!(payload, expected_payload, "{name} response.error payload");
                reencode(&header, &payload)
            }
            other => panic!("{name} has unknown decodeAs {other}"),
        };
        assert_eq!(
            reencoded, expected_bytes,
            "{name} must round-trip byte-exact"
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
        let result = decode_bytecode_request_start_frame(&frame);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireKind {
    Unary,
    Stream,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamPhase {
    WaitingStart,
    Streaming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WireResponse {
    Start,
    Chunk { seq: u64 },
    End { payload_present: bool },
    Error,
}

#[derive(Debug)]
struct WirePending {
    kind: WireKind,
    phase: Option<StreamPhase>,
    next_seq: u64,
    outcome: Option<String>,
    terminal_source: Option<String>,
    payload: Option<Vec<u8>>,
    chunks: Vec<Vec<u8>>,
}

impl WirePending {
    fn new(kind: WireKind) -> Self {
        Self {
            kind,
            phase: (kind == WireKind::Stream).then_some(StreamPhase::WaitingStart),
            next_seq: 0,
            outcome: None,
            terminal_source: None,
            payload: None,
            chunks: Vec::new(),
        }
    }

    fn terminal(&mut self, outcome: &str, source: &str) {
        self.outcome = Some(outcome.to_string());
        self.terminal_source = Some(source.to_string());
        self.phase = None;
    }
}

struct WireMachine {
    pending: HashMap<String, WirePending>,
    protocol_errors: u64,
    ignored_stale: u64,
}

impl WireMachine {
    fn new() -> Self {
        Self {
            pending: HashMap::new(),
            protocol_errors: 0,
            ignored_stale: 0,
        }
    }

    fn start(&mut self, request_id: &str, mode: &str) {
        let kind = match mode {
            "unary" => WireKind::Unary,
            "serverStream" => WireKind::Stream,
            other => panic!("unknown mode {other}"),
        };
        self.pending
            .insert(request_id.to_string(), WirePending::new(kind));
    }

    fn read(&mut self, request_id: &str, response: WireResponse, payload: Vec<u8>) {
        let Some(pending) = self.pending.get_mut(request_id) else {
            self.ignored_stale += 1;
            return;
        };
        if pending.outcome.is_some() {
            self.ignored_stale += 1;
            return;
        }
        match response {
            WireResponse::Start => {
                if pending.kind != WireKind::Stream
                    || pending.phase != Some(StreamPhase::WaitingStart)
                {
                    self.protocol_error(request_id);
                    return;
                }
                if !payload.is_empty() {
                    self.protocol_error(request_id);
                    return;
                }
                pending.phase = Some(StreamPhase::Streaming);
            }
            WireResponse::Chunk { seq } => {
                if pending.kind != WireKind::Stream || pending.phase != Some(StreamPhase::Streaming)
                {
                    self.protocol_error(request_id);
                    return;
                }
                if seq != pending.next_seq {
                    self.protocol_error(request_id);
                    return;
                }
                pending.chunks.push(payload);
                pending.next_seq += 1;
            }
            WireResponse::End { payload_present } => {
                let payload_is_empty = payload.is_empty();
                if pending.kind == WireKind::Stream {
                    if pending.phase != Some(StreamPhase::Streaming) {
                        self.protocol_error(request_id);
                        return;
                    }
                    if payload_present || !payload_is_empty {
                        self.protocol_error(request_id);
                        return;
                    }
                    pending.terminal("completed", "runtime_response_end");
                    return;
                }
                if payload_present == payload_is_empty {
                    self.protocol_error(request_id);
                    return;
                }
                pending.payload = Some(payload);
                pending.terminal("completed", "runtime_response_end");
            }
            WireResponse::Error => {
                pending.terminal("failed", "runtime_response_error");
            }
        }
    }

    fn cancel(&mut self, request_id: &str, direction: &str) {
        let Some(pending) = self.pending.get_mut(request_id) else {
            return;
        };
        if pending.outcome.is_some() {
            return;
        }
        let source = match direction {
            "routerToRuntime" => "router_cancel",
            "runtimeToRouter" => "runtime_request_cancel",
            other => panic!("unknown cancel direction {other}"),
        };
        pending.terminal("cancelled", source);
    }

    fn protocol_error(&mut self, request_id: &str) {
        self.protocol_errors += 1;
        let pending = self
            .pending
            .get_mut(request_id)
            .expect("protocol error requires a pending request");
        pending.terminal("protocolError", "protocol_error");
    }
}

#[test]
fn wire_scenarios_match_reference_machine() {
    let catalog = frame_catalog();
    for (name, json) in scenario_files() {
        let scenario: Scenario = serde_json::from_str(json)
            .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
        assert_eq!(scenario.schema_version, 1, "{name}");
        assert_eq!(scenario.scenario, name, "{name}");

        let mut machine = WireMachine::new();
        for event in &scenario.events {
            match event {
                Event::Start { request_id, mode } => machine.start(request_id, mode),
                Event::Read {
                    request_id,
                    frame,
                    payload_hex,
                } => {
                    let entry = catalog
                        .frames
                        .get(frame)
                        .unwrap_or_else(|| panic!("{name} references unknown frame {frame}"));
                    let payload = match payload_hex {
                        Some(hex) => decode_hex(hex),
                        None => decode_hex(&entry.payload_hex),
                    };
                    let response = match entry.decode_as.as_str() {
                        "ResponseStart" => WireResponse::Start,
                        "ResponseChunk" => {
                            let seq = entry
                                .header
                                .get("seq")
                                .and_then(Value::as_u64)
                                .expect("chunk seq");
                            WireResponse::Chunk { seq }
                        }
                        "ResponseEnd" => {
                            let payload_present = entry
                                .header
                                .get("payloadPresent")
                                .and_then(Value::as_bool)
                                .expect("response.end payloadPresent");
                            WireResponse::End { payload_present }
                        }
                        "ResponseErrorControl" | "ResponseErrorFixedService" => WireResponse::Error,
                        other => panic!("{name} read event uses non-response frame {other}"),
                    };
                    machine.read(request_id, response, payload);
                }
                Event::Cancel {
                    request_id,
                    direction,
                    frame,
                } => {
                    if let Some(frame_name) = frame {
                        assert!(
                            catalog.frames.contains_key(frame_name),
                            "{name} references unknown cancel frame {frame_name}"
                        );
                    }
                    machine.cancel(request_id, direction);
                }
            }
        }

        let mut actual_outcomes: HashMap<String, String> = HashMap::new();
        let mut actual_sources: HashMap<String, String> = HashMap::new();
        let mut actual_chunks: HashMap<String, Vec<String>> = HashMap::new();
        let mut actual_payload: HashMap<String, Option<String>> = HashMap::new();
        for (request_id, pending) in &machine.pending {
            if let Some(outcome) = &pending.outcome {
                actual_outcomes.insert(request_id.clone(), outcome.clone());
            }
            if let Some(source) = &pending.terminal_source {
                actual_sources.insert(request_id.clone(), source.clone());
            }
            if pending.kind == WireKind::Stream && !pending.chunks.is_empty() {
                actual_chunks.insert(
                    request_id.clone(),
                    pending.chunks.iter().map(|chunk| hex(chunk)).collect(),
                );
            }
            if let Some(payload) = &pending.payload {
                actual_payload.insert(request_id.clone(), Some(hex(payload)));
            }
        }
        assert_eq!(actual_outcomes, scenario.expect.outcomes, "{name} outcomes");
        assert_eq!(
            actual_sources, scenario.expect.terminal_sources,
            "{name} terminal sources"
        );
        assert_eq!(actual_chunks, scenario.expect.chunks, "{name} chunks");
        assert_eq!(actual_payload, scenario.expect.payload, "{name} payload");
        assert_eq!(
            machine.protocol_errors, scenario.expect.protocol_errors,
            "{name} protocol errors"
        );
        assert_eq!(
            machine.ignored_stale, scenario.expect.ignored_stale,
            "{name} ignored stale"
        );
    }
}

#[test]
fn scenarios_cover_required_request_wire_list() {
    let names: std::collections::HashSet<&str> =
        scenario_files().iter().map(|(name, _)| *name).collect();
    for required in REQUIRED_SCENARIOS {
        assert!(
            names.contains(required),
            "required scenario {required} is missing"
        );
    }
    assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

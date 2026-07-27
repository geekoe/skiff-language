use std::collections::HashSet;

use serde::Deserialize;
use serde_json::Value;

use super::{
    decode_runtime_assembly_request_start_frame,
    decode_runtime_assembly_websocket_connect_response_end_frame,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestStartFrameWireHeader,
};
use crate::protocol::{
    encode_binary_frame, RequestStartFrameHeader, BINARY_FRAME_HEADER_ENCODING_JSON,
    BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Corpus {
    request_start_headers: Vec<Value>,
    request_start_mutations: Vec<Mutation>,
    request_start_raw_cases: Vec<RawCase>,
    request_start_payload_cases: Vec<PayloadCase>,
    request_start_equivalent_option_pairs: Vec<EquivalentOptionPair>,
    legacy_request_start_headers: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mutation {
    name: String,
    base_index: usize,
    set_path: Option<String>,
    remove_path: Option<String>,
    #[serde(default)]
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawCase {
    name: String,
    outcome: String,
    header_text: Option<String>,
    header_hex: Option<String>,
    frame_hex: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PayloadCase {
    name: String,
    base_index: usize,
    payload_hex: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EquivalentOptionPair {
    name: String,
    base_index: usize,
    path: String,
    value: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectWireCorpus {
    request_cases: Vec<ConnectWireRequestCase>,
    request_mutations: Vec<ConnectWireMutation>,
    response_cases: Vec<ConnectWireResponseCase>,
    response_mutations: Vec<ConnectWireMutation>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectWireRequestCase {
    name: String,
    kind: String,
    header: Value,
    payload_hex: String,
    canonical_json: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectWireResponseCase {
    name: String,
    header: Value,
    payload_hex: String,
    canonical_json: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConnectWireMutation {
    name: String,
    base_index: usize,
    set_path: Option<String>,
    remove_path: Option<String>,
    #[serde(default)]
    value: Value,
    payload_hex: Option<String>,
}

#[test]
fn runtime_assembly_request_start_corpus_is_nonempty_and_uniquely_named() {
    let corpus = corpus();
    assert!(!corpus.request_start_headers.is_empty());
    assert!(!corpus.request_start_mutations.is_empty());
    assert!(!corpus.request_start_raw_cases.is_empty());
    assert!(!corpus.request_start_payload_cases.is_empty());
    assert!(!corpus.request_start_equivalent_option_pairs.is_empty());
    assert!(!corpus.legacy_request_start_headers.is_empty());

    let mut names = HashSet::new();
    for name in corpus
        .request_start_mutations
        .iter()
        .map(|case| case.name.as_str())
        .chain(
            corpus
                .request_start_raw_cases
                .iter()
                .map(|case| case.name.as_str()),
        )
        .chain(
            corpus
                .request_start_payload_cases
                .iter()
                .map(|case| case.name.as_str()),
        )
        .chain(
            corpus
                .request_start_equivalent_option_pairs
                .iter()
                .map(|case| case.name.as_str()),
        )
    {
        assert!(names.insert(name), "duplicate corpus case {name}");
    }
}

#[test]
fn runtime_assembly_request_start_normalizes_equivalent_optional_defaults() {
    let corpus = corpus();
    for pair in corpus.request_start_equivalent_option_pairs {
        let mut absent = corpus.request_start_headers[pair.base_index].clone();
        let mut explicit = absent.clone();
        apply_path(&mut absent, &pair.path, &Value::Null, true);
        apply_path(&mut explicit, &pair.path, &pair.value, false);
        let absent_frame = encode_binary_frame(&absent, &[]).unwrap();
        let explicit_frame = encode_binary_frame(&explicit, &[]).unwrap();
        let absent = decode_runtime_assembly_request_start_frame(&absent_frame)
            .unwrap_or_else(|error| panic!("{} absent: {error}", pair.name))
            .0;
        let explicit = decode_runtime_assembly_request_start_frame(&explicit_frame)
            .unwrap_or_else(|error| panic!("{} explicit: {error}", pair.name))
            .0;
        assert_eq!(absent, explicit, "{}", pair.name);
    }
}

#[test]
fn runtime_assembly_request_start_decodes_shared_http_headers() {
    let corpus = corpus();
    let mut modes = HashSet::new();
    for value in corpus.request_start_headers {
        let expected: RuntimeAssemblyRequestStartFrameHeader =
            serde_json::from_value(value.clone()).expect("canonical request header");
        modes.insert(expected.mode.clone());
        assert_eq!(expected.caller.kind, "gateway");
        assert_eq!(
            expected.routing.ingress.protocol,
            super::RuntimeAssemblyRequestIngressProtocol::Http
        );
        let serialized = serde_json::to_value(&expected).unwrap();
        let mut normalized = value;
        normalized
            .as_object_mut()
            .unwrap()
            .entry("testEffectsEnabled")
            .or_insert(Value::Bool(false));
        assert_eq!(serialized, normalized);
        let reparsed: RuntimeAssemblyRequestStartFrameHeader =
            serde_json::from_value(serialized).unwrap();
        assert_eq!(reparsed, expected);
        let frame = encode_binary_frame(&expected, &[]).unwrap();
        let (decoded, decoded_payload) =
            decode_runtime_assembly_request_start_frame(&frame).unwrap();
        assert_eq!(
            decoded,
            RuntimeAssemblyRequestStartFrameWireHeader::Http(expected)
        );
        assert!(decoded_payload.is_empty());
    }
    assert_eq!(
        modes,
        HashSet::from(["unary".to_string(), "serverStream".to_string()])
    );
}

#[test]
fn runtime_assembly_request_start_preserves_opaque_http_payload_boundaries() {
    let corpus = corpus();
    for payload_case in corpus.request_start_payload_cases {
        let header = &corpus.request_start_headers[payload_case.base_index];
        let payload = decode_hex(&payload_case.payload_hex);
        let frame = encode_binary_frame(header, &payload).unwrap();
        let (_, decoded_payload) = decode_runtime_assembly_request_start_frame(&frame)
            .unwrap_or_else(|error| panic!("{}: {error}", payload_case.name));
        assert_eq!(decoded_payload, payload, "{}", payload_case.name);
    }
}

#[test]
fn runtime_assembly_request_start_mutations_fail_closed() {
    let corpus = corpus();
    for mutation in corpus.request_start_mutations {
        assert!(
            mutation.base_index < corpus.request_start_headers.len(),
            "{}",
            mutation.name
        );
        let mut value = corpus.request_start_headers[mutation.base_index].clone();
        apply_mutation(&mut value, &mutation);
        assert!(
            serde_json::from_value::<RuntimeAssemblyRequestStartFrameWireHeader>(value.clone())
                .is_err(),
            "{}",
            mutation.name
        );
        if !mutation.name.contains("negative zero") {
            let frame = encode_binary_frame(&value, &[]).unwrap();
            assert!(
                decode_runtime_assembly_request_start_frame(&frame).is_err(),
                "{} production decoder",
                mutation.name
            );
        }
    }
}

#[test]
fn runtime_assembly_request_start_raw_json_and_frame_cases_fail_closed() {
    for raw_case in corpus().request_start_raw_cases {
        let frame = raw_case_frame(&raw_case);
        match raw_case.outcome.as_str() {
            "accept" => {
                decode_runtime_assembly_request_start_frame(&frame)
                    .unwrap_or_else(|error| panic!("{}: {error}", raw_case.name));
            }
            "reject" => assert!(
                decode_runtime_assembly_request_start_frame(&frame).is_err(),
                "{}",
                raw_case.name
            ),
            outcome => panic!("unknown raw outcome {outcome}"),
        }
    }
}

#[test]
fn runtime_assembly_request_start_preserves_legacy_decoder_baseline() {
    for value in corpus().legacy_request_start_headers {
        let decoded: RequestStartFrameHeader =
            serde_json::from_value(value.clone()).expect("legacy request baseline");
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
        assert!(
            serde_json::from_value::<RuntimeAssemblyRequestStartFrameWireHeader>(value).is_err()
        );
    }
}

#[test]
fn runtime_assembly_request_current_wire_corpus_is_exact_and_uniquely_named() {
    let corpus = connect_wire_corpus();
    assert_eq!(corpus.request_cases.len(), 3);
    assert!(corpus.request_mutations.len() >= 20);
    assert_eq!(corpus.response_cases.len(), 3);
    assert!(corpus.response_mutations.len() >= 20);

    let mut names = HashSet::new();
    for name in corpus
        .request_cases
        .iter()
        .map(|case| case.name.as_str())
        .chain(
            corpus
                .request_mutations
                .iter()
                .map(|case| case.name.as_str()),
        )
        .chain(corpus.response_cases.iter().map(|case| case.name.as_str()))
        .chain(
            corpus
                .response_mutations
                .iter()
                .map(|case| case.name.as_str()),
        )
    {
        assert!(
            names.insert(name),
            "duplicate connect wire corpus case {name}"
        );
    }
}

#[test]
fn runtime_assembly_request_current_http_and_websocket_json_match_shared_goldens() {
    for case in connect_wire_corpus().request_cases {
        let payload = decode_hex(&case.payload_hex);
        let frame = encode_binary_frame(&case.header, &payload)
            .unwrap_or_else(|error| panic!("{} must encode: {error}", case.name));
        let (decoded, decoded_payload) = decode_runtime_assembly_request_start_frame(&frame)
            .unwrap_or_else(|error| panic!("{} must decode: {error}", case.name));
        assert_eq!(decoded_payload, payload, "{} payload", case.name);
        match (&*case.kind, &decoded) {
            ("http", RuntimeAssemblyRequestStartFrameWireHeader::Http(_))
            | (
                "websocketConnect",
                RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(_),
            ) => {}
            other => panic!("{} has wrong request branch {other:?}", case.name),
        }
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            case.canonical_json,
            "{} canonical JSON",
            case.name
        );
    }
}

#[test]
fn runtime_assembly_request_current_mutations_fail_closed() {
    let corpus = connect_wire_corpus();
    for mutation in corpus.request_mutations {
        let base = corpus
            .request_cases
            .get(mutation.base_index)
            .unwrap_or_else(|| panic!("{} base index", mutation.name));
        let mut header = base.header.clone();
        apply_connect_wire_mutation(&mut header, &mutation);
        let payload = mutation
            .payload_hex
            .as_deref()
            .map(decode_hex)
            .unwrap_or_else(|| decode_hex(&base.payload_hex));
        let frame = encode_binary_frame(&header, &payload)
            .unwrap_or_else(|error| panic!("{} mutation frame: {error}", mutation.name));
        assert!(
            decode_runtime_assembly_request_start_frame(&frame).is_err(),
            "{}",
            mutation.name
        );
    }
}

#[test]
fn runtime_assembly_websocket_connect_response_json_matches_shared_goldens() {
    for case in connect_wire_corpus().response_cases {
        let payload = decode_hex(&case.payload_hex);
        let frame = encode_binary_frame(&case.header, &payload)
            .unwrap_or_else(|error| panic!("{} must encode: {error}", case.name));
        let decoded = decode_runtime_assembly_websocket_connect_response_end_frame(&frame)
            .unwrap_or_else(|error| panic!("{} must decode: {error}", case.name));
        assert_eq!(
            serde_json::to_string(&decoded).unwrap(),
            case.canonical_json,
            "{} canonical JSON",
            case.name
        );
    }
}

#[test]
fn runtime_assembly_websocket_connect_response_mutations_fail_closed() {
    let corpus = connect_wire_corpus();
    for mutation in corpus.response_mutations {
        let base = corpus
            .response_cases
            .get(mutation.base_index)
            .unwrap_or_else(|| panic!("{} base index", mutation.name));
        let mut header = base.header.clone();
        apply_connect_wire_mutation(&mut header, &mutation);
        let payload = mutation
            .payload_hex
            .as_deref()
            .map(decode_hex)
            .unwrap_or_else(|| decode_hex(&base.payload_hex));
        let frame = encode_binary_frame(&header, &payload)
            .unwrap_or_else(|error| panic!("{} mutation frame: {error}", mutation.name));
        assert!(
            decode_runtime_assembly_websocket_connect_response_end_frame(&frame).is_err(),
            "{}",
            mutation.name
        );
    }
}

#[test]
fn runtime_assembly_websocket_connect_response_rejects_duplicate_json_keys() {
    let frame = raw_json_frame(
        br#"{"schemaVersion":"skiff-runtime-frame-v1","type":"response.end","requestId":"one","requestId":"two","payloadPresent":false,"websocketConnect":{"result":"accept"}}"#,
    );
    assert!(decode_runtime_assembly_websocket_connect_response_end_frame(&frame).is_err());
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json"
    ))
    .expect("shared runtime request corpus")
}

fn connect_wire_corpus() -> ConnectWireCorpus {
    serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json"
    ))
    .expect("shared runtime websocketConnect wire corpus")
}

fn raw_case_frame(raw_case: &RawCase) -> Vec<u8> {
    let sources = [
        raw_case.header_text.is_some(),
        raw_case.header_hex.is_some(),
        raw_case.frame_hex.is_some(),
    ];
    assert_eq!(
        sources.into_iter().filter(|present| *present).count(),
        1,
        "{} raw input",
        raw_case.name
    );
    if let Some(frame_hex) = raw_case.frame_hex.as_deref() {
        return decode_hex(frame_hex);
    }
    let header = if let Some(text) = raw_case.header_text.as_deref() {
        text.as_bytes().to_vec()
    } else {
        decode_hex(raw_case.header_hex.as_deref().expect("header hex"))
    };
    raw_json_frame(&header)
}

fn raw_json_frame(header: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + header.len());
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&0_u32.to_be_bytes());
    frame.extend_from_slice(header);
    frame
}

fn apply_mutation(root: &mut Value, mutation: &Mutation) {
    let path = mutation
        .set_path
        .as_ref()
        .or(mutation.remove_path.as_ref())
        .expect("mutation path");
    apply_path(root, path, &mutation.value, mutation.remove_path.is_some());
}

fn apply_connect_wire_mutation(root: &mut Value, mutation: &ConnectWireMutation) {
    if let Some(path) = mutation.set_path.as_deref() {
        apply_path(root, path, &mutation.value, false);
    }
    if let Some(path) = mutation.remove_path.as_deref() {
        apply_path(root, path, &Value::Null, true);
    }
}

fn apply_path(root: &mut Value, path: &str, value: &Value, remove: bool) {
    let mut segments = path.split('.').collect::<Vec<_>>();
    let leaf = segments.pop().unwrap();
    let mut owner = root;
    for segment in segments {
        owner = match owner {
            Value::Object(object) => object.get_mut(segment).expect("mutation object owner"),
            Value::Array(array) => &mut array[segment.parse::<usize>().expect("array index")],
            _ => panic!("mutation owner must be an object or array"),
        };
    }
    match owner {
        Value::Object(object) => {
            if remove {
                object.remove(leaf);
            } else {
                object.insert(leaf.to_string(), value.clone());
            }
        }
        Value::Array(array) => {
            let index = leaf.parse::<usize>().expect("array leaf index");
            if remove {
                array.remove(index);
            } else {
                array[index] = value.clone();
            }
        }
        _ => panic!("mutation leaf owner must be an object or array"),
    }
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(pair, 16).unwrap()
        })
        .collect()
}

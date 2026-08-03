use std::collections::HashSet;

use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    decode_runtime_assembly_request_start_frame,
    decode_runtime_assembly_websocket_connect_response_end_frame,
    decode_runtime_assembly_websocket_jsonrpc_response_end_frame,
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

fn canonical_task_header(test_effects_enabled: bool) -> Value {
    let mut header = json!({
        "schemaVersion": "skiff-runtime-frame-v3",
        "type": "request.start",
        "requestId": "task-request-1",
        "mode": "unary",
        "caller": {"kind": "service"},
        "routing": {
            "kind": "runtimeAssembly",
            "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "assemblyGeneration": 7,
            "deployment": {
                "serviceId": "example.com/worker",
                "contractVersion": "1.0.0",
                "deploymentRevision": "deployment-1",
                "deploymentArtifactIdentity": "skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            }
        },
        "invocation": {
            "kind": "task",
            "targetKind": "function",
            "target": "function:worker.run"
        },
        "trace": {
            "traceId": "trace-task",
            "spanId": "span-task",
            "sampled": true
        },
        "testEffectsEnabled": test_effects_enabled
    });
    if test_effects_enabled {
        header["testCaseCapability"] = json!("test-case-capability-1");
    }
    header
}

#[test]
fn runtime_assembly_task_request_decodes_production_and_test_authority() {
    for test_effects_enabled in [false, true] {
        let frame =
            encode_binary_frame(&canonical_task_header(test_effects_enabled), &[0x81]).unwrap();
        let (header, payload) = decode_runtime_assembly_request_start_frame(&frame).unwrap();
        let RuntimeAssemblyRequestStartFrameWireHeader::Task(header) = header else {
            panic!("task invocation must select the closed task union branch")
        };
        assert_eq!(header.invocation.target, "function:worker.run");
        assert_eq!(header.test_effects_enabled, test_effects_enabled);
        assert_eq!(header.test_case_capability.is_some(), test_effects_enabled);
        assert_eq!(payload, vec![0x81]);
    }
}

#[test]
fn runtime_assembly_task_request_rejects_authority_mismatch_and_empty_payload() {
    let mut missing_capability = canonical_task_header(true);
    missing_capability
        .as_object_mut()
        .unwrap()
        .remove("testCaseCapability");
    let frame = encode_binary_frame(&missing_capability, &[0x81]).unwrap();
    assert!(decode_runtime_assembly_request_start_frame(&frame).is_err());

    let mut production_with_capability = canonical_task_header(false);
    production_with_capability["testCaseCapability"] = json!("test-case-capability-1");
    let frame = encode_binary_frame(&production_with_capability, &[0x81]).unwrap();
    assert!(decode_runtime_assembly_request_start_frame(&frame).is_err());

    let frame = encode_binary_frame(&canonical_task_header(false), &[]).unwrap();
    assert!(decode_runtime_assembly_request_start_frame(&frame).is_err());
}

#[test]
fn runtime_assembly_task_request_optional_task_attempt_is_validated() {
    let mut with_attempt = canonical_task_header(false);
    with_attempt["taskAttempt"] = json!({
        "taskId": "task-1",
        "attemptId": "attempt-1",
        "leaseId": "lease-1"
    });
    let frame = encode_binary_frame(&with_attempt, &[0x81]).unwrap();
    let (header, payload) = decode_runtime_assembly_request_start_frame(&frame)
        .expect("valid taskAttempt must decode");
    let RuntimeAssemblyRequestStartFrameWireHeader::Task(header) = header else {
        panic!("task invocation must select the closed task union branch")
    };
    let attempt = header
        .task_attempt
        .expect("taskAttempt must be present");
    assert_eq!(attempt.task_id, "task-1");
    assert_eq!(attempt.attempt_id, "attempt-1");
    assert_eq!(attempt.lease_id, "lease-1");
    assert_eq!(payload, vec![0x81]);

    for field in ["taskId", "attemptId", "leaseId"] {
        let mut invalid = canonical_task_header(false);
        invalid["taskAttempt"] = json!({
            "taskId": "task-1",
            "attemptId": "attempt-1",
            "leaseId": "lease-1"
        });
        invalid["taskAttempt"][field] = json!("");
        let frame = encode_binary_frame(&invalid, &[0x81]).unwrap();
        let error = decode_runtime_assembly_request_start_frame(&frame)
            .expect_err("empty taskAttempt field must be a wire error");
        assert!(
            error.to_string().contains("taskAttempt"),
            "wire error must name taskAttempt, got {error}"
        );
    }
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
fn runtime_assembly_http_request_round_trips_router_test_parent_authority() {
    let mut value = corpus().request_start_headers[0].clone();
    value["testEffectsEnabled"] = json!(true);
    value["testCaseCapability"] = json!("test-case:capability_1");
    value["testCaseParentRequestId"] = json!("request:parent_1");

    let frame = encode_binary_frame(&value, br#"{"nested":true}"#).unwrap();
    let (decoded, payload) = decode_runtime_assembly_request_start_frame(&frame).unwrap();
    let RuntimeAssemblyRequestStartFrameWireHeader::Http(decoded) = decoded else {
        panic!("Router HTTP request must decode through the HTTP wire branch")
    };
    assert!(decoded.test_effects_enabled);
    assert_eq!(
        decoded.test_case_capability.as_deref(),
        Some("test-case:capability_1")
    );
    assert_eq!(
        decoded.test_case_parent_request_id.as_deref(),
        Some("request:parent_1")
    );
    assert_eq!(payload, br#"{"nested":true}"#);

    let serialized = serde_json::to_value(decoded).unwrap();
    assert_eq!(
        serialized["testCaseParentRequestId"],
        json!("request:parent_1")
    );
    assert!(serialized.get("test_case_parent_request_id").is_none());
}

#[test]
fn runtime_assembly_http_request_enforces_test_authority_shape_and_tokens() {
    let baseline = corpus().request_start_headers[0].clone();
    let mut cases = Vec::new();

    let mut enabled_without_capability = baseline.clone();
    enabled_without_capability["testEffectsEnabled"] = json!(true);
    enabled_without_capability
        .as_object_mut()
        .unwrap()
        .remove("testCaseCapability");
    cases.push(("enabled without capability", enabled_without_capability));

    let mut production_with_capability = baseline.clone();
    production_with_capability["testEffectsEnabled"] = json!(false);
    production_with_capability["testCaseCapability"] = json!("test-case:capability_1");
    cases.push(("production with capability", production_with_capability));

    let mut parent_without_capability = baseline.clone();
    parent_without_capability["testEffectsEnabled"] = json!(false);
    parent_without_capability
        .as_object_mut()
        .unwrap()
        .remove("testCaseCapability");
    parent_without_capability["testCaseParentRequestId"] = json!("request:parent_1");
    cases.push(("parent without capability", parent_without_capability));

    for invalid_parent in ["", "contains whitespace", "slash/not-allowed"] {
        let mut invalid = baseline.clone();
        invalid["testEffectsEnabled"] = json!(true);
        invalid["testCaseCapability"] = json!("test-case:capability_1");
        invalid["testCaseParentRequestId"] = json!(invalid_parent);
        cases.push(("invalid parent token", invalid));
    }

    let mut overlong_parent = baseline;
    overlong_parent["testEffectsEnabled"] = json!(true);
    overlong_parent["testCaseCapability"] = json!("test-case:capability_1");
    overlong_parent["testCaseParentRequestId"] = json!("p".repeat(257));
    cases.push(("overlong parent token", overlong_parent));

    for (name, value) in cases {
        let frame = encode_binary_frame(&value, &[]).unwrap();
        assert!(
            decode_runtime_assembly_request_start_frame(&frame).is_err(),
            "{name}"
        );
    }
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
fn runtime_assembly_websocket_jsonrpc_decoder_accepts_method_bearing_request() {
    let header = websocket_jsonrpc_request_header();
    let payload = br#"{"query":"ready"}"#;
    let frame = encode_binary_frame(&header, payload).expect("canonical JSON-RPC request frame");
    let (decoded, decoded_payload) = decode_runtime_assembly_request_start_frame(&frame)
        .expect("method-bearing WebSocket request must select the JSON-RPC sibling");

    assert!(matches!(
        decoded,
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(_)
    ));
    assert_eq!(decoded_payload, payload);
    assert_eq!(serde_json::to_value(decoded).unwrap(), header);

    let scalar_frame = encode_binary_frame(&header, b"42").unwrap();
    assert_eq!(
        decode_runtime_assembly_request_start_frame(&scalar_frame)
            .expect("transport must not parse params business shape")
            .1,
        b"42"
    );
}

#[test]
fn runtime_assembly_websocket_jsonrpc_method_null_and_string_select_disjoint_siblings() {
    let connect = websocket_connect_v2_header();
    let connect_frame = encode_binary_frame(&connect, &[]).unwrap();
    assert!(matches!(
        decode_runtime_assembly_request_start_frame(&connect_frame)
            .expect("method null must remain websocketConnect")
            .0,
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketConnect(_)
    ));

    let jsonrpc = websocket_jsonrpc_request_header();
    let jsonrpc_frame = encode_binary_frame(&jsonrpc, br#"{}"#).unwrap();
    assert!(matches!(
        decode_runtime_assembly_request_start_frame(&jsonrpc_frame)
            .expect("method string must select websocketJsonRpc")
            .0,
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(_)
    ));
}

#[test]
fn runtime_assembly_websocket_jsonrpc_request_mutations_fail_closed() {
    let canonical = websocket_jsonrpc_request_header();
    let canonical_payload = br#"{"query":"ready"}"#.to_vec();
    let mut mutations = Vec::new();

    let mut wrong_mode = canonical.clone();
    wrong_mode["mode"] = Value::String("serverStream".to_string());
    mutations.push(("wrong mode", wrong_mode, canonical_payload.clone()));

    let mut noncanonical_request_id = canonical.clone();
    noncanonical_request_id["requestId"] = Value::String(" request-id ".to_string());
    mutations.push((
        "non-canonical request id",
        noncanonical_request_id,
        canonical_payload.clone(),
    ));

    let mut wrong_profile = canonical.clone();
    wrong_profile["websocketJsonRpc"]["profile"] = Value::String("jsonrpc-1.0".to_string());
    mutations.push(("wrong profile", wrong_profile, canonical_payload.clone()));

    let mut identity_mismatch = canonical.clone();
    identity_mismatch["websocketJsonRpc"]["gatewayEntryIdentity"] = Value::String(
        "skiff-gateway-entry-v2:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
            .to_string(),
    );
    mutations.push((
        "identity mismatch",
        identity_mismatch,
        canonical_payload.clone(),
    ));

    let mut unknown_top_level = canonical.clone();
    unknown_top_level["peerRequestId"] = Value::String("must-not-enter-wire".to_string());
    mutations.push((
        "unknown top-level field",
        unknown_top_level,
        canonical_payload.clone(),
    ));

    let mut unknown_nested = canonical.clone();
    unknown_nested["websocketJsonRpc"]["rawSocketId"] =
        Value::String("must-not-enter-wire".to_string());
    mutations.push((
        "unknown nested field",
        unknown_nested,
        canonical_payload.clone(),
    ));

    let mut method_null = canonical.clone();
    method_null["routing"]["ingress"]["method"] = Value::Null;
    mutations.push((
        "method null cannot carry websocketJsonRpc",
        method_null,
        canonical_payload.clone(),
    ));

    let mut connect_with_method = websocket_connect_v2_header();
    connect_with_method["routing"]["ingress"]["method"] = Value::String("status.get".to_string());
    mutations.push((
        "method string cannot carry websocketConnect",
        connect_with_method,
        canonical_payload.clone(),
    ));

    let mut empty_method = canonical.clone();
    empty_method["routing"]["ingress"]["method"] = Value::String(String::new());
    mutations.push(("empty method", empty_method, canonical_payload.clone()));

    let mut oversized_method = canonical.clone();
    oversized_method["routing"]["ingress"]["method"] = Value::String("m".repeat(257));
    mutations.push((
        "oversized method",
        oversized_method,
        canonical_payload.clone(),
    ));

    let mut invalid_connection_id = canonical.clone();
    invalid_connection_id["websocketJsonRpc"]["connectionId"] =
        Value::String("peer socket id".to_string());
    mutations.push((
        "non-canonical connection id",
        invalid_connection_id,
        canonical_payload.clone(),
    ));

    let mut explicit_null_business_identity = canonical.clone();
    explicit_null_business_identity["websocketJsonRpc"]["businessIdentity"] = Value::Null;
    mutations.push((
        "explicit null business identity",
        explicit_null_business_identity,
        canonical_payload.clone(),
    ));

    let mut oversized_business_identity = canonical.clone();
    oversized_business_identity["websocketJsonRpc"]["businessIdentity"] =
        Value::String("b".repeat(1025));
    mutations.push((
        "oversized business identity",
        oversized_business_identity,
        canonical_payload.clone(),
    ));

    let mut controlled_business_identity = canonical.clone();
    controlled_business_identity["websocketJsonRpc"]["businessIdentity"] =
        Value::String("tenant\u{0085}one".to_string());
    mutations.push((
        "control-character business identity",
        controlled_business_identity,
        canonical_payload.clone(),
    ));

    mutations.push(("missing payload", canonical.clone(), Vec::new()));
    mutations.push((
        "payload above limit",
        canonical.clone(),
        vec![b'x'; 1024 * 1024 + 1],
    ));

    for (name, header, payload) in mutations {
        let frame = encode_binary_frame(&header, &payload).unwrap();
        assert!(
            decode_runtime_assembly_request_start_frame(&frame).is_err(),
            "{name}"
        );
    }
}

#[test]
fn runtime_assembly_websocket_jsonrpc_response_outcomes_enforce_payload_presence() {
    let success = websocket_jsonrpc_response_header("success", true);
    let success_frame = encode_binary_frame(&success, b"null").unwrap();
    let (decoded, payload) =
        decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&success_frame)
            .expect("success with JSON null payload must decode");
    assert_eq!(payload, b"null");
    assert_eq!(serde_json::to_value(decoded).unwrap(), success);

    for outcome in ["invalidParams", "internalError", "deadlineExceeded"] {
        let header = websocket_jsonrpc_response_header(outcome, false);
        let frame = encode_binary_frame(&header, &[]).unwrap();
        let (_, payload) = decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&frame)
            .unwrap_or_else(|error| panic!("{outcome}: {error}"));
        assert!(payload.is_empty(), "{outcome}");
    }
}

#[test]
fn runtime_assembly_websocket_jsonrpc_response_mutations_fail_closed() {
    let success = websocket_jsonrpc_response_header("success", true);
    let error = websocket_jsonrpc_response_header("invalidParams", false);
    let mut cases = Vec::new();

    cases.push(("success missing payload", success.clone(), Vec::new()));
    cases.push((
        "success payload above limit",
        success.clone(),
        vec![b'x'; 1024 * 1024 + 1],
    ));
    cases.push(("error carrying payload", error.clone(), b"null".to_vec()));

    let mut wrong_success_presence = success.clone();
    wrong_success_presence["payloadPresent"] = Value::Bool(false);
    cases.push((
        "success payloadPresent false",
        wrong_success_presence,
        b"null".to_vec(),
    ));

    let mut wrong_error_presence = error.clone();
    wrong_error_presence["payloadPresent"] = Value::Bool(true);
    cases.push((
        "error payloadPresent true",
        wrong_error_presence,
        Vec::new(),
    ));

    for outcome in ["cancelled", "unknown"] {
        cases.push((
            outcome,
            websocket_jsonrpc_response_header(outcome, false),
            Vec::new(),
        ));
    }

    let mut unknown_top_level = error.clone();
    unknown_top_level["message"] = Value::String("must-not-enter-wire".to_string());
    cases.push(("unknown top-level field", unknown_top_level, Vec::new()));

    let mut noncanonical_request_id = error.clone();
    noncanonical_request_id["requestId"] = Value::String(" request-id ".to_string());
    cases.push((
        "non-canonical request id",
        noncanonical_request_id,
        Vec::new(),
    ));

    let mut controlled_request_id = error.clone();
    controlled_request_id["requestId"] = Value::String("request\u{0085}id".to_string());
    cases.push((
        "control-character request id",
        controlled_request_id,
        Vec::new(),
    ));

    let mut unknown_nested = error;
    unknown_nested["websocketJsonRpc"]["stack"] = Value::String("must-not-enter-wire".to_string());
    cases.push(("unknown nested field", unknown_nested, Vec::new()));

    for (name, header, payload) in cases {
        let frame = encode_binary_frame(&header, &payload).unwrap();
        assert!(
            decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&frame).is_err(),
            "{name}"
        );
    }
}

#[test]
fn runtime_assembly_websocket_jsonrpc_response_rejects_duplicate_json_keys() {
    let frame = raw_json_frame_with_payload(
        br#"{"schemaVersion":"skiff-runtime-frame-v3","type":"response.end","requestId":"one","requestId":"two","payloadPresent":true,"websocketJsonRpc":{"outcome":"success"}}"#,
        b"null",
    );
    assert!(decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&frame).is_err());
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
        br#"{"schemaVersion":"skiff-runtime-frame-v3","type":"response.end","requestId":"one","requestId":"two","payloadPresent":false,"websocketConnect":{"result":"accept"}}"#,
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

fn websocket_connect_v2_header() -> Value {
    let mut header = connect_wire_corpus()
        .request_cases
        .into_iter()
        .find(|case| case.kind == "websocketConnect")
        .expect("websocketConnect baseline")
        .header;
    let identity = Value::String(
        "skiff-gateway-entry-v2:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            .to_string(),
    );
    header["requestId"] = Value::String("request-websocket-connect-v2".to_string());
    header["routing"]["gatewayEntryIdentity"] = identity.clone();
    header["websocketConnect"]["gatewayEntryIdentity"] = identity;
    header
}

fn websocket_jsonrpc_request_header() -> Value {
    let mut header = websocket_connect_v2_header();
    header["requestId"] = Value::String("request-websocket-jsonrpc-1".to_string());
    header["routing"]["ingress"]["method"] = Value::String("status.get".to_string());
    let websocket_connect = header
        .as_object_mut()
        .expect("request object")
        .remove("websocketConnect")
        .expect("websocketConnect metadata");
    let websocket_connect = websocket_connect
        .as_object()
        .expect("websocketConnect object");
    let connection_id = websocket_connect["connectionId"].clone();
    let websocket_entry_id = websocket_connect["websocketEntryId"].clone();
    let gateway_entry_identity = header["routing"]["gatewayEntryIdentity"].clone();
    header.as_object_mut().expect("request object").insert(
        "websocketJsonRpc".to_string(),
        serde_json::json!({
            "profile": "jsonrpc-2.0-text",
            "connectionId": connection_id,
            "websocketEntryId": websocket_entry_id,
            "gatewayEntryIdentity": gateway_entry_identity,
            "businessIdentity": "tenant-1"
        }),
    );
    header
}

fn websocket_jsonrpc_response_header(outcome: &str, payload_present: bool) -> Value {
    serde_json::json!({
        "schemaVersion": "skiff-runtime-frame-v3",
        "type": "response.end",
        "requestId": "request-websocket-jsonrpc-1",
        "payloadPresent": payload_present,
        "websocketJsonRpc": {
            "outcome": outcome
        }
    })
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
    raw_json_frame_with_payload(header, &[])
}

fn raw_json_frame_with_payload(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + header.len() + payload.len());
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(header);
    frame.extend_from_slice(payload);
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

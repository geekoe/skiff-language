use serde::Deserialize;
use serde_json::Value;

use crate::protocol::{
    BINARY_FRAME_HEADER_ENCODING_JSON, BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION,
};

use super::{
    assert_websocket_generation_lifecycle_response_matches,
    decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Corpus {
    valid_controls: Vec<ValidControl>,
    control_mutations: Vec<ControlMutation>,
    raw_invalid_controls: Vec<RawInvalidControl>,
    response_correlations: Vec<ResponseCorrelation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ValidControl {
    name: String,
    direction: String,
    control: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ControlMutation {
    name: String,
    base_index: usize,
    direction: String,
    set_path: Option<String>,
    remove_path: Option<String>,
    #[serde(default)]
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawInvalidControl {
    name: String,
    direction: String,
    raw_json: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResponseCorrelation {
    name: String,
    request_index: usize,
    response_index: usize,
    matches: bool,
    set_path: Option<String>,
    #[serde(default)]
    value: Value,
}

#[test]
fn exact_acquire_release_ack_and_rejection_round_trip_shared_corpus() {
    let corpus = corpus();
    assert_eq!(corpus.valid_controls.len(), 7);
    for fixture in corpus.valid_controls {
        let direction = direction(&fixture.direction);
        let control: WebSocketGenerationLifecycleControl =
            serde_json::from_value(fixture.control.clone())
                .unwrap_or_else(|error| panic!("{} typed decode: {error}", fixture.name));
        let frame = encode_websocket_generation_lifecycle_frame(direction, &control)
            .unwrap_or_else(|error| panic!("{} encode: {error}", fixture.name));
        let decoded = decode_websocket_generation_lifecycle_frame(direction, &frame)
            .unwrap_or_else(|error| panic!("{} decode: {error}", fixture.name));
        assert_eq!(decoded, control, "{}", fixture.name);
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            fixture.control,
            "{} JSON fields",
            fixture.name
        );
    }
}

#[test]
fn exact_duplicate_release_is_the_same_idempotency_key() {
    let corpus = corpus();
    let original: WebSocketGenerationLifecycleControl =
        serde_json::from_value(corpus.valid_controls[3].control.clone()).unwrap();
    let duplicate: WebSocketGenerationLifecycleControl =
        serde_json::from_value(corpus.valid_controls[6].control.clone()).unwrap();
    assert_eq!(duplicate, original);
}

#[test]
fn control_mutations_fail_closed() {
    let corpus = corpus();
    assert_eq!(corpus.control_mutations.len(), 24);
    for mutation in corpus.control_mutations {
        let mut value = corpus.valid_controls[mutation.base_index].control.clone();
        apply_mutation(&mut value, &mutation);
        let frame = crate::protocol::encode_binary_frame(&value, &[]).unwrap();
        assert!(
            decode_websocket_generation_lifecycle_frame(direction(&mutation.direction), &frame,)
                .is_err(),
            "{}",
            mutation.name
        );
    }
}

#[test]
fn duplicate_json_keys_and_non_empty_payload_fail_closed() {
    let corpus = corpus();
    assert_eq!(corpus.raw_invalid_controls.len(), 2);
    for invalid in corpus.raw_invalid_controls {
        let frame = frame_with_raw_header(invalid.raw_json.as_bytes(), &[]);
        assert!(
            decode_websocket_generation_lifecycle_frame(direction(&invalid.direction), &frame,)
                .is_err(),
            "{}",
            invalid.name
        );
    }

    let fixture = &corpus.valid_controls[0];
    let frame = frame_with_raw_header(
        serde_json::to_string(&fixture.control).unwrap().as_bytes(),
        &[1],
    );
    assert!(
        decode_websocket_generation_lifecycle_frame(direction(&fixture.direction), &frame,)
            .is_err()
    );
}

#[test]
fn responses_must_echo_exact_operation_request_id_and_tuple() {
    let corpus = corpus();
    assert_eq!(corpus.response_correlations.len(), 7);
    for correlation in corpus.response_correlations {
        let request: WebSocketGenerationLifecycleControl = serde_json::from_value(
            corpus.valid_controls[correlation.request_index]
                .control
                .clone(),
        )
        .unwrap();
        let mut response_value = corpus.valid_controls[correlation.response_index]
            .control
            .clone();
        if let Some(path) = &correlation.set_path {
            set_path(&mut response_value, path, correlation.value.clone());
        }
        let response: WebSocketGenerationLifecycleControl =
            serde_json::from_value(response_value).unwrap();
        let result = assert_websocket_generation_lifecycle_response_matches(&request, &response);
        assert_eq!(
            result.is_ok(),
            correlation.matches,
            "{}: {result:?}",
            correlation.name
        );
    }
}

fn corpus() -> Corpus {
    serde_json::from_str(include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/websocket-generation-lifecycle-wire.json"
    ))
    .expect("shared websocket generation lifecycle corpus")
}

fn direction(value: &str) -> WebSocketGenerationLifecycleDirection {
    match value {
        "routerToRuntime" => WebSocketGenerationLifecycleDirection::RouterToRuntime,
        "runtimeToRouter" => WebSocketGenerationLifecycleDirection::RuntimeToRouter,
        other => panic!("unknown lifecycle direction {other}"),
    }
}

fn apply_mutation(value: &mut Value, mutation: &ControlMutation) {
    if let Some(path) = &mutation.set_path {
        set_path(value, path, mutation.value.clone());
        return;
    }
    if let Some(path) = &mutation.remove_path {
        remove_path(value, path);
    }
}

fn set_path(root: &mut Value, path: &str, value: Value) {
    let (owner, leaf) = path_owner(root, path);
    owner.insert(leaf.to_string(), value);
}

fn remove_path(root: &mut Value, path: &str) {
    let (owner, leaf) = path_owner(root, path);
    owner.remove(leaf);
}

fn path_owner<'a>(
    root: &'a mut Value,
    path: &'a str,
) -> (&'a mut serde_json::Map<String, Value>, &'a str) {
    let mut segments = path.split('.').collect::<Vec<_>>();
    let leaf = segments.pop().expect("mutation path");
    let mut owner = root;
    for segment in segments {
        owner = owner
            .as_object_mut()
            .and_then(|object| object.get_mut(segment))
            .expect("mutation owner");
    }
    (owner.as_object_mut().expect("mutation leaf owner"), leaf)
}

fn frame_with_raw_header(header: &[u8], payload: &[u8]) -> Vec<u8> {
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

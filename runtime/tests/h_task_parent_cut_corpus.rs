//! H-task-parent-cut shared-corpus consumer gate (Runtime crate side).
//!
//! Consumes the frozen C-model-task corpus
//! (`transport/testdata/task-wire/`) through the canonical task codec:
//! every non-legacy frame roundtrips byte-exact, the legacy old shape (no
//! `callerKind`) has no compatible reader, and every frozen parent-resolution
//! scenario replays through the reference resolver/router model with typed
//! `(callerKind, callerRequestId)` namespaces (collision, parent terminal,
//! replacement, connection and authority mismatch all fail closed).

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_task_cancel_error_frame, decode_task_cancel_request_frame,
    decode_task_cancel_response_frame, decode_task_status_error_frame,
    decode_task_status_request_frame, decode_task_status_response_frame,
    decode_task_submit_error_frame, decode_task_submit_request_frame,
    decode_task_submit_response_frame, encode_binary_frame, encode_task_cancel_error_frame,
    encode_task_cancel_request_frame, encode_task_cancel_response_frame,
    encode_task_status_error_frame, encode_task_status_request_frame,
    encode_task_status_response_frame, encode_task_submit_error_frame,
    encode_task_submit_request_frame, encode_task_submit_response_frame,
    TaskSubmitRequestFrameHeaderV2,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameWireHeader,
};

const REQUIRED_FRAMES: [&str; 22] = [
    "task.submit.request.function",
    "task.submit.request.actorMethod",
    "task.submit.request.legacy-no-caller-kind",
    "task.submit.request.timing.after",
    "task.submit.request.timing.at",
    "task.submit.response",
    "task.submit.error.parentNotFound",
    "task.submit.error.invalidTiming",
    "task.submit.error.payloadInvalid",
    "task.submit.error.quotaExceeded",
    "task.submit.error.storeUnavailable",
    "task.submit.error.rejected",
    "task.status.request",
    "task.status.response.scheduled",
    "task.status.error.notFound",
    "task.status.error.storeUnavailable",
    "task.cancel.request",
    "task.cancel.response.canceled",
    "task.cancel.error.notFound",
    "task.cancel.error.storeUnavailable",
    "request.start.task.without-attempt",
    "request.start.task.with-attempt",
];

const REQUIRED_SCENARIOS: [&str; 10] = [
    "resolve-function-parent-exact",
    "resolve-actor-invocation-parent-exact",
    "same-request-id-both-namespaces-no-collision",
    "missing-caller-kind-legacy-cut-rejected",
    "parent-terminal-before-submit-rejected",
    "parent-replaced-before-submit-rejected",
    "parent-connection-mismatch-rejected",
    "authority-mismatch-rejected",
    "accepted-task-outlives-parent-terminal",
    "target-kind-mismatch-rejected",
];

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "payloadBase64")]
    payload_base64: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    #[serde(rename = "legacyCut")]
    legacy_cut: bool,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn catalog() -> Catalog {
    let value = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("transport/testdata/task-wire/frames.json"),
    )
    .expect("task-wire frames.json must be readable");
    serde_json::from_str(&value).expect("task-wire frames.json must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("frame hex"))
        .collect()
}

// ---------------------------------------------------------------------------
// Resolver / router reference model (C-model-task §4 / C-task §2-§4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ParentRecord {
    runtime_id: String,
    connection: String,
    assembly_generation: u64,
    test_case_capability: Option<String>,
    active: bool,
    replaced: bool,
}

#[derive(Debug, Default)]
struct ParentStores {
    request: HashMap<String, ParentRecord>,
    actor_invocation: HashMap<String, ParentRecord>,
}

impl ParentStores {
    fn get(&self, caller_kind: &str, id: &str) -> Option<&ParentRecord> {
        match caller_kind {
            "request" => self.request.get(id),
            "actorInvocation" => self.actor_invocation.get(id),
            _ => None,
        }
    }

    fn get_mut(&mut self, caller_kind: &str, id: &str) -> Option<&mut ParentRecord> {
        match caller_kind {
            "request" => self.request.get_mut(id),
            "actorInvocation" => self.actor_invocation.get_mut(id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ParentJson {
    id: String,
    runtime_id: String,
    connection: String,
    assembly_generation: u64,
    test_case_capability: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ParentsJson {
    request: Vec<ParentJson>,
    #[serde(rename = "actorInvocation")]
    actor_invocation: Vec<ParentJson>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitEvent {
    op: String,
    #[serde(default)]
    legacy: bool,
    #[serde(default)]
    caller_kind: Option<String>,
    #[serde(default)]
    caller_request_id: Option<String>,
    #[serde(default)]
    target_kind: Option<String>,
    #[serde(default)]
    actor_method: bool,
    #[serde(default)]
    connection: Option<String>,
    #[serde(default)]
    assembly_generation: Option<u64>,
    #[serde(default)]
    test_case_capability: Option<String>,
    #[serde(default)]
    new_connection: Option<String>,
    #[serde(default)]
    new_runtime_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskScenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    parents: ParentsJson,
    events: Vec<SubmitEvent>,
    expect: TaskExpect,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskExpect {
    accepted: Vec<String>,
    rejected: Vec<String>,
    errors: BTreeMap<String, String>,
    #[serde(default)]
    accepted_tasks: usize,
}

fn event_key(event: &SubmitEvent) -> String {
    if event.legacy {
        format!(
            "legacy:{}",
            event.caller_request_id.as_deref().unwrap_or("?")
        )
    } else {
        format!(
            "{}:{}",
            event.caller_kind.as_deref().unwrap_or("?"),
            event.caller_request_id.as_deref().unwrap_or("?")
        )
    }
}

fn reference_submit(stores: &ParentStores, event: &SubmitEvent) -> Result<String, &'static str> {
    if event.legacy || event.caller_kind.is_none() {
        return Err("CallerKindRejected");
    }
    let caller_kind = event.caller_kind.as_deref().expect("callerKind");
    if caller_kind != "request" && caller_kind != "actorInvocation" {
        return Err("CallerKindRejected");
    }
    let parent = stores
        .get(
            caller_kind,
            event.caller_request_id.as_deref().expect("callerRequestId"),
        )
        .ok_or("ParentNotFound")?;
    if !parent.active {
        return Err("ParentTerminal");
    }
    if parent.replaced {
        return Err("ParentReplaced");
    }
    if parent.connection != event.connection.as_deref().unwrap_or("") {
        return Err("ParentConnectionMismatch");
    }
    if parent.test_case_capability.is_some()
        && event.test_case_capability != parent.test_case_capability
    {
        return Err("TestCapabilityMismatch");
    }
    if let Some(generation) = event.assembly_generation {
        if generation != parent.assembly_generation {
            return Err("AuthorityMismatch");
        }
    }
    let target_kind = event.target_kind.as_deref().ok_or("TargetKindMismatch")?;
    if target_kind == "actorMethod" && !event.actor_method {
        return Err("TargetKindMismatch");
    }
    if target_kind == "function" && event.actor_method {
        return Err("TargetKindMismatch");
    }
    Ok(event_key(event))
}

fn run_task_scenario(expected_name: &str, raw: &str) {
    let scenario: TaskScenario = serde_json::from_str(raw).expect("task scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.scenario, expected_name);
    let mut stores = ParentStores::default();
    for parent in scenario.parents.request {
        stores.request.insert(
            parent.id.clone(),
            ParentRecord {
                runtime_id: parent.runtime_id,
                connection: parent.connection,
                assembly_generation: parent.assembly_generation,
                test_case_capability: parent.test_case_capability,
                active: true,
                replaced: false,
            },
        );
    }
    for parent in scenario.parents.actor_invocation {
        stores.actor_invocation.insert(
            parent.id.clone(),
            ParentRecord {
                runtime_id: parent.runtime_id,
                connection: parent.connection,
                assembly_generation: parent.assembly_generation,
                test_case_capability: parent.test_case_capability,
                active: true,
                replaced: false,
            },
        );
    }
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut errors = BTreeMap::new();
    let mut accepted_tasks = 0;
    for event in &scenario.events {
        match event.op.as_str() {
            "submit" => match reference_submit(&stores, event) {
                Ok(key) => {
                    accepted.push(key);
                    accepted_tasks += 1;
                }
                Err(error) => {
                    let key = event_key(event);
                    rejected.push(key.clone());
                    errors.insert(key, error.to_string());
                }
            },
            "parentTerminal" => {
                let record = stores
                    .get_mut(
                        event
                            .caller_kind
                            .as_deref()
                            .expect("parentTerminal callerKind"),
                        event
                            .caller_request_id
                            .as_deref()
                            .expect("parentTerminal callerRequestId"),
                    )
                    .expect("parentTerminal parent exists");
                record.active = false;
            }
            "replace" => {
                let record = stores
                    .get_mut(
                        event.caller_kind.as_deref().expect("replace callerKind"),
                        event
                            .caller_request_id
                            .as_deref()
                            .expect("replace callerRequestId"),
                    )
                    .expect("replace parent exists");
                record.replaced = true;
                if let Some(connection) = &event.new_connection {
                    record.connection = connection.clone();
                }
                if let Some(runtime_id) = &event.new_runtime_id {
                    record.runtime_id = runtime_id.clone();
                }
            }
            other => panic!("unknown task scenario op {other}"),
        }
    }
    assert_eq!(accepted, scenario.expect.accepted, "accepted");
    assert_eq!(rejected, scenario.expect.rejected, "rejected");
    assert_eq!(errors, scenario.expect.errors, "errors");
    assert_eq!(
        accepted_tasks, scenario.expect.accepted_tasks,
        "acceptedTasks"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_consumer_roundtrips_task_wire_corpus_byte_exact() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "task-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "corpus must contain required frame {required}"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        assert!(
            catalog.frames["task.submit.request.legacy-no-caller-kind"].legacy_cut,
            "legacy old-shape frame must be legacyCut"
        );
        for (name, entry) in &catalog.frames {
            assert_eq!(
                entry.direction,
                expected_direction(name),
                "{name}: direction"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v3",
                "{name}: schemaVersion"
            );
            assert_eq!(entry.header["type"], entry.frame_type, "{name}: type");
        }

        for name in [
            "task.submit.request.function",
            "task.submit.request.actorMethod",
            "task.submit.request.timing.after",
            "task.submit.request.timing.at",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_task_submit_request_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                encode_task_submit_request_frame(&header, &payload).expect("re-encode"),
                bytes,
                "{name} must roundtrip byte-exact"
            );
            let fixture_header: TaskSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert!(!payload.is_empty(), "{name}: payload must be present");
            assert!(!entry.legacy_cut, "{name} must not be legacy cut");
        }

        let response = &catalog.frames["task.submit.response"];
        let header = decode_task_submit_response_frame(&hex_bytes(&response.frame_hex))
            .expect("task.submit.response must decode");
        assert_eq!(
            encode_task_submit_response_frame(&header).expect("response re-encode"),
            hex_bytes(&response.frame_hex),
            "task.submit.response must be byte-exact"
        );
        assert_eq!(header.status, "submitted");
        assert_eq!(header.task_ref.task_id(), "task-1");
        assert_eq!(header.task_ref.owner(), "example.com/docs");

        for name in [
            "task.submit.error.parentNotFound",
            "task.submit.error.invalidTiming",
            "task.submit.error.payloadInvalid",
            "task.submit.error.quotaExceeded",
            "task.submit.error.storeUnavailable",
            "task.submit.error.rejected",
        ] {
            let error = &catalog.frames[name];
            let header = decode_task_submit_error_frame(&hex_bytes(&error.frame_hex))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                encode_task_submit_error_frame(&header).expect("error re-encode"),
                hex_bytes(&error.frame_hex),
                "{name} must be byte-exact"
            );
        }
        assert_eq!(
            catalog.frames["task.submit.error.parentNotFound"]
                .header["error"]["code"],
            "ParentNotFound"
        );
    }

    #[test]
    fn runtime_consumer_roundtrips_status_cancel_and_request_start_frames() {
        let catalog = catalog();
        for name in [
            "task.status.request",
            "task.status.response.scheduled",
            "task.status.error.notFound",
            "task.status.error.storeUnavailable",
            "task.cancel.request",
            "task.cancel.response.canceled",
            "task.cancel.error.notFound",
            "task.cancel.error.storeUnavailable",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let reencoded = match entry.decode_as.as_str() {
                "TaskStatusRequest" => encode_task_status_request_frame(
                    &decode_task_status_request_frame(&bytes).expect(name),
                )
                .expect("status request re-encode"),
                "TaskStatusResponse" => encode_task_status_response_frame(
                    &decode_task_status_response_frame(&bytes).expect(name),
                )
                .expect("status response re-encode"),
                "TaskStatusError" => encode_task_status_error_frame(
                    &decode_task_status_error_frame(&bytes).expect(name),
                )
                .expect("status error re-encode"),
                "TaskCancelRequest" => encode_task_cancel_request_frame(
                    &decode_task_cancel_request_frame(&bytes).expect(name),
                )
                .expect("cancel request re-encode"),
                "TaskCancelResponse" => encode_task_cancel_response_frame(
                    &decode_task_cancel_response_frame(&bytes).expect(name),
                )
                .expect("cancel response re-encode"),
                "TaskCancelError" => encode_task_cancel_error_frame(
                    &decode_task_cancel_error_frame(&bytes).expect(name),
                )
                .expect("cancel error re-encode"),
                other => panic!("{name}: unexpected decodeAs {other}"),
            };
            assert_eq!(reencoded, bytes, "{name} must be byte-exact");
        }

        for name in [
            "request.start.task.without-attempt",
            "request.start.task.with-attempt",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_runtime_assembly_request_start_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let RuntimeAssemblyRequestStartFrameWireHeader::Task(header) = header else {
                panic!("{name}: must decode as task request.start")
            };
            assert_eq!(
                encode_binary_frame(&header, &payload).expect("re-encode"),
                bytes,
                "{name} must be byte-exact"
            );
            assert_eq!(
                header.task_attempt.is_some(),
                name == "request.start.task.with-attempt",
                "{name}: taskAttempt presence"
            );
        }
    }

    #[test]
    fn runtime_consumer_rejects_legacy_old_shape_with_no_compatible_reader() {
        let catalog = catalog();
        let entry = &catalog.frames["task.submit.request.legacy-no-caller-kind"];
        assert!(entry.legacy_cut);
        let error = decode_task_submit_request_frame(&hex_bytes(&entry.frame_hex))
            .expect_err("legacy old-shape frame must be rejected");
        assert!(
            error.to_string().contains("callerKind"),
            "legacy rejection must name callerKind, got {error}"
        );
    }

    #[test]
    fn runtime_consumer_sees_all_frozen_task_scenarios_and_replays_them() {
        let scenarios_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("transport/testdata/task-wire/scenarios");
        let mut names = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(&scenarios_dir)
            .expect("task scenarios dir must be readable")
            .map(|entry| entry.expect("scenario entry").path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect();
        entries.sort();
        for path in entries {
            let raw = std::fs::read_to_string(&path).expect("scenario must be readable");
            let scenario: TaskScenario = serde_json::from_str(&raw).expect("scenario must decode");
            names.push(scenario.scenario.clone());
            run_task_scenario(&scenario.scenario, &raw);
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "required task scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }

    fn expected_direction(name: &str) -> &'static str {
        match name {
            "task.submit.request.function"
            | "task.submit.request.actorMethod"
            | "task.submit.request.legacy-no-caller-kind"
            | "task.submit.request.timing.after"
            | "task.submit.request.timing.at"
            | "task.status.request"
            | "task.cancel.request" => "RuntimeToRouter",
            "task.submit.response"
            | "task.submit.error.parentNotFound"
            | "task.submit.error.invalidTiming"
            | "task.submit.error.payloadInvalid"
            | "task.submit.error.quotaExceeded"
            | "task.submit.error.storeUnavailable"
            | "task.submit.error.rejected"
            | "task.status.response.scheduled"
            | "task.status.error.notFound"
            | "task.status.error.storeUnavailable"
            | "task.cancel.response.canceled"
            | "task.cancel.error.notFound"
            | "task.cancel.error.storeUnavailable"
            | "request.start.task.without-attempt"
            | "request.start.task.with-attempt" => "RouterToRuntime",
            _ => panic!("unexpected task frame {name}"),
        }
    }
}

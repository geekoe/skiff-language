//! Task-family byte-exact wire corpus + resolver/router reference model for
//! C-model-task / C-task
//! (`doc/implementation/router-rust-migration-c-model-task-contract.md`,
//! `doc/implementation/router-rust-migration-c-task-contract.md`).
//!
//! The target wire freezes the explicit closed enum
//! `callerKind = request | actorInvocation` (plan §5.3) with a typed parent
//! namespace and no string-prefix fallback. The old shape (no `callerKind`)
//! is `legacyCut` and must be rejected with no compatible reader. The frame
//! hexes are frozen from the target mirror (C-model-task); since W-model-task
//! the production canonical codec carries `callerKind` and takes over the same
//! corpus byte-exactly. `H-task-parent-cut` is exactly the later change that
//! switches the production consumers and deletes the old shape.

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_task_cancel_request_frame, decode_task_cancel_response_frame,
    decode_task_status_request_frame, decode_task_status_response_frame,
    decode_task_submit_error_frame, decode_task_submit_request_frame,
    decode_task_submit_response_frame, encode_task_cancel_request_frame,
    encode_task_cancel_response_frame, encode_task_status_request_frame,
    encode_task_status_response_frame, encode_task_submit_error_frame,
    encode_task_submit_request_frame, encode_task_submit_response_frame,
    TaskSubmitRejectionCode, TaskSubmitRequestFrameHeaderV2,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_request_start_frame, RuntimeAssemblyRequestStartFrameWireHeader,
};

const REQUIRED_FRAMES: [&str; 18] = [
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
    "task.cancel.request",
    "task.cancel.response.canceled",
    "request.start.task.without-attempt",
    "request.start.task.with-attempt",
];

#[derive(Debug, Clone, Deserialize)]
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
    serde_json::from_str(include_str!("../testdata/task-wire/frames.json"))
        .expect("task wire corpus must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("frameHex hex"))
        .collect()
}

fn payload_of(entry: &FrameEntry) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(&entry.payload_base64)
        .expect("payloadBase64 must be canonical base64")
}

// ---------------------------------------------------------------------------
// Resolver / router reference model (C-task)
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

#[derive(Debug, Default)]
struct TaskSubmitRouter {
    accepted_tasks: usize,
}

impl TaskSubmitRouter {
    fn submit(
        &mut self,
        stores: &ParentStores,
        event: &SubmitEvent,
    ) -> Result<String, &'static str> {
        let key = if event.legacy {
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
        };
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
        self.accepted_tasks += 1;
        Ok(key)
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
    let mut router = TaskSubmitRouter::default();
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut errors = BTreeMap::new();
    for event in &scenario.events {
        match event.op.as_str() {
            "submit" => {
                let key = if event.legacy {
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
                };
                match router.submit(&stores, event) {
                    Ok(_) => accepted.push(key),
                    Err(error) => {
                        rejected.push(key.clone());
                        errors.insert(key, error.to_string());
                    }
                }
            }
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
        router.accepted_tasks, scenario.expect.accepted_tasks,
        "acceptedTasks"
    );
}

const TASK_SCENARIOS: [(&str, &str); 10] = [
    (
        "resolve-function-parent-exact",
        include_str!("../testdata/task-wire/scenarios/01-resolve-function-parent-exact.json"),
    ),
    (
        "resolve-actor-invocation-parent-exact",
        include_str!(
            "../testdata/task-wire/scenarios/02-resolve-actor-invocation-parent-exact.json"
        ),
    ),
    (
        "same-request-id-both-namespaces-no-collision",
        include_str!(
            "../testdata/task-wire/scenarios/03-same-request-id-both-namespaces-no-collision.json"
        ),
    ),
    (
        "missing-caller-kind-legacy-cut-rejected",
        include_str!(
            "../testdata/task-wire/scenarios/04-missing-caller-kind-legacy-cut-rejected.json"
        ),
    ),
    (
        "parent-terminal-before-submit-rejected",
        include_str!(
            "../testdata/task-wire/scenarios/05-parent-terminal-before-submit-rejected.json"
        ),
    ),
    (
        "parent-replaced-before-submit-rejected",
        include_str!(
            "../testdata/task-wire/scenarios/06-parent-replaced-before-submit-rejected.json"
        ),
    ),
    (
        "parent-connection-mismatch-rejected",
        include_str!(
            "../testdata/task-wire/scenarios/07-parent-connection-mismatch-rejected.json"
        ),
    ),
    (
        "authority-mismatch-rejected",
        include_str!("../testdata/task-wire/scenarios/08-authority-mismatch-rejected.json"),
    ),
    (
        "accepted-task-outlives-parent-terminal",
        include_str!(
            "../testdata/task-wire/scenarios/09-accepted-task-outlives-parent-terminal.json"
        ),
    ),
    (
        "target-kind-mismatch-rejected",
        include_str!("../testdata/task-wire/scenarios/10-target-kind-mismatch-rejected.json"),
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_required_frames_are_frozen() {
        let catalog = catalog();
        assert_eq!(catalog.schema_version, 1);
        assert_eq!(catalog.corpus, "task-wire-v1");
        for required in REQUIRED_FRAMES {
            assert!(
                catalog.frames.contains_key(required),
                "required task frame {required} is missing"
            );
        }
        assert_eq!(catalog.frames.len(), REQUIRED_FRAMES.len());
        let legacy = &catalog.frames["task.submit.request.legacy-no-caller-kind"];
        assert!(
            legacy.legacy_cut,
            "legacy old-shape frame must be legacyCut"
        );
    }

    #[test]
    fn task_family_rule_is_mixed_direction_with_required_payload_and_frame_table() {
        use skiff_runtime_transport::protocol::task_submit_frame_direction;
        use skiff_runtime_transport::protocol::{
            FrameDirection, PayloadPresenceRule, RuntimeFrameFamily, TASK_SUBMIT_ERROR_FRAME_TYPE,
            TASK_SUBMIT_REQUEST_FRAME_TYPE, TASK_SUBMIT_RESPONSE_FRAME_TYPE,
            TASK_CANCEL_REQUEST_FRAME_TYPE, TASK_CANCEL_RESPONSE_FRAME_TYPE,
            TASK_STATUS_REQUEST_FRAME_TYPE, TASK_STATUS_RESPONSE_FRAME_TYPE,
        };
        assert_eq!(
            RuntimeFrameFamily::Task.direction(),
            FrameDirection::Either,
            "task family is mixed-direction; consumers narrow per frame"
        );
        assert_eq!(
            RuntimeFrameFamily::Task.payload_presence(),
            PayloadPresenceRule::Required
        );
        assert_eq!(RuntimeFrameFamily::Task.wire_type_prefix(), "task.");
        assert_eq!(
            task_submit_frame_direction(TASK_SUBMIT_REQUEST_FRAME_TYPE),
            Some(FrameDirection::RuntimeToRouter)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_SUBMIT_RESPONSE_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_SUBMIT_ERROR_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_STATUS_REQUEST_FRAME_TYPE),
            Some(FrameDirection::RuntimeToRouter)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_STATUS_RESPONSE_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_CANCEL_REQUEST_FRAME_TYPE),
            Some(FrameDirection::RuntimeToRouter)
        );
        assert_eq!(
            task_submit_frame_direction(TASK_CANCEL_RESPONSE_FRAME_TYPE),
            Some(FrameDirection::RouterToRuntime)
        );
    }

    #[test]
    fn frame_metadata_is_frozen() {
        let catalog = catalog();
        for (name, entry) in &catalog.frames {
            assert_eq!(
                entry.direction,
                expected_direction(name),
                "{name}: direction"
            );
            let (frame_type, decode_as, presence) = match name.as_str() {
                "task.submit.request.function"
                | "task.submit.request.actorMethod"
                | "task.submit.request.legacy-no-caller-kind"
                | "task.submit.request.timing.after"
                | "task.submit.request.timing.at" => {
                    ("task.submit.request", "TaskSubmitRequest", "required")
                }
                "task.submit.response" => {
                    ("task.submit.response", "TaskSubmitResponse", "empty")
                }
                "task.submit.error.parentNotFound"
                | "task.submit.error.invalidTiming"
                | "task.submit.error.payloadInvalid"
                | "task.submit.error.quotaExceeded"
                | "task.submit.error.storeUnavailable"
                | "task.submit.error.rejected" => {
                    ("task.submit.error", "TaskSubmitError", "empty")
                }
                "task.status.request" => {
                    ("task.status.request", "TaskStatusRequest", "empty")
                }
                "task.status.response.scheduled" => {
                    ("task.status.response", "TaskStatusResponse", "empty")
                }
                "task.cancel.request" => {
                    ("task.cancel.request", "TaskCancelRequest", "empty")
                }
                "task.cancel.response.canceled" => {
                    ("task.cancel.response", "TaskCancelResponse", "empty")
                }
                "request.start.task.without-attempt" | "request.start.task.with-attempt" => {
                    ("request.start", "RuntimeAssemblyTaskRequestStart", "required")
                }
                _ => panic!("unexpected task frame {name}"),
            };
            assert_eq!(entry.frame_type, frame_type, "{name}: frameType");
            assert_eq!(entry.decode_as, decode_as, "{name}: decodeAs");
            assert_eq!(entry.payload_presence, presence, "{name}: payloadPresence");
        }
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
            | "task.cancel.response.canceled"
            | "request.start.task.without-attempt"
            | "request.start.task.with-attempt" => "RouterToRuntime",
            _ => panic!("unexpected task frame {name}"),
        }
    }

    #[test]
    fn target_shaped_requests_are_byte_exact_and_payload_consistent() {
        let catalog = catalog();
        for name in [
            "task.submit.request.function",
            "task.submit.request.actorMethod",
            "task.submit.request.timing.after",
            "task.submit.request.timing.at",
        ] {
            let entry = &catalog.frames[name];
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_task_submit_request_frame(&bytes)
                .expect("target shape must decode through the canonical codec");
            let reencoded = encode_task_submit_request_frame(&header, &payload)
                .expect("target shape re-encode");
            assert_eq!(bytes, reencoded, "{name} must be byte-exact");
            let fixture_header: TaskSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert_eq!(payload, payload_of(entry), "{name} payload mismatch");
            assert!(!entry.legacy_cut, "{name} is not legacy cut");
        }
        let after = decode_task_submit_request_frame(&hex_bytes(
            &catalog.frames["task.submit.request.timing.after"].frame_hex,
        ))
        .expect("after timing request")
        .0;
        assert_eq!(
            after.timing,
            Some(skiff_runtime_transport::protocol::TaskSubmitTiming::After {
                duration_ms: 5_000
            })
        );
        let at = decode_task_submit_request_frame(&hex_bytes(
            &catalog.frames["task.submit.request.timing.at"].frame_hex,
        ))
        .expect("at timing request")
        .0;
        assert_eq!(
            at.timing,
            Some(skiff_runtime_transport::protocol::TaskSubmitTiming::At {
                utc_millis: 1_700_000_000_000
            })
        );
    }

    #[test]
    fn legacy_old_shape_has_no_compatible_reader() {
        let catalog = catalog();
        let entry = &catalog.frames["task.submit.request.legacy-no-caller-kind"];
        assert!(entry.legacy_cut);
        let bytes = hex_bytes(&entry.frame_hex);
        let error = decode_task_submit_request_frame(&bytes).expect_err(
            "legacy old-shape frame (no callerKind) must be rejected by the canonical codec",
        );
        assert!(
            error.to_string().contains("callerKind"),
            "legacy rejection must name callerKind, got {error}"
        );
        // No fallback: the closed enum has exactly two values and the canonical
        // codec requires the field. A reader that guessed a default would accept
        // this frame; the frozen contract forbids that reader.
        let header_json: Value = serde_json::from_str(
            r#"{
                "schemaVersion":"skiff-runtime-frame-v3",
                "type":"task.submit.request",
                "rpcId":"rpc:probe-1",
                "runtimeId":"runtime-a",
                "callerKind":"function",
                "callerRequestId":"parent-1",
                "targetKind":"function",
                "serviceId":"example.com/docs",
                "serviceVersion":"1.0.0",
                "serviceProtocolIdentity":"example.com/docs:1.0.0",
                "target":"example.com/fn",
                "activationIdentity":{
                    "assemblyIdentity":"skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "generation":42,
                    "runtimeReplicaId":"runtime-a",
                    "deploymentRevision":"rev-1"
                }
            }"#,
        )
        .expect("probe json");
        let probe_result = serde_json::from_value::<TaskSubmitRequestFrameHeaderV2>(header_json);
        assert!(
            probe_result.is_err(),
            "callerKind=function is not a valid parent kind; closed enum must reject it"
        );
    }

    #[test]
    fn response_and_error_frames_round_trip_through_canonical_dtos() {
        let catalog = catalog();
        let entry = &catalog.frames["task.submit.response"];
        let header = decode_task_submit_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.submit.response must decode");
        assert_eq!(header.task_id, "task-1");
        assert_eq!(header.task_ref.task_id(), "task-1");
        assert_eq!(header.task_ref.owner(), "example.com/docs");
        assert_eq!(
            bytes_hex(&encode_task_submit_response_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "task.submit.response must be byte-exact"
        );

        for name in [
            "task.submit.error.parentNotFound",
            "task.submit.error.invalidTiming",
            "task.submit.error.payloadInvalid",
            "task.submit.error.quotaExceeded",
            "task.submit.error.storeUnavailable",
            "task.submit.error.rejected",
        ] {
            let entry = &catalog.frames[name];
            let header = decode_task_submit_error_frame(&hex_bytes(&entry.frame_hex))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                bytes_hex(&encode_task_submit_error_frame(&header).expect("re-encode")),
                entry.frame_hex,
                "{name} must be byte-exact"
            );
            assert_eq!(
                TaskSubmitRejectionCode::parse(&header.error.code).is_some(),
                name != "task.submit.error.parentNotFound",
                "{name}: D1 rejection-code projection"
            );
        }
        assert!(TaskSubmitRejectionCode::StoreUnavailable.is_transient());
        for code in [
            TaskSubmitRejectionCode::InvalidTiming,
            TaskSubmitRejectionCode::PayloadInvalid,
            TaskSubmitRejectionCode::QuotaExceeded,
            TaskSubmitRejectionCode::Rejected,
        ] {
            assert!(code.is_definite(), "{} must be definite", code.as_str());
        }
    }

    #[test]
    fn status_and_cancel_frames_round_trip_through_canonical_dtos() {
        use skiff_runtime_transport::protocol::{
            TaskCancelResultKindWire, TaskCancelResultWire, TaskStatusKindWire, TaskStatusWire,
        };

        let catalog = catalog();
        let entry = &catalog.frames["task.status.request"];
        let header = decode_task_status_request_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.status.request must decode");
        assert_eq!(
            bytes_hex(&encode_task_status_request_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "task.status.request must be byte-exact"
        );
        assert_eq!(header.task_ref.task_id(), "task-1");

        let entry = &catalog.frames["task.status.response.scheduled"];
        let header = decode_task_status_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.status.response must decode");
        assert_eq!(
            bytes_hex(&encode_task_status_response_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "task.status.response must be byte-exact"
        );
        assert_eq!(
            header.status,
            TaskStatusWire {
                kind: TaskStatusKindWire::Scheduled
            }
        );

        let entry = &catalog.frames["task.cancel.request"];
        let header = decode_task_cancel_request_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.cancel.request must decode");
        assert_eq!(
            bytes_hex(&encode_task_cancel_request_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "task.cancel.request must be byte-exact"
        );
        assert_eq!(header.task_ref.owner(), "example.com/docs");

        let entry = &catalog.frames["task.cancel.response.canceled"];
        let header = decode_task_cancel_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.cancel.response must decode");
        assert_eq!(
            bytes_hex(&encode_task_cancel_response_frame(&header).expect("re-encode")),
            entry.frame_hex,
            "task.cancel.response must be byte-exact"
        );
        assert_eq!(
            header.result,
            TaskCancelResultWire {
                kind: TaskCancelResultKindWire::Canceled
            }
        );
    }

    #[test]
    fn task_request_start_frames_carry_optional_task_attempt_header() {
        let catalog = catalog();
        let entry = &catalog.frames["request.start.task.without-attempt"];
        let bytes = hex_bytes(&entry.frame_hex);
        let (header, payload) = decode_runtime_assembly_request_start_frame(&bytes)
            .expect("task request.start without attempt must decode");
        let RuntimeAssemblyRequestStartFrameWireHeader::Task(header) = header else {
            panic!("request.start.task must decode as the task union branch")
        };
        assert!(header.task_attempt.is_none());
        assert_eq!(
            skiff_runtime_transport::protocol::encode_binary_frame(&header, &payload)
                .expect("re-encode"),
            bytes,
            "request.start.task.without-attempt must be byte-exact"
        );

        let entry = &catalog.frames["request.start.task.with-attempt"];
        let bytes = hex_bytes(&entry.frame_hex);
        let (header, payload) = decode_runtime_assembly_request_start_frame(&bytes)
            .expect("task request.start with attempt must decode");
        let RuntimeAssemblyRequestStartFrameWireHeader::Task(header) = header else {
            panic!("request.start.task must decode as the task union branch")
        };
        let attempt = header
            .task_attempt
            .as_ref()
            .expect("taskAttempt must be present");
        assert_eq!(attempt.task_id, "task-1");
        assert_eq!(attempt.attempt_id, "attempt-1");
        assert_eq!(attempt.lease_id, "lease-1");
        assert_eq!(
            skiff_runtime_transport::protocol::encode_binary_frame(&header, &payload)
                .expect("re-encode"),
            bytes,
            "request.start.task.with-attempt must be byte-exact"
        );
    }

    #[test]
    fn task_scenarios_drive_the_resolver_router_reference_model() {
        for (name, raw) in TASK_SCENARIOS {
            run_task_scenario(name, raw);
        }
    }

    #[test]
    fn required_task_scenario_names_are_frozen() {
        let required = [
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
        for (name, _) in TASK_SCENARIOS {
            assert!(
                required.contains(&name),
                "task scenario {name} is not in the frozen required list"
            );
        }
        for name in required {
            assert!(
                TASK_SCENARIOS
                    .iter()
                    .any(|(scenario, _)| *scenario == name),
                "required task scenario {name} is missing"
            );
        }
    }

    fn bytes_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

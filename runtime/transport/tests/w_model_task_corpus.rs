//! W-model-task corpus gate: the task-wire corpus is consumed through the
//! canonical production codec (`callerKind` closed enum + required
//! `callerRequestId`). The legacy old shape has no compatible reader; the
//! golden `frameHex` values are the C-model-task frozen bytes and must not
//! change.
//!
//! Durable task semantics are defined by
//! `doc/architecture/durable-task-dispatch.md`; this test owns the frozen
//! byte-exact transport corpus.

use std::collections::BTreeMap;

use base64::Engine as _;
use serde::Deserialize;
use serde_json::Value;
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, BytecodeRequestStartFrameWireHeader,
};
use skiff_runtime_transport::protocol::{
    decode_task_cancel_request_frame, decode_task_cancel_response_frame,
    decode_task_status_request_frame, decode_task_status_response_frame,
    decode_task_submit_error_frame, decode_task_submit_request_frame,
    decode_task_submit_response_frame, encode_binary_frame, encode_task_cancel_request_frame,
    encode_task_cancel_response_frame, encode_task_status_request_frame,
    encode_task_status_response_frame, encode_task_submit_error_frame,
    encode_task_submit_request_frame, encode_task_submit_response_frame,
    TaskSubmitRequestFrameHeaderV2,
};

const REQUIRED_FRAMES: [&str; 23] = [
    "task.submit.request.function",
    "task.submit.request.actorMethod",
    "task.submit.request.actorMethod.snapshot",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w_model_task_corpus_is_frozen() {
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
                entry.frame_type,
                expected_frame_type(name),
                "{name}: frameType"
            );
            assert_eq!(
                entry.decode_as,
                expected_decode_as(name),
                "{name}: decodeAs"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v5",
                "{name}: header schemaVersion"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
        }
    }

    #[test]
    fn w_model_task_canonical_requests_roundtrip_byte_exact() {
        let catalog = catalog();
        for name in [
            "task.submit.request.function",
            "task.submit.request.actorMethod",
            "task.submit.request.actorMethod.snapshot",
            "task.submit.request.timing.after",
            "task.submit.request.timing.at",
        ] {
            let entry = &catalog.frames[name];
            assert!(!entry.legacy_cut, "{name} must not be legacy cut");
            let bytes = hex_bytes(&entry.frame_hex);
            let (header, payload) = decode_task_submit_request_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let reencoded = encode_task_submit_request_frame(&header, &payload)
                .expect("canonical request re-encode");
            assert_eq!(reencoded, bytes, "{name} must be byte-exact");
            assert_eq!(payload, payload_of(entry), "{name} payload mismatch");
            let fixture_header: TaskSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert_eq!(entry.payload_presence, "required", "{name} presence");
        }
    }

    #[test]
    fn w_model_task_response_and_error_roundtrip_byte_exact() {
        let catalog = catalog();
        let entry = &catalog.frames["task.submit.response"];
        let header = decode_task_submit_response_frame(&hex_bytes(&entry.frame_hex))
            .expect("task.submit.response must decode");
        assert_eq!(
            encode_task_submit_response_frame(&header).expect("response re-encode"),
            hex_bytes(&entry.frame_hex),
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
            let entry = &catalog.frames[name];
            let header = decode_task_submit_error_frame(&hex_bytes(&entry.frame_hex))
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(
                encode_task_submit_error_frame(&header).expect("error re-encode"),
                hex_bytes(&entry.frame_hex),
                "{name} must be byte-exact"
            );
            assert!(!header.error.code.is_empty(), "{name}: code");
        }
        assert_eq!(
            catalog.frames["task.submit.error.parentNotFound"].header["error"]["code"],
            "ParentNotFound"
        );
    }

    #[test]
    fn w_model_task_legacy_old_shape_has_no_compatible_reader() {
        let catalog = catalog();
        let entry = &catalog.frames["task.submit.request.legacy-no-caller-kind"];
        assert!(entry.legacy_cut);
        let error = decode_task_submit_request_frame(&hex_bytes(&entry.frame_hex))
            .expect_err("legacy old-shape frame must be rejected");
        assert!(
            error.to_string().contains("callerKind"),
            "legacy rejection must name callerKind, got {error}"
        );

        // Closed enum: any non-request/actorInvocation value must fail even when
        // the field is present.
        let invalid_kind = serde_json::json!({
            "schemaVersion": "skiff-runtime-frame-v5",
            "type": "task.submit.request",
            "rpcId": "rpc:probe-1",
            "runtimeId": "runtime-a",
            "callerKind": "function",
            "callerRequestId": "parent-1",
            "targetKind": "function",
            "serviceId": "example.com/docs",
            "serviceVersion": "1.0.0",
            "serviceProtocolIdentity": "example.com/docs:1.0.0",
            "target": "example.com/fn",
            "activationIdentity": {
                "assemblyIdentity": format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "a".repeat(64)
                ),
                "generation": 42,
                "runtimeReplicaId": "runtime-a",
                "deploymentRevision": "rev-1"
            }
        });
        let bytes = encode_binary_frame(&invalid_kind, b"").expect("probe frame must encode");
        assert!(
            decode_task_submit_request_frame(&bytes).is_err(),
            "callerKind=function must be rejected by the closed enum"
        );
    }

    #[test]
    fn w_model_task_target_kind_and_actor_method_must_agree() {
        let base = serde_json::json!({
            "schemaVersion": "skiff-runtime-frame-v5",
            "type": "task.submit.request",
            "rpcId": "rpc:probe-1",
            "runtimeId": "runtime-a",
            "callerKind": "request",
            "callerRequestId": "parent-1",
            "serviceId": "example.com/docs",
            "serviceVersion": "1.0.0",
            "serviceProtocolIdentity": "example.com/docs:1.0.0",
            "target": "example.com/fn",
            "activationIdentity": {
                "assemblyIdentity": format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "a".repeat(64)
                ),
                "generation": 42,
                "runtimeReplicaId": "runtime-a",
                "deploymentRevision": "rev-1"
            }
        });
        let actor_method = serde_json::json!({
            "actorRef": {
                "serviceId": "example.com/docs",
                "actorTypeIdentity": "CounterActor",
                "actorIdTypeIdentity": "CounterId",
                "actorIdEncodingVersion": "skiff-actor-id-encoding-v1",
                "canonicalActorIdKeyBytesBase64": "AQID",
                "actorIdHash": format!("sha256:{}", "1".repeat(64)),
                "epoch": 7
            },
            "declarationOwner": {
                "unit": { "kind": "service" },
                "file": { "kind": "fileIrIdentity", "value": "file:1" },
                "actorSymbol": "Counter"
            },
            "actorAbiIdentity": format!("skiff-actor-abi-v1:sha256:{}", "a".repeat(64)),
            "actorImplementationIdentity": format!(
                "skiff-actor-implementation-v1:sha256:{}",
                "b".repeat(64)
            ),
            "methodIdentity": format!("skiff-actor-method-v1:sha256:{}", "c".repeat(64)),
            "activation": {
                "key": "eyJhY3RvcklkRW5jb2RpbmdWZXJzaW9uIjoic2tpZmYtYWN0b3ItaWQtZW5jb2RpbmctdjEiLCJhY3RvcklkSGFzaCI6InNoYTI1NjoxMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExMTExIiwiYWN0b3JJZFR5cGVJZGVudGl0eSI6IkNvdW50ZXJJZCIsImFjdG9yVHlwZUlkZW50aXR5IjoiQ291bnRlckFjdG9yIiwiY2Fub25pY2FsQWN0b3JJZEtleUJ5dGVzQmFzZTY0IjoiQVFJRCIsInNlcnZpY2VJZCI6ImV4YW1wbGUuY29tL2RvY3MifQ==",
                "createInput": "W10=",
                "expectedTypePlan": {
                    "label": "record",
                    "node": { "kind": "record", "fields": [] }
                }
            }
        });

        let mut function_with_actor_method = base.clone();
        function_with_actor_method["targetKind"] = Value::String("function".into());
        function_with_actor_method["actorMethod"] = actor_method.clone();
        let bytes = encode_binary_frame(&function_with_actor_method, b"").expect("encode");
        let error = decode_task_submit_request_frame(&bytes)
            .expect_err("function target must not carry actorMethod");
        assert!(error.to_string().contains("actorMethod"));

        let mut actor_method_without_metadata = base;
        actor_method_without_metadata["targetKind"] = Value::String("actorMethod".into());
        let bytes = encode_binary_frame(&actor_method_without_metadata, b"").expect("encode");
        let error = decode_task_submit_request_frame(&bytes)
            .expect_err("actorMethod target requires actorMethod metadata");
        assert!(error.to_string().contains("actorMethod"));
    }

    #[test]
    fn w_model_task_scenario_names_are_frozen() {
        let mut names = Vec::new();
        for entry in std::fs::read_dir(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/task-wire/scenarios"
        ))
        .expect("task scenarios dir must be readable")
        {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &std::fs::read_to_string(&path).expect("scenario must be readable"),
            )
            .expect("scenario must decode");
            names.push(
                value["scenario"]
                    .as_str()
                    .expect("scenario name")
                    .to_string(),
            );
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                names.iter().any(|name| name == required),
                "required task scenario {required} is missing"
            );
        }
        assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
    }

    #[test]
    fn w_model_task_status_cancel_and_request_start_frames_roundtrip_byte_exact() {
        let catalog = catalog();
        for name in [
            "task.status.request",
            "task.status.response.scheduled",
            "task.cancel.request",
            "task.cancel.response.canceled",
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
                "TaskCancelRequest" => encode_task_cancel_request_frame(
                    &decode_task_cancel_request_frame(&bytes).expect(name),
                )
                .expect("cancel request re-encode"),
                "TaskCancelResponse" => encode_task_cancel_response_frame(
                    &decode_task_cancel_response_frame(&bytes).expect(name),
                )
                .expect("cancel response re-encode"),
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
            let (header, payload) = decode_bytecode_request_start_frame(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let BytecodeRequestStartFrameWireHeader::Task(header) = header else {
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

    fn expected_frame_type(name: &str) -> &str {
        match name {
            "task.submit.request.function"
            | "task.submit.request.actorMethod"
            | "task.submit.request.actorMethod.snapshot"
            | "task.submit.request.legacy-no-caller-kind"
            | "task.submit.request.timing.after"
            | "task.submit.request.timing.at" => "task.submit.request",
            "task.submit.response" => "task.submit.response",
            "task.submit.error.parentNotFound"
            | "task.submit.error.invalidTiming"
            | "task.submit.error.payloadInvalid"
            | "task.submit.error.quotaExceeded"
            | "task.submit.error.storeUnavailable"
            | "task.submit.error.rejected" => "task.submit.error",
            "task.status.request" => "task.status.request",
            "task.status.response.scheduled" => "task.status.response",
            "task.status.error.notFound" | "task.status.error.storeUnavailable" => {
                "task.status.error"
            }
            "task.cancel.request" => "task.cancel.request",
            "task.cancel.response.canceled" => "task.cancel.response",
            "task.cancel.error.notFound" | "task.cancel.error.storeUnavailable" => {
                "task.cancel.error"
            }
            "request.start.task.without-attempt" | "request.start.task.with-attempt" => {
                "request.start"
            }
            _ => panic!("unexpected task frame {name}"),
        }
    }

    fn expected_direction(name: &str) -> &'static str {
        match name {
            "task.submit.request.function"
            | "task.submit.request.actorMethod"
            | "task.submit.request.actorMethod.snapshot"
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

    fn expected_decode_as(name: &str) -> &'static str {
        match name {
            "task.submit.request.function"
            | "task.submit.request.actorMethod"
            | "task.submit.request.actorMethod.snapshot"
            | "task.submit.request.legacy-no-caller-kind"
            | "task.submit.request.timing.after"
            | "task.submit.request.timing.at" => "TaskSubmitRequest",
            "task.submit.response" => "TaskSubmitResponse",
            "task.submit.error.parentNotFound"
            | "task.submit.error.invalidTiming"
            | "task.submit.error.payloadInvalid"
            | "task.submit.error.quotaExceeded"
            | "task.submit.error.storeUnavailable"
            | "task.submit.error.rejected" => "TaskSubmitError",
            "task.status.request" => "TaskStatusRequest",
            "task.status.response.scheduled" => "TaskStatusResponse",
            "task.status.error.notFound" | "task.status.error.storeUnavailable" => {
                "TaskStatusError"
            }
            "task.cancel.request" => "TaskCancelRequest",
            "task.cancel.response.canceled" => "TaskCancelResponse",
            "task.cancel.error.notFound" | "task.cancel.error.storeUnavailable" => {
                "TaskCancelError"
            }
            "request.start.task.without-attempt" | "request.start.task.with-attempt" => {
                "BytecodeTaskRequestStart"
            }
            _ => panic!("unexpected task frame {name}"),
        }
    }
}

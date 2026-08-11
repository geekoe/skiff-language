//! M-task consumer gate: the `runtime` crate consumes the frozen
//! C-model-task corpus (`transport/testdata/task-wire/`) through the
//! canonical task codec. The legacy old shape has no compatible reader and
//! every non-legacy frame roundtrips byte-exact.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

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
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, BytecodeRequestStartFrameWireHeader,
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
#[serde(rename_all = "camelCase")]
struct FrameEntry {
    direction: String,
    #[serde(rename = "frameType")]
    frame_type: String,
    #[serde(rename = "decodeAs")]
    decode_as: String,
    #[serde(rename = "payloadPresence")]
    payload_presence: String,
    #[serde(rename = "frameHex")]
    frame_hex: String,
    #[serde(rename = "legacyCut")]
    legacy_cut: bool,
    header: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Catalog {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    corpus: String,
    frames: BTreeMap<String, FrameEntry>,
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("transport/testdata/task-wire")
}

fn catalog() -> Catalog {
    let value = fs::read_to_string(corpus_dir().join("frames.json"))
        .expect("task-wire frames.json must be readable");
    serde_json::from_str(&value).expect("task-wire frames.json must decode")
}

fn hex_bytes(hex: &str) -> Vec<u8> {
    assert!(hex.len() % 2 == 0, "frame hex must have even length");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex must be ASCII");
            u8::from_str_radix(text, 16).expect("frame hex must be valid")
        })
        .collect()
}

fn frame_payload(bytes: &[u8]) -> Vec<u8> {
    let header_len = u32::from_be_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let payload_len = u32::from_be_bytes([bytes[10], bytes[11], bytes[12], bytes[13]]) as usize;
    bytes[14 + header_len..14 + header_len + payload_len].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_consumer_roundtrips_task_wire_corpus_through_canonical_codec() {
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
                entry.payload_presence,
                expected_payload_presence(name),
                "{name}: payloadPresence"
            );
            assert_eq!(
                entry.header["schemaVersion"], "skiff-runtime-frame-v4",
                "{name}: header schemaVersion"
            );
            assert_eq!(
                entry.header["type"], entry.frame_type,
                "{name}: header type"
            );
        }

        for name in [
            "task.submit.request.function",
            "task.submit.request.actorMethod",
            "task.submit.request.actorMethod.snapshot",
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
            assert!(
                !frame_payload(&bytes).is_empty(),
                "{name}: task.submit.request payload must be present"
            );
            let fixture_header: TaskSubmitRequestFrameHeaderV2 =
                serde_json::from_value(entry.header.clone()).expect("fixture header typed");
            assert_eq!(fixture_header, header, "{name} header mismatch");
            assert!(!entry.legacy_cut, "{name} must not be legacy cut");
        }

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
        }
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

        let invalid_kind = serde_json::json!({
            "schemaVersion": "skiff-runtime-frame-v4",
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
    fn runtime_consumer_sees_all_frozen_task_scenarios() {
        let mut names = Vec::new();
        for entry in fs::read_dir(corpus_dir().join("scenarios"))
            .expect("task scenarios dir must be readable")
        {
            let path = entry.expect("scenario entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let value: Value = serde_json::from_str(
                &fs::read_to_string(&path).expect("scenario must be readable"),
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

    fn expected_payload_presence(name: &str) -> &'static str {
        match name {
            "task.submit.request.function"
            | "task.submit.request.actorMethod"
            | "task.submit.request.actorMethod.snapshot"
            | "task.submit.request.legacy-no-caller-kind"
            | "task.submit.request.timing.after"
            | "task.submit.request.timing.at"
            | "request.start.task.without-attempt"
            | "request.start.task.with-attempt" => "required",
            "task.submit.response"
            | "task.submit.error.parentNotFound"
            | "task.submit.error.invalidTiming"
            | "task.submit.error.payloadInvalid"
            | "task.submit.error.quotaExceeded"
            | "task.submit.error.storeUnavailable"
            | "task.submit.error.rejected"
            | "task.status.request"
            | "task.status.response.scheduled"
            | "task.status.error.notFound"
            | "task.status.error.storeUnavailable"
            | "task.cancel.request"
            | "task.cancel.response.canceled"
            | "task.cancel.error.notFound"
            | "task.cancel.error.storeUnavailable" => "empty",
            _ => panic!("unexpected task frame {name}"),
        }
    }
}

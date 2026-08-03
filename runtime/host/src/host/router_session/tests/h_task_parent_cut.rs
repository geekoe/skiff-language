//! H-task-parent-cut driver tests (Runtime crate side).
//!
//! The Runtime driver is the production outbound owner of
//! `task.submit.request`: it must emit the canonical
//! `TaskSubmitRequestFrameHeaderV2` with the closed `callerKind` enum, and
//! the legacy shape (`OutboundControlMessage::TaskSubmit` without
//! `callerKind`) must fail closed with no compatible reader.

use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, TaskCallerKind, TaskSubmitControlMessage,
    TaskSubmitControlRequest,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::{
    decode_task_submit_request_frame, TaskCallerKind as WireTaskCallerKind,
};

use super::*;

fn task_submit_message(
    caller_kind: TaskCallerKind,
    caller_request_id: &str,
) -> TaskSubmitControlMessage {
    TaskSubmitControlMessage {
        request: TaskSubmitControlRequest {
            rpc_id: "rpc:task-1".to_string(),
            runtime_id: "runtime-a".to_string(),
            target_kind: "function".to_string(),
            service_id: "example.com/docs".to_string(),
            service_version: "1.0.0".to_string(),
            service_protocol_identity: "example.com/docs:1.0.0".to_string(),
            target: "example.com/fn".to_string(),
            task_id: Some("task-1".to_string()),
            build_id: None,
            activation_identity: ActivationIdentityControl {
                assembly_identity: AssemblyIdentity::new(
                    "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                generation: 42,
                runtime_replica_id: "runtime-a".to_string(),
                deployment_revision: DeploymentRevision::new("rev-1"),
            },
            caller_request_id: Some(caller_request_id.to_string()),
            trace_id: None,
            caller_target: None,
            max_queue_wait_ms: None,
            actor_method: None,
        },
        payload: vec![1, 2],
        caller_kind,
    }
}

#[test]
fn driver_encodes_function_submit_byte_exact_to_frozen_corpus_frame() {
    let message = task_submit_message(TaskCallerKind::Request, "parent-1");
    let frame =
        crate::host::router_session::task_submit::encode_task_submit_wire_message(message)
            .expect("canonical task submit must encode");
    let catalog: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../runtime/transport/testdata/task-wire/frames.json"
    ))
    .expect("task wire corpus must decode");
    let expected = catalog["frames"]["task.submit.request.function"]["frameHex"]
        .as_str()
        .expect("function frame hex");
    let expected_bytes: Vec<u8> = (0..expected.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&expected[index..index + 2], 16).expect("hex"))
        .collect();
    assert_eq!(
        frame, expected_bytes,
        "driver V2 encode must be byte-identical to the frozen task-wire corpus"
    );
}

#[test]
fn driver_encodes_actor_invocation_submit_with_closed_caller_kind() {
    let message = task_submit_message(TaskCallerKind::ActorInvocation, "parent-1");
    let frame =
        crate::host::router_session::task_submit::encode_task_submit_wire_message(message)
            .expect("canonical task submit must encode");
    let (header, payload) =
        decode_task_submit_request_frame(&frame).expect("canonical task submit must decode");
    assert_eq!(header.caller_kind, WireTaskCallerKind::ActorInvocation);
    assert_eq!(header.caller_request_id, "parent-1");
    assert_eq!(
        header.target_kind,
        skiff_runtime_transport::protocol::TaskTargetKind::Function
    );
    assert_eq!(payload, vec![1, 2]);
    let reencoded =
        skiff_runtime_transport::protocol::encode_task_submit_request_frame(&header, &payload)
            .expect("re-encode");
    assert_eq!(reencoded, frame, "canonical roundtrip must be byte-exact");
}

#[test]
fn driver_rejects_legacy_control_task_submit_with_no_compatible_reader() {
    let message = task_submit_message(TaskCallerKind::Request, "parent-1");
    let legacy =
        RouterWriterMessage::Control(skiff_runtime_request::OutboundControlMessage::TaskSubmit {
            request: message.request,
            payload: message.payload,
        });
    let error = encode_writer_message(legacy)
        .expect_err("legacy task.submit.request without callerKind must fail closed");
    assert!(
        error.to_string().contains("callerKind"),
        "legacy rejection must name callerKind, got {error}"
    );
}

#[test]
fn driver_rejects_task_submit_without_caller_request_id() {
    let mut message = task_submit_message(TaskCallerKind::Request, "parent-1");
    message.request.caller_request_id = None;
    let error =
        crate::host::router_session::task_submit::encode_task_submit_wire_message(message)
            .expect_err("callerRequestId is required on the canonical wire");
    assert!(
        error.to_string().contains("callerRequestId"),
        "rejection must name callerRequestId, got {error}"
    );
}

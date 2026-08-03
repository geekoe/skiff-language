use serde_json::json;

use super::*;
use crate::protocol::{
    encode_binary_frame, ActorTaskRuntimeErrorFrameHeader, TaskSubmitResponseFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

fn activation_identity() -> ActivationIdentityFrameMetadata {
    serde_json::from_value(json!({
        "assemblyIdentity": format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "a".repeat(64)
        ),
        "generation": 42,
        "runtimeReplicaId": "runtime-a",
        "deploymentRevision": "rev-1"
    }))
    .expect("activation identity fixture")
}

fn canonical_request(
    caller_kind: TaskCallerKind,
    target_kind: TaskTargetKind,
) -> TaskSubmitRequestFrameHeaderV2 {
    TaskSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
        rpc_id: "rpc:task-1".to_string(),
        runtime_id: "runtime-a".to_string(),
        caller_kind,
        caller_request_id: "parent-1".to_string(),
        target_kind,
        service_id: "example.com/docs".to_string(),
        service_version: "1.0.0".to_string(),
        service_protocol_identity: "example.com/docs:1.0.0".to_string(),
        target: "example.com/fn".to_string(),
        timing: None,
        task_id: Some("task-1".to_string()),
        build_id: None,
        activation_identity: activation_identity(),
        trace_id: None,
        caller_target: None,
        max_queue_wait_ms: None,
        actor_method: None,
    }
}

#[test]
fn canonical_request_round_trips_byte_exact() {
    let header = canonical_request(TaskCallerKind::Request, TaskTargetKind::Function);
    let bytes = encode_task_submit_request_frame(&header, b"\x01\x02")
        .expect("canonical request must encode");
    let (decoded, payload) =
        decode_task_submit_request_frame(&bytes).expect("canonical request must decode");
    assert_eq!(decoded, header);
    assert_eq!(payload, b"\x01\x02");
    assert_eq!(
        encode_task_submit_request_frame(&decoded, &payload).expect("re-encode"),
        bytes,
        "canonical request must be byte-exact"
    );
}

#[test]
fn legacy_old_shape_without_caller_kind_is_rejected() {
    let legacy = json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": TASK_SUBMIT_REQUEST_FRAME_TYPE,
        "rpcId": "rpc:task-legacy-1",
        "runtimeId": "runtime-a",
        "callerRequestId": "parent-1",
        "targetKind": "function",
        "serviceId": "example.com/docs",
        "serviceVersion": "1.0.0",
        "serviceProtocolIdentity": "example.com/docs:1.0.0",
        "target": "example.com/fn",
        "taskId": "task-legacy-1",
        "activationIdentity": activation_identity_json(),
    });
    let bytes = encode_binary_frame(&legacy, &[1, 2, 3]).expect("legacy frame must encode");
    let error = decode_task_submit_request_frame(&bytes)
        .expect_err("legacy old-shape frame must be rejected with no compatible reader");
    assert!(
        error.to_string().contains("callerKind"),
        "legacy rejection must name callerKind, got {error}"
    );
}

#[test]
fn caller_kind_is_a_closed_enum() {
    let invalid = json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": TASK_SUBMIT_REQUEST_FRAME_TYPE,
        "rpcId": "rpc:task-1",
        "runtimeId": "runtime-a",
        "callerKind": "function",
        "callerRequestId": "parent-1",
        "targetKind": "function",
        "serviceId": "example.com/docs",
        "serviceVersion": "1.0.0",
        "serviceProtocolIdentity": "example.com/docs:1.0.0",
        "target": "example.com/fn",
        "activationIdentity": activation_identity_json(),
    });
    let error = serde_json::from_value::<TaskSubmitRequestFrameHeaderV2>(invalid)
        .expect_err("callerKind=function must be rejected by the closed enum");
    assert!(
        error.to_string().contains("unknown variant"),
        "closed enum rejection must reject the unknown variant, got {error}"
    );
}

#[test]
fn target_kind_and_actor_method_must_agree() {
    let mut function_with_actor_method =
        canonical_request(TaskCallerKind::ActorInvocation, TaskTargetKind::Function);
    function_with_actor_method.actor_method = Some(actor_method_metadata());
    let error = encode_task_submit_request_frame(&function_with_actor_method, &[])
        .expect_err("function target must not carry actorMethod");
    assert!(error.to_string().contains("actorMethod"));

    let mut actor_method_without_metadata = canonical_request(
        TaskCallerKind::ActorInvocation,
        TaskTargetKind::ActorMethod,
    );
    actor_method_without_metadata.actor_method = None;
    let error = encode_task_submit_request_frame(&actor_method_without_metadata, &[])
        .expect_err("actorMethod target requires actorMethod metadata");
    assert!(error.to_string().contains("actorMethod"));
}

#[test]
fn response_and_error_frames_enforce_empty_payload() {
    let response = TaskSubmitResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:task-1".to_string(),
        task_ref: TaskRef::new("task-1", "example.com/docs").expect("task ref"),
        task_id: "task-1".to_string(),
        request_id: "req:task-1".to_string(),
        status: TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
    };
    let clean = encode_task_submit_response_frame(&response).expect("response must encode");
    assert_eq!(
        decode_task_submit_response_frame(&clean).expect("response must decode"),
        response
    );
    let with_payload =
        encode_binary_frame(&response, b"intruder").expect("raw response must encode");
    let error = decode_task_submit_response_frame(&with_payload)
        .expect_err("response payload must be empty");
    assert!(error.to_string().contains("payload must be empty"));

    let error_header = ActorTaskRuntimeErrorFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_SUBMIT_ERROR_FRAME_TYPE.to_string(),
        rpc_id: "rpc:task-1".to_string(),
        error: crate::protocol::RuntimeErrorFramePayload {
            code: "ParentNotFound".to_string(),
            message: "dispatch callerRequestId does not identify an active parent".to_string(),
            status: Some(404),
            details: None,
        },
    };
    let clean = encode_task_submit_error_frame(&error_header).expect("error must encode");
    assert_eq!(
        decode_task_submit_error_frame(&clean).expect("error must decode"),
        error_header
    );
    let with_payload =
        encode_binary_frame(&error_header, b"intruder").expect("raw error must encode");
    let error =
        decode_task_submit_error_frame(&with_payload).expect_err("error payload must be empty");
    assert!(error.to_string().contains("payload must be empty"));
}

#[test]
fn task_frame_direction_table_is_frozen_per_frame() {
    use crate::protocol::FrameDirection;
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
        task_submit_frame_direction("task.submit.unknown"),
        None,
        "unknown task frame types have no direction"
    );
}

#[test]
fn task_submit_response_projection_round_trips() {
    let mut header = canonical_request(
        TaskCallerKind::ActorInvocation,
        TaskTargetKind::ActorMethod,
    );
    header.actor_method = Some(actor_method_metadata());
    let reconstructed =
        encode_task_submit_request_frame(&header, b"\x01\x02").expect("request must re-encode");
    assert_eq!(
        decode_task_submit_request_frame(&reconstructed).expect("must decode"),
        (header, b"\x01\x02".to_vec())
    );

    let response = TaskSubmitResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:task-1".to_string(),
        task_ref: TaskRef::new("task-1", "svc-1").expect("taskRef"),
        task_id: "task-1".to_string(),
        request_id: "task-1".to_string(),
        status: TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
    };
    assert_eq!(response.rpc_id, "rpc:task-1");
    assert_eq!(response.task_id, "task-1");
    assert_eq!(response.request_id, "task-1");
    assert_eq!(response.status, TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED);
    let response_bytes = encode_task_submit_response_frame(&response)
        .expect("response projection must encode");
    assert_eq!(
        decode_task_submit_response_frame(&response_bytes).expect("must decode"),
        response
    );
}

fn activation_identity_json() -> serde_json::Value {
    json!({
        "assemblyIdentity": format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "a".repeat(64)
        ),
        "generation": 42,
        "runtimeReplicaId": "runtime-a",
        "deploymentRevision": "rev-1"
    })
}

fn actor_method_metadata() -> TaskActorMethodTargetFrameMetadata {
    serde_json::from_value(json!({
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
        "methodIdentity": format!("skiff-actor-method-v1:sha256:{}", "c".repeat(64))
    }))
    .expect("actor method metadata fixture")
}

#[test]
fn submit_timing_three_kinds_round_trip_and_missing_defaults_to_immediate() {
    let immediate = canonical_request(TaskCallerKind::Request, TaskTargetKind::Function);
    assert_eq!(immediate.timing, None, "construction default is immediate");
    let bytes = encode_task_submit_request_frame(&immediate, b"\x01\x02")
        .expect("immediate request must encode");
    let (decoded, payload) =
        decode_task_submit_request_frame(&bytes).expect("immediate request must decode");
    assert_eq!(decoded.timing, None);
    assert_eq!(payload, b"\x01\x02");
    let header_json: serde_json::Value = serde_json::from_slice(
        &crate::protocol::decode_binary_frame(&bytes)
            .expect("frame")
            .header
            .to_string()
            .into_bytes(),
    )
    .expect("header json");
    assert!(
        header_json.get("timing").is_none(),
        "immediate timing must be omitted from the canonical wire"
    );

    let after = TaskSubmitTiming::After { duration_ms: 5_000 };
    let mut header = canonical_request(TaskCallerKind::Request, TaskTargetKind::Function);
    header.timing = Some(after);
    let bytes = encode_task_submit_request_frame(&header, b"\x01\x02").expect("after encode");
    let (decoded, _) =
        decode_task_submit_request_frame(&bytes).expect("after request must decode");
    assert_eq!(decoded.timing, Some(after));

    let at = TaskSubmitTiming::At {
        utc_millis: 1_700_000_000_000,
    };
    header.timing = Some(at);
    let bytes = encode_task_submit_request_frame(&header, b"\x01\x02").expect("at encode");
    let (decoded, _) = decode_task_submit_request_frame(&bytes).expect("at request must decode");
    assert_eq!(decoded.timing, Some(at));

    let invalid = serde_json::json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": TASK_SUBMIT_REQUEST_FRAME_TYPE,
        "rpcId": "rpc:task-1",
        "runtimeId": "runtime-a",
        "callerKind": "request",
        "callerRequestId": "parent-1",
        "targetKind": "function",
        "serviceId": "example.com/docs",
        "serviceVersion": "1.0.0",
        "serviceProtocolIdentity": "example.com/docs:1.0.0",
        "target": "example.com/fn",
        "timing": { "kind": "someday" },
        "activationIdentity": activation_identity_json(),
    });
    let bytes = encode_binary_frame(&invalid, b"").expect("invalid timing frame must encode");
    let error = decode_task_submit_request_frame(&bytes)
        .expect_err("unknown timing kind must be rejected by the closed enum");
    assert!(
        error.to_string().contains("unknown variant"),
        "closed timing enum must reject the unknown kind, got {error}"
    );
}

#[test]
fn task_ref_round_trips_and_malformed_wire_values_are_rejected() {
    let task_ref = TaskRef::new("task-1", "example.com/docs").expect("task ref");
    assert_eq!(task_ref.task_id(), "task-1");
    assert_eq!(task_ref.owner(), "example.com/docs");
    assert_eq!(
        task_ref.as_str(),
        "skiff-task-v1:ZXhhbXBsZS5jb20vZG9jcw.dGFzay0x"
    );
    let parsed = TaskRef::parse(task_ref.as_str()).expect("canonical task ref parses");
    assert_eq!(parsed, task_ref);
    assert_eq!(
        serde_json::from_value::<TaskRef>(serde_json::json!(task_ref.as_str()))
            .expect("wire decode"),
        task_ref
    );

    for raw in [
        "task-v1:abc.def",
        "skiff-task-v1:abc",
        "skiff-task-v1:.dGFzay0x",
        "skiff-task-v1:ZXhhbXBsZS5jb20vZG9jcw.",
        "skiff-task-v1:!!!.dGFzay0x",
        "skiff-task-v1:ZXhhbXBsZS5jb20vZG9jcw==.dGFzay0x",
    ] {
        assert!(
            TaskRef::parse(raw).is_err(),
            "malformed taskRef {raw:?} must be a wire error"
        );
    }
    assert!(TaskRef::new("", "example.com/docs").is_err());
    assert!(TaskRef::new("task-1", "").is_err());
}

#[test]
fn status_and_cancel_kind_spellings_match_reference() {
    let status_kinds = [
        (TaskStatusKindWire::Scheduled, "scheduled"),
        (TaskStatusKindWire::Ready, "ready"),
        (TaskStatusKindWire::Running, "running"),
        (TaskStatusKindWire::Succeeded, "succeeded"),
        (TaskStatusKindWire::Failed, "failed"),
        (TaskStatusKindWire::PlatformFailed, "platformFailed"),
        (TaskStatusKindWire::Canceled, "canceled"),
        (TaskStatusKindWire::Expired, "expired"),
    ];
    for (kind, expected) in status_kinds {
        assert_eq!(kind.as_str(), expected, "TaskStatus kind spelling");
        let wire = serde_json::to_value(TaskStatusWire { kind }).expect("serialize");
        assert_eq!(wire, serde_json::json!({ "kind": expected }));
    }
    let cancel_kinds = [
        (TaskCancelResultKindWire::Canceled, "canceled"),
        (TaskCancelResultKindWire::AlreadyStarted, "alreadyStarted"),
        (TaskCancelResultKindWire::AlreadyTerminal, "alreadyTerminal"),
        (TaskCancelResultKindWire::Expired, "expired"),
    ];
    for (kind, expected) in cancel_kinds {
        assert_eq!(kind.as_str(), expected, "TaskCancelResult kind spelling");
        let wire = serde_json::to_value(TaskCancelResultWire { kind }).expect("serialize");
        assert_eq!(wire, serde_json::json!({ "kind": expected }));
    }
}

#[test]
fn rejection_code_projection_and_transient_classification() {
    for (code, expected) in [
        (TaskSubmitRejectionCode::InvalidTiming, "invalidTiming"),
        (TaskSubmitRejectionCode::PayloadInvalid, "payloadInvalid"),
        (TaskSubmitRejectionCode::QuotaExceeded, "quotaExceeded"),
        (TaskSubmitRejectionCode::StoreUnavailable, "storeUnavailable"),
        (TaskSubmitRejectionCode::Rejected, "rejected"),
        (TaskSubmitRejectionCode::UnsupportedTarget, "unsupportedTarget"),
    ] {
        assert_eq!(code.as_str(), expected);
        assert_eq!(TaskSubmitRejectionCode::parse(expected), Some(code));
        assert!(!code.is_definite() || !code.is_transient());
    }
    assert_eq!(TaskSubmitRejectionCode::parse("ParentNotFound"), None);
    assert!(TaskSubmitRejectionCode::StoreUnavailable.is_transient());
    assert!(!TaskSubmitRejectionCode::StoreUnavailable.is_definite());
    for definite in [
        TaskSubmitRejectionCode::InvalidTiming,
        TaskSubmitRejectionCode::PayloadInvalid,
        TaskSubmitRejectionCode::QuotaExceeded,
        TaskSubmitRejectionCode::Rejected,
        TaskSubmitRejectionCode::UnsupportedTarget,
    ] {
        assert!(definite.is_definite());
        assert!(!definite.is_transient());
    }
}

#[test]
fn status_and_cancel_frames_round_trip_and_enforce_empty_payload() {
    let task_ref = TaskRef::new("task-1", "example.com/docs").expect("task ref");
    let status_request = TaskStatusRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_STATUS_REQUEST_FRAME_TYPE.to_string(),
        rpc_id: "rpc:status-1".to_string(),
        task_ref: task_ref.clone(),
    };
    let bytes = encode_task_status_request_frame(&status_request).expect("encode");
    assert_eq!(
        decode_task_status_request_frame(&bytes).expect("decode"),
        status_request
    );

    let status_response = TaskStatusResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_STATUS_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:status-1".to_string(),
        task_ref: task_ref.clone(),
        status: TaskStatusWire {
            kind: TaskStatusKindWire::Running,
        },
    };
    let bytes = encode_task_status_response_frame(&status_response).expect("encode");
    assert_eq!(
        decode_task_status_response_frame(&bytes).expect("decode"),
        status_response
    );

    let cancel_request = TaskCancelRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_CANCEL_REQUEST_FRAME_TYPE.to_string(),
        rpc_id: "rpc:cancel-1".to_string(),
        task_ref: task_ref.clone(),
    };
    let bytes = encode_task_cancel_request_frame(&cancel_request).expect("encode");
    assert_eq!(
        decode_task_cancel_request_frame(&bytes).expect("decode"),
        cancel_request
    );

    let cancel_response = TaskCancelResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_CANCEL_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:cancel-1".to_string(),
        task_ref,
        result: TaskCancelResultWire {
            kind: TaskCancelResultKindWire::AlreadyStarted,
        },
    };
    let bytes = encode_task_cancel_response_frame(&cancel_response).expect("encode");
    assert_eq!(
        decode_task_cancel_response_frame(&bytes).expect("decode"),
        cancel_response
    );

    let with_payload =
        encode_binary_frame(&status_request, b"intruder").expect("raw frame must encode");
    let error = decode_task_status_request_frame(&with_payload)
        .expect_err("task.status.request payload must be empty");
    assert!(error.to_string().contains("payload must be empty"));
}

#[test]
fn submit_response_task_ref_is_a_wire_error_when_undecodable() {
    let mut response = TaskSubmitResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: TASK_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:task-1".to_string(),
        task_ref: TaskRef::new("task-1", "example.com/docs").expect("task ref"),
        task_id: "task-1".to_string(),
        request_id: "req:task-1".to_string(),
        status: TASK_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
    };
    let bytes = encode_task_submit_response_frame(&response).expect("response must encode");
    assert_eq!(
        decode_task_submit_response_frame(&bytes).expect("decode"),
        response
    );

    let mut header_json: serde_json::Value = serde_json::from_slice(
        &crate::protocol::decode_binary_frame(&bytes)
            .expect("frame")
            .header
            .to_string()
            .into_bytes(),
    )
    .expect("header json");
    header_json["taskRef"] = serde_json::json!("not-a-task-ref");
    let malformed = encode_binary_frame(&header_json, &[]).expect("malformed frame must encode");
    let error = decode_task_submit_response_frame(&malformed)
        .expect_err("undecodable taskRef must be a wire error");
    assert!(
        error.to_string().contains("taskRef"),
        "wire error must name taskRef, got {error}"
    );

    response.task_ref = TaskRef::new("task-2", "example.com/docs").expect("task ref");
    assert_ne!(
        encode_task_submit_response_frame(&response).expect("re-encode"),
        bytes,
        "taskRef must be part of the canonical response bytes"
    );
}

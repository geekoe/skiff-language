use serde_json::json;

use super::*;
use crate::protocol::{
    encode_binary_frame, ActorSpawnRuntimeErrorFrameHeader, SpawnSubmitResponseFrameHeader,
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
    caller_kind: SpawnCallerKind,
    target_kind: SpawnTargetKind,
) -> SpawnSubmitRequestFrameHeaderV2 {
    SpawnSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
        rpc_id: "rpc:spawn-1".to_string(),
        runtime_id: "runtime-a".to_string(),
        caller_kind,
        caller_request_id: "parent-1".to_string(),
        target_kind,
        service_id: "example.com/docs".to_string(),
        service_version: "1.0.0".to_string(),
        service_protocol_identity: "example.com/docs:1.0.0".to_string(),
        target: "example.com/fn".to_string(),
        spawn_id: Some("spawn-1".to_string()),
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
    let header = canonical_request(SpawnCallerKind::Request, SpawnTargetKind::Function);
    let bytes = encode_spawn_submit_request_frame(&header, b"\x01\x02")
        .expect("canonical request must encode");
    let (decoded, payload) =
        decode_spawn_submit_request_frame(&bytes).expect("canonical request must decode");
    assert_eq!(decoded, header);
    assert_eq!(payload, b"\x01\x02");
    assert_eq!(
        encode_spawn_submit_request_frame(&decoded, &payload).expect("re-encode"),
        bytes,
        "canonical request must be byte-exact"
    );
}

#[test]
fn legacy_old_shape_without_caller_kind_is_rejected() {
    let legacy = json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": SPAWN_SUBMIT_REQUEST_FRAME_TYPE,
        "rpcId": "rpc:spawn-legacy-1",
        "runtimeId": "runtime-a",
        "callerRequestId": "parent-1",
        "targetKind": "function",
        "serviceId": "example.com/docs",
        "serviceVersion": "1.0.0",
        "serviceProtocolIdentity": "example.com/docs:1.0.0",
        "target": "example.com/fn",
        "spawnId": "spawn-legacy-1",
        "activationIdentity": activation_identity_json(),
    });
    let bytes = encode_binary_frame(&legacy, &[1, 2, 3]).expect("legacy frame must encode");
    let error = decode_spawn_submit_request_frame(&bytes)
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
        "type": SPAWN_SUBMIT_REQUEST_FRAME_TYPE,
        "rpcId": "rpc:spawn-1",
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
    let error = serde_json::from_value::<SpawnSubmitRequestFrameHeaderV2>(invalid)
        .expect_err("callerKind=function must be rejected by the closed enum");
    assert!(
        error.to_string().contains("unknown variant"),
        "closed enum rejection must reject the unknown variant, got {error}"
    );
}

#[test]
fn target_kind_and_actor_method_must_agree() {
    let mut function_with_actor_method =
        canonical_request(SpawnCallerKind::ActorInvocation, SpawnTargetKind::Function);
    function_with_actor_method.actor_method = Some(actor_method_metadata());
    let error = encode_spawn_submit_request_frame(&function_with_actor_method, &[])
        .expect_err("function target must not carry actorMethod");
    assert!(error.to_string().contains("actorMethod"));

    let mut actor_method_without_metadata = canonical_request(
        SpawnCallerKind::ActorInvocation,
        SpawnTargetKind::ActorMethod,
    );
    actor_method_without_metadata.actor_method = None;
    let error = encode_spawn_submit_request_frame(&actor_method_without_metadata, &[])
        .expect_err("actorMethod target requires actorMethod metadata");
    assert!(error.to_string().contains("actorMethod"));
}

#[test]
fn response_and_error_frames_enforce_empty_payload() {
    let response = SpawnSubmitResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: SPAWN_SUBMIT_RESPONSE_FRAME_TYPE.to_string(),
        rpc_id: "rpc:spawn-1".to_string(),
        spawn_id: "spawn-1".to_string(),
        request_id: "req:spawned-1".to_string(),
        status: SPAWN_SUBMIT_RESPONSE_STATUS_SUBMITTED.to_string(),
    };
    let clean = encode_spawn_submit_response_frame(&response).expect("response must encode");
    assert_eq!(
        decode_spawn_submit_response_frame(&clean).expect("response must decode"),
        response
    );
    let with_payload =
        encode_binary_frame(&response, b"intruder").expect("raw response must encode");
    let error = decode_spawn_submit_response_frame(&with_payload)
        .expect_err("response payload must be empty");
    assert!(error.to_string().contains("payload must be empty"));

    let error_header = ActorSpawnRuntimeErrorFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: SPAWN_SUBMIT_ERROR_FRAME_TYPE.to_string(),
        rpc_id: "rpc:spawn-1".to_string(),
        error: crate::protocol::RuntimeErrorFramePayload {
            code: "ParentNotFound".to_string(),
            message: "spawn callerRequestId does not identify an active parent".to_string(),
            status: Some(404),
            details: None,
        },
    };
    let clean = encode_spawn_submit_error_frame(&error_header).expect("error must encode");
    assert_eq!(
        decode_spawn_submit_error_frame(&clean).expect("error must decode"),
        error_header
    );
    let with_payload =
        encode_binary_frame(&error_header, b"intruder").expect("raw error must encode");
    let error =
        decode_spawn_submit_error_frame(&with_payload).expect_err("error payload must be empty");
    assert!(error.to_string().contains("payload must be empty"));
}

#[test]
fn spawn_frame_direction_table_is_frozen_per_frame() {
    use crate::protocol::FrameDirection;
    assert_eq!(
        spawn_submit_frame_direction(SPAWN_SUBMIT_REQUEST_FRAME_TYPE),
        Some(FrameDirection::RuntimeToRouter)
    );
    assert_eq!(
        spawn_submit_frame_direction(SPAWN_SUBMIT_RESPONSE_FRAME_TYPE),
        Some(FrameDirection::RouterToRuntime)
    );
    assert_eq!(
        spawn_submit_frame_direction(SPAWN_SUBMIT_ERROR_FRAME_TYPE),
        Some(FrameDirection::RouterToRuntime)
    );
    assert_eq!(
        spawn_submit_frame_direction("spawn.submit.unknown"),
        None,
        "unknown spawn frame types have no direction"
    );
}

#[test]
fn spawn_submit_acceptance_carries_raw_wire_request_and_response_projection() {
    let mut header = canonical_request(
        SpawnCallerKind::ActorInvocation,
        SpawnTargetKind::ActorMethod,
    );
    header.actor_method = Some(actor_method_metadata());
    let request = SpawnSubmitRequestFrame {
        header: header.clone(),
        payload: b"\x01\x02".to_vec(),
    };
    let acceptance = SpawnSubmitAcceptance {
        request,
        spawn_id: "spawn-1".to_string(),
        request_id: "req:spawned-1".to_string(),
    };

    // The acceptance boundary preserves the raw wire header + args bytes so
    // the execution sink can reconstruct the outbound request without
    // re-parsing (C-model-spawn §7.2).
    let reconstructed =
        encode_spawn_submit_request_frame(&acceptance.request.header, &acceptance.request.payload)
            .expect("accepted request must re-encode");
    assert_eq!(
        decode_spawn_submit_request_frame(&reconstructed).expect("must decode"),
        (header, b"\x01\x02".to_vec())
    );

    let response = acceptance.response_header();
    assert_eq!(response.rpc_id, "rpc:spawn-1");
    assert_eq!(response.spawn_id, "spawn-1");
    assert_eq!(response.request_id, "req:spawned-1");
    assert_eq!(response.status, SPAWN_SUBMIT_RESPONSE_STATUS_SUBMITTED);
    let response_bytes = encode_spawn_submit_response_frame(&response)
        .expect("acceptance response projection must encode");
    assert_eq!(
        decode_spawn_submit_response_frame(&response_bytes).expect("must decode"),
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

fn actor_method_metadata() -> SpawnActorMethodTargetFrameMetadata {
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

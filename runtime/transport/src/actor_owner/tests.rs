use super::*;
use crate::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorMethodDeadlineFrameHeader, ActorOwnerFileFrameHeader,
    ActorOwnerUnitFrameHeader,
};

fn identity(prefix: &str, byte: char) -> String {
    format!("{prefix}:{}", byte.to_string().repeat(64))
}

#[test]
fn owner_failure_round_trips_as_a_non_actor_domain_terminal() {
    let header = ActorOwnerFailureFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_FAILURE_FRAME_TYPE.into(),
        invocation_id: "invoke-1".into(),
        owner_runtime_id: "runtime-1".into(),
        owner_lease_id: "lease-1".into(),
        epoch: 1,
        actor_implementation_identity: ActorImplementationIdentity::new(identity(
            "skiff-actor-implementation-v1:sha256",
            'a',
        )),
        reason: ActorOwnerFailureReasonFrameHeader {
            code: "runtimeExecutionFailed".into(),
            message: "boom".into(),
        },
    };
    let wire = encode_actor_owner_failure_frame(&header).unwrap();
    assert_eq!(decode_actor_owner_failure_frame(&wire).unwrap(), header);
}

#[test]
fn owner_failure_rejects_empty_or_oversized_messages() {
    let mut header = ActorOwnerFailureFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_FAILURE_FRAME_TYPE.into(),
        invocation_id: "invoke-1".into(),
        owner_runtime_id: "runtime-1".into(),
        owner_lease_id: "lease-1".into(),
        epoch: 1,
        actor_implementation_identity: ActorImplementationIdentity::new(identity(
            "skiff-actor-implementation-v1:sha256",
            'a',
        )),
        reason: ActorOwnerFailureReasonFrameHeader {
            code: "runtimeExecutionFailed".into(),
            message: String::new(),
        },
    };
    assert!(encode_actor_owner_failure_frame(&header).is_err());
    header.reason.message = "x".repeat(4097);
    assert!(encode_actor_owner_failure_frame(&header).is_err());
}

fn declaration_owner() -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: ActorOwnerUnitFrameHeader::Service,
        file: ActorOwnerFileFrameHeader::FileIrIdentity("file:actor-1".to_string()),
        actor_symbol: "Counter".to_string(),
    }
}

fn logical_key() -> ActorOwnerLogicalKeyFrameHeader {
    ActorOwnerLogicalKeyFrameHeader {
        service_id: "example.com/actor".to_string(),
        actor_type_identity: "actor.example.Counter".to_string(),
        actor_id_type_identity: "type.example.CounterId".to_string(),
        actor_id_encoding_version: "json-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "AQ==".to_string(),
        actor_id_hash: format!("sha256:{}", "d".repeat(64)),
    }
}

fn fence() -> ActorOwnerControlFenceFrameHeader {
    let key = logical_key();
    ActorOwnerControlFenceFrameHeader {
        service_id: key.service_id,
        actor_type_identity: key.actor_type_identity,
        actor_id_type_identity: key.actor_id_type_identity,
        actor_id_encoding_version: key.actor_id_encoding_version,
        canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64,
        actor_id_hash: key.actor_id_hash,
        epoch: 1,
        actor_abi_identity: ActorAbiIdentity::new(identity("skiff-actor-abi-v1:sha256", 'a')),
        actor_implementation_identity: ActorImplementationIdentity::new(identity(
            "skiff-actor-implementation-v1:sha256",
            'b',
        )),
        declaration_owner: declaration_owner(),
        owner_lease_id: "lease-1".to_string(),
        eviction_request_id: None,
    }
}

fn route_authority() -> ActorOwnerRouteAuthorityFrameHeader {
    ActorOwnerRouteAuthorityFrameHeader {
        assembly_identity: format!("skiff-runtime-assembly-v3:sha256:{}", "c".repeat(64)),
        assembly_generation: 3,
    }
}

#[test]
fn activate_initial_control_round_trips_with_bootstrap_and_deadline() {
    let header = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.into(),
        target_runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        fence: fence(),
        route_authority: route_authority(),
        transition: None,
        bootstrap: Some(ActorActivationBootstrapFrameHeader {
            encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            payload_base64: base64::engine::general_purpose::STANDARD.encode([1u8, 2, 3]),
        }),
        deadline: Some(ActorMethodDeadlineFrameHeader {
            timeout_ms: 30_000,
            expires_at: "2099-01-01T00:00:00.000Z".to_string(),
        }),
        test_case_capability: Some("test-case:create_1".to_string()),
        test_case_parent_request_id: Some("request:parent_1".to_string()),
    };

    let wire = encode_actor_owner_control_frame(&header).unwrap();
    let decoded_wire = crate::protocol::decode_binary_frame(&wire).unwrap();
    assert_eq!(
        decoded_wire.header.get("testCaseCapability"),
        Some(&serde_json::json!("test-case:create_1"))
    );
    assert_eq!(
        decoded_wire.header.get("testCaseParentRequestId"),
        Some(&serde_json::json!("request:parent_1"))
    );
    assert!(decoded_wire.header.get("test_case_capability").is_none());
    assert!(decoded_wire
        .header
        .get("test_case_parent_request_id")
        .is_none());
    assert_eq!(decode_actor_owner_control_frame(&wire).unwrap(), header);
}

#[test]
fn activate_initial_control_rejects_missing_bootstrap_or_deadline() {
    let mut header = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.into(),
        target_runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        fence: fence(),
        route_authority: route_authority(),
        transition: None,
        bootstrap: None,
        deadline: None,
        test_case_capability: None,
        test_case_parent_request_id: None,
    };
    assert!(encode_actor_owner_control_frame(&header).is_err());
    header.bootstrap = Some(ActorActivationBootstrapFrameHeader {
        encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
        payload_base64: base64::engine::general_purpose::STANDARD.encode([1u8]),
    });
    assert!(encode_actor_owner_control_frame(&header).is_err());
    header.deadline = Some(ActorMethodDeadlineFrameHeader {
        timeout_ms: 30_000,
        expires_at: "2099-01-01T00:00:00.000Z".to_string(),
    });
    let wire = encode_actor_owner_control_frame(&header).unwrap();
    let decoded_wire = crate::protocol::decode_binary_frame(&wire).unwrap();
    assert!(decoded_wire.header.get("testCaseCapability").is_none());
    assert!(decoded_wire.header.get("testCaseParentRequestId").is_none());
}

#[test]
fn activate_initial_control_requires_test_case_authority_pair() {
    let mut header = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.into(),
        target_runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        fence: fence(),
        route_authority: route_authority(),
        transition: None,
        bootstrap: Some(ActorActivationBootstrapFrameHeader {
            encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            payload_base64: base64::engine::general_purpose::STANDARD.encode([1u8]),
        }),
        deadline: Some(ActorMethodDeadlineFrameHeader {
            timeout_ms: 30_000,
            expires_at: "2099-01-01T00:00:00.000Z".to_string(),
        }),
        test_case_capability: Some("test-case:create_1".to_string()),
        test_case_parent_request_id: None,
    };
    assert!(encode_actor_owner_control_frame(&header).is_err());

    header.test_case_capability = None;
    header.test_case_parent_request_id = Some("request:parent_1".to_string());
    assert!(encode_actor_owner_control_frame(&header).is_err());
}

#[test]
fn activate_initial_control_rejects_invalid_test_case_authority_tokens() {
    let mut header = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.into(),
        target_runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        fence: fence(),
        route_authority: route_authority(),
        transition: None,
        bootstrap: Some(ActorActivationBootstrapFrameHeader {
            encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
            payload_base64: base64::engine::general_purpose::STANDARD.encode([1u8]),
        }),
        deadline: Some(ActorMethodDeadlineFrameHeader {
            timeout_ms: 30_000,
            expires_at: "2099-01-01T00:00:00.000Z".to_string(),
        }),
        test_case_capability: Some("not canonical".to_string()),
        test_case_parent_request_id: Some("request:parent_1".to_string()),
    };
    assert!(encode_actor_owner_control_frame(&header).is_err());

    header.test_case_capability = Some("test-case:create_1".to_string());
    header.test_case_parent_request_id = Some("not canonical".to_string());
    assert!(encode_actor_owner_control_frame(&header).is_err());
}

#[test]
fn non_initial_control_rejects_test_case_authority() {
    let header = ActorOwnerControlFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_FRAME_TYPE.into(),
        target_runtime_id: "runtime-1".to_string(),
        request_id: "actor-control-1".to_string(),
        operation: ActorOwnerControlOperation::MarkUpgrading,
        fence: fence(),
        route_authority: route_authority(),
        transition: None,
        bootstrap: None,
        deadline: None,
        test_case_capability: Some("test-case:create_1".to_string()),
        test_case_parent_request_id: Some("request:parent_1".to_string()),
    };
    assert!(encode_actor_owner_control_frame(&header).is_err());
}

#[test]
fn control_ack_round_trips_with_failure_reason() {
    let header = ActorOwnerControlAckFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE.into(),
        runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        accepted: false,
        reason: Some(ActorOwnerFailureReasonFrameHeader {
            code: "ActorCreateFailed".to_string(),
            message: "create boom".to_string(),
        }),
    };
    let wire = encode_actor_owner_control_ack_frame(&header).unwrap();
    let (decoded, payload): (ActorOwnerControlAckFrameHeader, Vec<u8>) =
        crate::protocol::decode_typed_binary_frame(&wire).expect("ack header decodes");
    assert!(payload.is_empty());
    assert_eq!(decoded, header);
}

#[test]
fn control_ack_rejects_reason_when_accepted() {
    let header = ActorOwnerControlAckFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
        envelope_type: ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE.into(),
        runtime_id: "runtime-1".to_string(),
        request_id: "actor-bootstrap-1".to_string(),
        operation: ActorOwnerControlOperation::ActivateInitial,
        accepted: true,
        reason: Some(ActorOwnerFailureReasonFrameHeader {
            code: "ActorCreateFailed".to_string(),
            message: "boom".to_string(),
        }),
    };
    assert!(encode_actor_owner_control_ack_frame(&header).is_err());
}

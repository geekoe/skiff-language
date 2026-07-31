use super::*;

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

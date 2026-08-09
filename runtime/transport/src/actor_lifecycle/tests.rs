use serde_json::json;
use skiff_artifact_identity::{
    ACTOR_ABI_IDENTITY_PREFIX, ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, DeploymentArtifactIdentity, DeploymentRevision,
    ServiceDeploymentRef,
};

use super::validation::{JAVASCRIPT_MAX_SAFE_INTEGER, SHA256_PREFIX};
use super::*;
use crate::{
    actor_owner::ActorOwnerLogicalKeyFrameHeader,
    protocol::{encode_binary_frame, TaskActorActivationSnapshotFrameMetadata},
};

fn sha256_identity(prefix: &str, byte: char) -> String {
    format!("{prefix}:{}", byte.to_string().repeat(64))
}

fn deployment(build_byte: char) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/counter".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision-7"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(sha256_identity(
            DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
            build_byte,
        )),
    }
}

fn deployment_owner(build_byte: char) -> ExactDeploymentOwnerFrameMetadata {
    ExactDeploymentOwnerFrameMetadata::new(deployment(build_byte)).unwrap()
}

fn execution(build_byte: char) -> ExactActorExecutionIdentityFrameMetadata {
    ExactActorExecutionIdentityFrameMetadata::new(
        deployment_owner(build_byte),
        ActorAbiIdentity::new(sha256_identity(ACTOR_ABI_IDENTITY_PREFIX, 'a')),
        ActorImplementationIdentity::new(sha256_identity(
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            'b',
        )),
    )
    .unwrap()
}

fn actor() -> ActorOwnerLogicalKeyFrameHeader {
    ActorOwnerLogicalKeyFrameHeader {
        service_id: "example.com/counter".to_string(),
        actor_type_identity: "example.com/counter.Counter".to_string(),
        actor_id_type_identity: "example.com/counter.CounterId".to_string(),
        actor_id_encoding_version: "skiff-canonical-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "AQ==".to_string(),
        actor_id_hash: sha256_identity(SHA256_PREFIX, 'c'),
    }
}

fn fence(build_byte: char) -> ExactActorOwnerFenceFrameMetadata {
    ExactActorOwnerFenceFrameMetadata::new(
        actor(),
        execution(build_byte),
        ActorIncarnation::new(7).unwrap(),
        ActorArenaEpoch::new(3).unwrap(),
        "runtime-1",
        "owner-lease-9",
    )
    .unwrap()
}

fn activation() -> TaskActorActivationSnapshotFrameMetadata {
    TaskActorActivationSnapshotFrameMetadata {
        key: "AQ==".to_string(),
        create_input: "W10=".to_string(),
        expected_type_plan: json!({
            "label": "create",
            "node": { "kind": "record", "fields": [] }
        }),
    }
}

#[test]
fn exact_deployment_owner_is_the_typed_full_coordinate() {
    let owner = deployment_owner('d');
    assert_eq!(
        owner.build_id().as_str(),
        sha256_identity(DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, 'd')
    );

    let json = serde_json::to_value(&owner).unwrap();
    assert_eq!(json["serviceId"], "example.com/counter");
    assert_eq!(json["contractVersion"], "1.0.0");
    assert_eq!(json["deploymentRevision"], "revision-7");
    assert_eq!(
        serde_json::from_value::<ExactDeploymentOwnerFrameMetadata>(json).unwrap(),
        owner
    );
}

#[test]
fn exact_deployment_owner_rejects_unframed_or_partial_coordinates() {
    let mut invalid = deployment('d');
    invalid.deployment_artifact_identity = DeploymentArtifactIdentity::new("build-7");
    assert!(matches!(
        ExactDeploymentOwnerFrameMetadata::new(invalid),
        Err(ActorLifecycleContractError::InvalidSha256Identity {
            field: "deploymentOwner.deploymentArtifactIdentity",
            ..
        })
    ));

    let mut missing_service = deployment('d');
    missing_service.service_id.clear();
    assert_eq!(
        ExactDeploymentOwnerFrameMetadata::new(missing_service),
        Err(ActorLifecycleContractError::EmptyField {
            field: "deploymentOwner.serviceId"
        })
    );
}

#[test]
fn actor_fence_keeps_exact_build_incarnation_and_arena_epoch_distinct() {
    let build_a = fence('d');
    let build_b = fence('e');

    assert_ne!(build_a, build_b);
    assert_eq!(build_a.incarnation().get(), 7);
    assert_eq!(build_a.arena_epoch().get(), 3);
    assert_eq!(
        build_a.execution().build_id().as_str(),
        sha256_identity(DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, 'd')
    );

    let value = serde_json::to_value(&build_a).unwrap();
    assert_eq!(value["incarnation"], 7);
    assert_eq!(value["arenaEpoch"], 3);
    assert_eq!(value["ownerRuntimeId"], "runtime-1");
    assert_eq!(
        serde_json::from_value::<ExactActorOwnerFenceFrameMetadata>(value).unwrap(),
        build_a
    );
}

#[test]
fn actor_fence_rejects_service_and_counter_mismatches_structurally() {
    let mut wrong_actor = actor();
    wrong_actor.service_id = "example.com/other".to_string();
    assert!(matches!(
        ExactActorOwnerFenceFrameMetadata::new(
            wrong_actor,
            execution('d'),
            ActorIncarnation::new(1).unwrap(),
            ActorArenaEpoch::new(1).unwrap(),
            "runtime-1",
            "lease-1",
        ),
        Err(ActorLifecycleContractError::ActorDeploymentServiceMismatch { .. })
    ));

    assert_eq!(
        ActorIncarnation::new(0),
        Err(ActorLifecycleContractError::InvalidPositiveSequence {
            field: "incarnation",
            value: 0,
        })
    );
    assert_eq!(
        ActorArenaEpoch::new(JAVASCRIPT_MAX_SAFE_INTEGER + 1),
        Err(ActorLifecycleContractError::InvalidPositiveSequence {
            field: "arenaEpoch",
            value: JAVASCRIPT_MAX_SAFE_INTEGER + 1,
        })
    );
}

#[test]
fn actor_execution_identity_validates_typed_actor_identities() {
    let result = ExactActorExecutionIdentityFrameMetadata::new(
        deployment_owner('d'),
        ActorAbiIdentity::new("actor-abi-unframed"),
        ActorImplementationIdentity::new(sha256_identity(
            ACTOR_IMPLEMENTATION_IDENTITY_PREFIX,
            'b',
        )),
    );
    assert!(matches!(
        result,
        Err(ActorLifecycleContractError::InvalidSha256Identity {
            field: "actorAbiIdentity",
            ..
        })
    ));
}

#[test]
fn idle_discard_request_and_ack_round_trip_the_same_exact_fence() {
    let fence = fence('d');
    let request = ActorIdleDiscardRequestFrameHeader::new(
        "discard-1",
        fence.owner_runtime_id(),
        fence.clone(),
    )
    .unwrap();
    let request_wire = encode_actor_idle_discard_request_frame(&request).unwrap();
    assert_eq!(
        decode_actor_idle_discard_request_frame(&request_wire).unwrap(),
        request
    );

    let ack = ActorIdleDiscardAckFrameHeader::new(
        request.request_id(),
        request.target_runtime_id(),
        fence,
        ActorIdleDiscardAckOutcome::Discarded,
    )
    .unwrap();
    let ack_wire = encode_actor_idle_discard_ack_frame(&ack).unwrap();
    let decoded_ack = decode_actor_idle_discard_ack_frame(&ack_wire).unwrap();
    assert_eq!(decoded_ack.request_id(), request.request_id());
    assert_eq!(decoded_ack.fence(), request.fence());
    assert_eq!(
        request.validate_ack(&decoded_ack).unwrap(),
        ActorIdleDiscardAckOutcome::Discarded
    );
    assert!(decoded_ack.outcome().confirms_absence());
    assert!(!ActorIdleDiscardAckOutcome::FenceMismatch.confirms_absence());
}

#[test]
fn idle_discard_ack_cannot_clear_a_different_request_or_fence() {
    let request =
        ActorIdleDiscardRequestFrameHeader::new("discard-1", "runtime-1", fence('d')).unwrap();
    let wrong_request = ActorIdleDiscardAckFrameHeader::new(
        "discard-2",
        "runtime-1",
        fence('d'),
        ActorIdleDiscardAckOutcome::Discarded,
    )
    .unwrap();
    assert!(matches!(
        request.validate_ack(&wrong_request),
        Err(ActorLifecycleContractError::DiscardAckRequestMismatch { .. })
    ));

    let wrong_fence = ActorIdleDiscardAckFrameHeader::new(
        "discard-1",
        "runtime-1",
        fence('e'),
        ActorIdleDiscardAckOutcome::Discarded,
    )
    .unwrap();
    assert_eq!(
        request.validate_ack(&wrong_fence),
        Err(ActorLifecycleContractError::DiscardAckFenceMismatch)
    );
}

#[test]
fn idle_discard_frames_reject_runtime_mismatch_payload_and_unknown_fields() {
    assert!(matches!(
        ActorIdleDiscardRequestFrameHeader::new("discard-1", "runtime-2", fence('d')),
        Err(ActorLifecycleContractError::TargetRuntimeMismatch { .. })
    ));
    assert!(matches!(
        ActorIdleDiscardAckFrameHeader::new(
            "discard-1",
            "runtime-2",
            fence('d'),
            ActorIdleDiscardAckOutcome::AlreadyAbsent,
        ),
        Err(ActorLifecycleContractError::AckRuntimeMismatch { .. })
    ));

    let request =
        ActorIdleDiscardRequestFrameHeader::new("discard-1", "runtime-1", fence('d')).unwrap();
    let non_empty = encode_binary_frame(&request, b"not-empty").unwrap();
    assert!(decode_actor_idle_discard_request_frame(&non_empty).is_err());

    let mut value = serde_json::to_value(request).unwrap();
    value["generation"] = json!(9);
    assert!(serde_json::from_value::<ActorIdleDiscardRequestFrameHeader>(value).is_err());
}

#[test]
fn durable_snapshot_pins_exact_code_but_not_a_live_incarnation() {
    let snapshot =
        DurableActorActivationSnapshotFrameMetadata::new(actor(), execution('d'), activation())
            .unwrap();
    let value = serde_json::to_value(&snapshot).unwrap();
    let wire = serde_json::to_string(&value).unwrap();

    assert_eq!(
        value["execution"]["deploymentOwner"]["deploymentArtifactIdentity"],
        sha256_identity(DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX, 'd')
    );
    for forbidden in [
        "assemblyIdentity",
        "generation",
        "providerExecutable",
        "declarationOwner",
        "incarnation",
        "arenaEpoch",
        "ownerLeaseId",
    ] {
        assert!(!wire.contains(forbidden), "unexpected field {forbidden}");
    }
    assert_eq!(
        serde_json::from_value::<DurableActorActivationSnapshotFrameMetadata>(value).unwrap(),
        snapshot
    );
}

#[test]
fn durable_snapshot_reuses_and_validates_existing_activation_payload() {
    let mut invalid = activation();
    invalid.key = "not base64".to_string();
    assert!(matches!(
        DurableActorActivationSnapshotFrameMetadata::new(actor(), execution('d'), invalid),
        Err(ActorLifecycleContractError::InvalidActivationSnapshot { .. })
    ));
}

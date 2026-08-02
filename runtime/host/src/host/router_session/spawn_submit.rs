//! H-spawn-parent-cut driver-side `spawn.submit.request` encoder.
//!
//! The Runtime driver owns the production outbound wire for spawn submits.
//! After the cut it encodes the canonical
//! [`SpawnSubmitRequestFrameHeaderV2`] with the closed `callerKind` enum and
//! rejects the legacy shape (`OutboundControlMessage::SpawnSubmit` without
//! `callerKind`) with no compatible reader. `runtime/transport` keeps its
//! canonical codec; this module only maps the host control DTO onto it.

use base64::Engine as _;
use skiff_runtime_capability_context::{
    ActorInvocationOwnerFile, ActorInvocationOwnerUnit, SpawnCallerKind, SpawnSubmitControlMessage,
    SpawnSubmitControlRequest,
};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader, ActorOwnerFileFrameHeader,
    ActorOwnerUnitFrameHeader,
};
use skiff_runtime_transport::protocol::{
    encode_spawn_submit_request_frame, ActivationIdentityFrameMetadata,
    SpawnActorMethodTargetFrameMetadata, SpawnCallerKind as WireSpawnCallerKind,
    SpawnSubmitRequestFrameHeaderV2, SpawnTargetKind, RUNTIME_FRAME_SCHEMA_VERSION,
    SPAWN_SUBMIT_REQUEST_FRAME_TYPE,
};

use crate::error::{Result, RuntimeError};

/// Fail-closed rejection for the legacy outbound shape (no `callerKind`).
pub(crate) fn legacy_spawn_submit_rejected() -> RuntimeError {
    RuntimeError::Protocol {
        target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
        message: "spawn.submit.request without callerKind is not supported (H-spawn-parent-cut); the legacy shape has no compatible reader"
            .to_string(),
    }
}

pub(crate) fn encode_spawn_submit_wire_message(
    message: SpawnSubmitControlMessage,
) -> Result<Vec<u8>> {
    let header = spawn_submit_request_header_v2(&message.request, message.caller_kind)?;
    encode_spawn_submit_request_frame(&header, &message.payload).map_err(|error| {
        RuntimeError::Protocol {
            target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
            message: format!("failed to encode canonical spawn.submit.request: {error}"),
        }
    })
}

fn spawn_submit_request_header_v2(
    request: &SpawnSubmitControlRequest,
    caller_kind: SpawnCallerKind,
) -> Result<SpawnSubmitRequestFrameHeaderV2> {
    // The shared strict parser is still exposed through the historical
    // publication storage projection. This boundary owns a service ID, not a
    // storage path or package ID.
    skiff_artifact_identity::publication_storage_segment(&request.service_id, "service ID")
        .map_err(|_| RuntimeError::Protocol {
            target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
            message: "spawn.submit.request serviceId is not a valid publication service ID"
                .to_string(),
        })?;
    let caller_request_id =
        request
            .caller_request_id
            .clone()
            .ok_or_else(|| RuntimeError::Protocol {
                target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
                message:
                    "spawn.submit.request callerRequestId is required after H-spawn-parent-cut"
                        .to_string(),
            })?;
    let target_kind = match request.target_kind.as_str() {
        "function" => SpawnTargetKind::Function,
        "actorMethod" => SpawnTargetKind::ActorMethod,
        other => {
            return Err(RuntimeError::Protocol {
                target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
                message: format!(
                    "spawn.submit.request targetKind {other} is not a closed spawn target kind"
                ),
            });
        }
    };
    let activation_identity = ActivationIdentityFrameMetadata {
        assembly_identity: request
            .activation_identity
            .assembly_identity
            .clone()
            .into_string(),
        generation: request.activation_identity.generation,
        runtime_replica_id: request.activation_identity.runtime_replica_id.clone(),
        deployment_revision: request
            .activation_identity
            .deployment_revision
            .clone()
            .into_string(),
    };
    let actor_method = request
        .actor_method
        .as_ref()
        .map(actor_method_spawn_target_frame)
        .transpose()?;
    Ok(SpawnSubmitRequestFrameHeaderV2 {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
        rpc_id: request.rpc_id.clone(),
        runtime_id: request.runtime_id.clone(),
        caller_kind: match caller_kind {
            SpawnCallerKind::Request => WireSpawnCallerKind::Request,
            SpawnCallerKind::ActorInvocation => WireSpawnCallerKind::ActorInvocation,
        },
        caller_request_id,
        target_kind,
        service_id: request.service_id.clone(),
        service_version: request.service_version.clone(),
        service_protocol_identity: request.service_protocol_identity.clone(),
        target: request.target.clone(),
        spawn_id: request.spawn_id.clone(),
        build_id: request.build_id.clone(),
        activation_identity,
        trace_id: request.trace_id.clone(),
        caller_target: request.caller_target.clone(),
        max_queue_wait_ms: request.max_queue_wait_ms,
        actor_method,
    })
}

fn actor_method_spawn_target_frame(
    target: &skiff_runtime_capability_context::ActorMethodSpawnTargetControl,
) -> Result<SpawnActorMethodTargetFrameMetadata> {
    let epoch = target
        .actor_ref
        .epoch()
        .ok_or_else(|| RuntimeError::Protocol {
            target: SPAWN_SUBMIT_REQUEST_FRAME_TYPE.to_string(),
            message: "actor method spawn target requires a pinned Actor epoch".to_string(),
        })?;
    Ok(SpawnActorMethodTargetFrameMetadata {
        actor_ref: ActorLogicalRefFrameHeader {
            service_id: target.actor_ref.service_id().to_string(),
            actor_type_identity: target.actor_ref.actor_type_identity().to_string(),
            actor_id_type_identity: target.actor_ref.actor_id_type_identity().to_string(),
            actor_id_encoding_version: target.actor_ref.actor_id_encoding_version().to_string(),
            canonical_actor_id_key_bytes_base64: base64::engine::general_purpose::STANDARD
                .encode(target.actor_ref.canonical_actor_id_key_bytes()),
            actor_id_hash: target.actor_ref.actor_id_hash().to_string(),
            epoch,
        },
        declaration_owner: actor_declaration_owner_frame(&target.declaration_owner),
        actor_abi_identity: target.actor_abi_identity.clone(),
        actor_implementation_identity: target.actor_implementation_identity.clone(),
        method_identity: target.method_identity.clone(),
    })
}

fn actor_declaration_owner_frame(
    owner: &skiff_runtime_capability_context::ActorInvocationDeclarationOwner,
) -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: match &owner.unit {
            ActorInvocationOwnerUnit::Service => ActorOwnerUnitFrameHeader::Service,
            ActorInvocationOwnerUnit::Package(index) => ActorOwnerUnitFrameHeader::Package(*index),
        },
        file: match &owner.file {
            ActorInvocationOwnerFile::LoadedFileIndex(index) => {
                ActorOwnerFileFrameHeader::LoadedFileIndex(*index)
            }
            ActorInvocationOwnerFile::FileIrIdentity(identity) => {
                ActorOwnerFileFrameHeader::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: owner.actor_symbol.clone(),
    }
}

use base64::Engine as _;
use serde::Serialize;
use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorKeyControlMetadata, ActorRemoveControlRequest, ActorReplaceControlRequest,
    ConnectionRequestCancelControl, ConnectionRequestControl, ConnectionSendControl,
    OutboundControlMessage, RequestCancelControl, RuntimeDeadlineControl,
    SpawnSubmitControlRequest,
};

use crate::{
    actor_method::{
        ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
        ActorMethodDeadlineFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
    },
    cancel_reason::{request_cancel_wire_reason_for_internal, RequestCancelReason},
    connection_protocol::{
        encode_connection_request_cancel_frame, encode_connection_request_frame,
        ConnectionRequestCancelFrameHeader, ConnectionRequestFrameHeader, WebSocketRpcProfile,
    },
    error::TransportResult,
    protocol::{
        encode_binary_frame, ActivationIdentityFrameMetadata, ActorFindRequestFrameHeader,
        ActorGetOrCreateRequestFrameHeader, ActorKeyFrameMetadata, ActorRemoveRequestFrameHeader,
        ActorReplaceRequestFrameHeader, ConnectionSendFrameHeader, RequestCancelFrameHeader,
        RuntimeDeadlineFrameHeader, SpawnActorMethodTargetFrameMetadata,
        SpawnSubmitRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

pub fn encode_outbound_control_message(
    command: OutboundControlMessage,
) -> TransportResult<Vec<u8>> {
    match command {
        OutboundControlMessage::ActorGetOrCreate { request, payload } => {
            actor_get_or_create_request_frame(
                actor_get_or_create_request_frame_header(request),
                &payload,
            )
        }
        OutboundControlMessage::ActorReplace { request, payload } => {
            actor_replace_request_frame(actor_replace_request_frame_header(request), &payload)
        }
        OutboundControlMessage::ActorFind { request } => {
            actor_find_request_frame(actor_find_request_frame_header(request), &[])
        }
        OutboundControlMessage::ActorRemove { request } => {
            actor_remove_request_frame(actor_remove_request_frame_header(request), &[])
        }
        OutboundControlMessage::SpawnSubmit { request, payload } => {
            spawn_submit_request_frame(spawn_submit_request_frame_header(request)?, &payload)
        }
        OutboundControlMessage::RequestCancel { request } => {
            request_cancel_frame(request_cancel_frame_header(request), &[])
        }
        OutboundControlMessage::ConnectionSend { request, payload } => {
            connection_send_frame(connection_send_frame_header(request), &payload)
        }
        OutboundControlMessage::ConnectionRequest { request, payload } => {
            encode_connection_request_frame(&connection_request_frame_header(request)?, &payload)
        }
        OutboundControlMessage::ConnectionRequestCancel { request } => {
            encode_connection_request_cancel_frame(&connection_request_cancel_frame_header(
                request,
            )?)
        }
    }
}

pub fn connection_send_frame(
    header: ConnectionSendFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn actor_get_or_create_request_frame(
    header: ActorGetOrCreateRequestFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn actor_replace_request_frame(
    header: ActorReplaceRequestFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn actor_find_request_frame(
    header: ActorFindRequestFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn actor_remove_request_frame(
    header: ActorRemoveRequestFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn spawn_submit_request_frame(
    header: SpawnSubmitRequestFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn request_cancel_frame(
    header: RequestCancelFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

fn actor_get_or_create_request_frame_header(
    request: ActorGetOrCreateControlRequest,
) -> ActorGetOrCreateRequestFrameHeader {
    ActorGetOrCreateRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.getOrCreate.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        activation_identity: activation_identity_frame_metadata(request.activation_identity),
        actor_key: actor_key_frame_metadata(request.actor_key),
        actor_abi_identity: request.actor_abi_identity,
        actor_implementation_identity: request.actor_implementation_identity,
        bootstrap_encoding_version: request.bootstrap_encoding_version,
        declaration_owner: actor_declaration_owner_frame(request.declaration_owner),
        deadline: request.deadline.map(actor_control_deadline_frame),
    }
}

fn actor_replace_request_frame_header(
    request: ActorReplaceControlRequest,
) -> ActorReplaceRequestFrameHeader {
    ActorReplaceRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.replace.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        activation_identity: activation_identity_frame_metadata(request.activation_identity),
        actor_key: actor_key_frame_metadata(request.actor_key),
        actor_abi_identity: request.actor_abi_identity,
        actor_implementation_identity: request.actor_implementation_identity,
        bootstrap_encoding_version: request.bootstrap_encoding_version,
        declaration_owner: actor_declaration_owner_frame(request.declaration_owner),
        deadline: request.deadline.map(actor_control_deadline_frame),
    }
}

fn actor_declaration_owner_frame(
    owner: skiff_runtime_request_contract::ActorInvocationDeclarationOwner,
) -> ActorDeclarationOwnerFrameHeader {
    ActorDeclarationOwnerFrameHeader {
        unit: match owner.unit {
            skiff_runtime_request_contract::ActorInvocationOwnerUnit::Service => {
                ActorOwnerUnitFrameHeader::Service
            }
            skiff_runtime_request_contract::ActorInvocationOwnerUnit::Package(index) => {
                ActorOwnerUnitFrameHeader::Package(index)
            }
        },
        file: match owner.file {
            skiff_runtime_request_contract::ActorInvocationOwnerFile::LoadedFileIndex(index) => {
                ActorOwnerFileFrameHeader::LoadedFileIndex(index)
            }
            skiff_runtime_request_contract::ActorInvocationOwnerFile::FileIrIdentity(identity) => {
                ActorOwnerFileFrameHeader::FileIrIdentity(identity)
            }
        },
        actor_symbol: owner.actor_symbol,
    }
}

fn actor_control_deadline_frame(
    deadline: skiff_runtime_request_contract::ActorControlDeadline,
) -> ActorMethodDeadlineFrameHeader {
    ActorMethodDeadlineFrameHeader {
        timeout_ms: deadline.timeout_ms,
        expires_at: deadline.expires_at,
    }
}

fn actor_find_request_frame_header(
    request: ActorFindControlRequest,
) -> ActorFindRequestFrameHeader {
    ActorFindRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.find.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        activation_identity: activation_identity_frame_metadata(request.activation_identity),
        actor_key: actor_key_frame_metadata(request.actor_key),
    }
}

fn actor_remove_request_frame_header(
    request: ActorRemoveControlRequest,
) -> ActorRemoveRequestFrameHeader {
    ActorRemoveRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.remove.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        activation_identity: activation_identity_frame_metadata(request.activation_identity),
        actor_key: actor_key_frame_metadata(request.actor_key),
    }
}

fn actor_key_frame_metadata(metadata: ActorKeyControlMetadata) -> ActorKeyFrameMetadata {
    ActorKeyFrameMetadata {
        service_id: metadata.service_id,
        actor_type_identity: metadata.actor_type_identity,
        actor_id_type_identity: metadata.actor_id_type_identity,
        actor_id_encoding_version: metadata.actor_id_encoding_version,
        canonical_actor_id_key_bytes_base64: metadata.canonical_actor_id_key_bytes_base64,
        actor_id_hash: metadata.actor_id_hash,
    }
}

fn activation_identity_frame_metadata(
    identity: ActivationIdentityControl,
) -> ActivationIdentityFrameMetadata {
    ActivationIdentityFrameMetadata {
        assembly_identity: identity.assembly_identity.into_string(),
        generation: identity.generation,
        runtime_replica_id: identity.runtime_replica_id,
        deployment_revision: identity.deployment_revision.into_string(),
    }
}

fn spawn_submit_request_frame_header(
    request: SpawnSubmitControlRequest,
) -> TransportResult<SpawnSubmitRequestFrameHeader> {
    // The shared strict parser is still exposed through the historical publication storage
    // projection. Reuse that grammar without retaining its projected component; this boundary
    // owns a service ID, not a storage path or package ID.
    skiff_artifact_identity::publication_storage_segment(&request.service_id, "service ID")
        .map_err(|_| crate::TransportError::invalid_outbound_service_id("spawn.submit.request"))?;

    Ok(SpawnSubmitRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.submit.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        target_kind: request.target_kind,
        service_id: request.service_id,
        service_version: request.service_version,
        service_protocol_identity: request.service_protocol_identity,
        target: request.target,
        spawn_id: request.spawn_id,
        build_id: request.build_id,
        activation_identity: activation_identity_frame_metadata(request.activation_identity),
        caller_request_id: request.caller_request_id,
        trace_id: request.trace_id,
        caller_target: request.caller_target,
        max_queue_wait_ms: request.max_queue_wait_ms,
        actor_method: request
            .actor_method
            .map(actor_method_spawn_target_frame)
            .transpose()?,
    })
}

fn actor_method_spawn_target_frame(
    target: skiff_runtime_request_contract::ActorMethodSpawnTargetControl,
) -> TransportResult<SpawnActorMethodTargetFrameMetadata> {
    let epoch = target.actor_ref.epoch().ok_or_else(|| {
        crate::TransportError::decode(
            "actor method spawn target requires a pinned Actor epoch".to_string(),
        )
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
        declaration_owner: actor_declaration_owner_frame(target.declaration_owner),
        actor_abi_identity: target.actor_abi_identity,
        actor_implementation_identity: target.actor_implementation_identity,
        method_identity: target.method_identity,
    })
}

fn runtime_deadline_frame_header(deadline: RuntimeDeadlineControl) -> RuntimeDeadlineFrameHeader {
    RuntimeDeadlineFrameHeader {
        timeout_ms: deadline.timeout_ms,
        expires_at: deadline.expires_at,
    }
}

fn request_cancel_frame_header(request: RequestCancelControl) -> RequestCancelFrameHeader {
    RequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.cancel".to_string(),
        request_id: request.request_id,
        reason: request_cancel_wire_reason_for_internal(&request.reason).to_string(),
    }
}

fn connection_send_frame_header(request: ConnectionSendControl) -> ConnectionSendFrameHeader {
    ConnectionSendFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.send".to_string(),
        service_id: request.service_id,
        websocket_entry_id: request.websocket_entry_id,
        business_identity: request.business_identity,
        connection_id: request.connection_id,
        payload_kind: request.payload_kind,
    }
}

fn connection_request_frame_header(
    request: ConnectionRequestControl,
) -> TransportResult<ConnectionRequestFrameHeader> {
    let websocket_entry_id =
        skiff_artifact_model::WebSocketEntryId::parse(request.websocket_entry_id)
            .map_err(|error| crate::TransportError::decode(error.to_string()))?;
    Ok(ConnectionRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request".to_string(),
        request_id: request.request_id,
        service_id: request.service_id,
        websocket_entry_id,
        connection_id: request.connection_id,
        profile: WebSocketRpcProfile::JsonRpc2_0Text,
        method: request.method,
        deadline: request.deadline.map(runtime_deadline_frame_header),
    })
}

fn connection_request_cancel_frame_header(
    request: ConnectionRequestCancelControl,
) -> TransportResult<ConnectionRequestCancelFrameHeader> {
    let reason = RequestCancelReason::from_wire(&request.reason).ok_or_else(|| {
        crate::TransportError::decode(format!(
            "invalid connection request cancel reason {}",
            request.reason
        ))
    })?;
    Ok(ConnectionRequestCancelFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.request.cancel".to_string(),
        request_id: request.request_id,
        reason,
    })
}

fn encode_control_frame<THeader: Serialize>(
    header: &THeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_binary_frame(header, payload)
}

#[cfg(test)]
mod tests;

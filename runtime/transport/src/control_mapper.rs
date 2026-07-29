use serde::Serialize;
use skiff_runtime_request_contract::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorKeyControlMetadata, ActorRemoveControlRequest, ActorReplaceControlRequest,
    ConnectionRequestCancelControl, ConnectionRequestControl, ConnectionSendControl,
    OutboundControlMessage, RequestCancelControl, RuntimeDeadlineControl,
    SpawnSubmitControlRequest,
};

use crate::{
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
        RuntimeDeadlineFrameHeader, SpawnSubmitRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
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
mod tests {
    use super::{
        actor_find_request_frame, actor_get_or_create_request_frame, actor_remove_request_frame,
        actor_replace_request_frame, connection_send_frame, encode_outbound_control_message,
        request_cancel_frame, spawn_submit_request_frame,
    };
    use crate::protocol::{
        decode_typed_binary_frame, ActivationIdentityFrameMetadata, ActorFindRequestFrameHeader,
        ActorGetOrCreateRequestFrameHeader, ActorKeyFrameMetadata, ActorRemoveRequestFrameHeader,
        ActorReplaceRequestFrameHeader, ConnectionSendFrameHeader, RequestCancelFrameHeader,
        SpawnSubmitRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
    use skiff_runtime_request_contract::{
        ActivationIdentityControl, ActorGetOrCreateControlRequest, ActorKeyControlMetadata,
        ActorReplaceControlRequest, OutboundControlMessage, RequestCancelControl,
        SpawnSubmitControlRequest,
    };

    #[test]
    fn connection_send_frame_maps_header_and_opaque_payload() {
        let header = ConnectionSendFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "connection.send".to_string(),
            service_id: "example.com/chat".to_string(),
            websocket_entry_id: Some("gateway.websocket.chat".to_string()),
            business_identity: Some("user-1".to_string()),
            connection_id: None,
            payload_kind: Some("text".to_string()),
        };
        let payload = b"opaque websocket bytes".to_vec();

        let frame =
            connection_send_frame(header.clone(), &payload).expect("connection frame encodes");
        let (decoded, decoded_payload): (ConnectionSendFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("connection frame decodes");

        assert_eq!(decoded, header);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn actor_control_request_frames_map_headers_and_opaque_payloads() {
        let actor_key = actor_key();
        let get_or_create_header = ActorGetOrCreateRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.getOrCreate.request".to_string(),
            rpc_id: "rpc-get-or-create".to_string(),
            runtime_id: "runtime-1".to_string(),
            activation_identity: activation_identity_frame(),
            actor_key: actor_key.clone(),
            actor_abi_identity: "actor-abi:1".to_string(),
            actor_implementation_identity: "build:1".to_string(),
            bootstrap_encoding_version: "canonical-value-v1".to_string(),
        };
        let bootstrap_payload = b"canonical actor bootstrap".to_vec();

        let get_or_create_frame =
            actor_get_or_create_request_frame(get_or_create_header.clone(), &bootstrap_payload)
                .expect("getOrCreate frame encodes");
        let (decoded_get_or_create, decoded_bootstrap): (
            ActorGetOrCreateRequestFrameHeader,
            Vec<u8>,
        ) = decode_typed_binary_frame(&get_or_create_frame).expect("getOrCreate frame decodes");

        assert_eq!(decoded_get_or_create, get_or_create_header);
        assert_eq!(decoded_bootstrap, bootstrap_payload);

        let replace_header = ActorReplaceRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.replace.request".to_string(),
            rpc_id: "rpc-replace".to_string(),
            runtime_id: "runtime-1".to_string(),
            activation_identity: activation_identity_frame(),
            actor_key: actor_key.clone(),
            actor_abi_identity: "actor-abi:1".to_string(),
            actor_implementation_identity: "build:2".to_string(),
            bootstrap_encoding_version: "canonical-value-v1".to_string(),
        };
        let replace_frame = actor_replace_request_frame(replace_header.clone(), &bootstrap_payload)
            .expect("replace frame encodes");
        let (decoded_replace, decoded_replace_bootstrap): (
            ActorReplaceRequestFrameHeader,
            Vec<u8>,
        ) = decode_typed_binary_frame(&replace_frame).expect("replace frame decodes");
        assert_eq!(decoded_replace, replace_header);
        assert_eq!(decoded_replace_bootstrap, bootstrap_payload);

        let find_header = ActorFindRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.request".to_string(),
            rpc_id: "rpc-find".to_string(),
            runtime_id: "runtime-1".to_string(),
            activation_identity: activation_identity_frame(),
            actor_key: actor_key.clone(),
        };

        let find_frame =
            actor_find_request_frame(find_header.clone(), &[]).expect("find frame encodes");
        let (decoded_find, decoded_find_payload): (ActorFindRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&find_frame).expect("find frame decodes");

        assert_eq!(decoded_find, find_header);
        assert!(decoded_find_payload.is_empty());

        let remove_header = ActorRemoveRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.remove.request".to_string(),
            rpc_id: "rpc-remove".to_string(),
            runtime_id: "runtime-1".to_string(),
            activation_identity: activation_identity_frame(),
            actor_key,
        };

        let remove_frame =
            actor_remove_request_frame(remove_header.clone(), &[]).expect("remove frame encodes");
        let (decoded_remove, decoded_remove_payload): (ActorRemoveRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&remove_frame).expect("remove frame decodes");

        assert_eq!(decoded_remove, remove_header);
        assert!(decoded_remove_payload.is_empty());
    }

    #[test]
    fn spawn_submit_request_frame_maps_header_and_opaque_payload() {
        let header = SpawnSubmitRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "spawn.submit.request".to_string(),
            rpc_id: "rpc-spawn".to_string(),
            runtime_id: "runtime-1".to_string(),
            target_kind: "operation".to_string(),
            service_id: "example.com/worker".to_string(),
            service_version: "1.0.0".to_string(),
            service_protocol_identity: "service-protocol-1".to_string(),
            target: "Worker.run".to_string(),
            spawn_id: Some("spawn-1".to_string()),
            build_id: Some("build-1".to_string()),
            activation_identity: activation_identity_frame(),
            caller_request_id: Some("request-1".to_string()),
            trace_id: Some("trace-1".to_string()),
            caller_target: Some("Caller.start".to_string()),
            max_queue_wait_ms: Some(250.0),
        };
        let payload = b"opaque spawn args".to_vec();

        let frame =
            spawn_submit_request_frame(header.clone(), &payload).expect("spawn frame encodes");
        let (decoded, decoded_payload): (SpawnSubmitRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("spawn frame decodes");

        assert_eq!(decoded, header);
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn outbound_spawn_submit_rejects_package_id_embedded_in_service_id() {
        let error = encode_outbound_control_message(OutboundControlMessage::SpawnSubmit {
            request: spawn_submit_control_request("test.skiff/agine.ai/api-tests/case-23"),
            payload: b"opaque spawn args".to_vec(),
        })
        .expect_err("non-canonical service ID must fail before frame encoding");

        assert!(matches!(
            &error,
            crate::TransportError::InvalidOutboundServiceId {
                envelope_type: "spawn.submit.request",
                ..
            }
        ));
        assert!(
            error.to_string().contains("service ID"),
            "mapping error must identify the service ID boundary: {error}"
        );
    }

    #[test]
    fn outbound_spawn_submit_accepts_existing_and_generated_service_ids() {
        for service_id in [
            "example.com/worker",
            "test.skiff/p-0123456789abcdef0123456789abcdef/case-23",
        ] {
            let frame = encode_outbound_control_message(OutboundControlMessage::SpawnSubmit {
                request: spawn_submit_control_request(service_id),
                payload: b"opaque spawn args".to_vec(),
            })
            .expect("canonical service ID should encode");
            let (header, payload): (SpawnSubmitRequestFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("spawn.submit.request should decode");

            assert_eq!(header.service_id, service_id);
            assert_eq!(payload, b"opaque spawn args");
        }
    }

    #[test]
    fn outbound_spawn_submit_rejects_invalid_service_id_forms() {
        let overlong = format!("example.com/{}", "a".repeat(64));
        for service_id in [
            "",
            overlong.as_str(),
            "Example.com/worker",
            "example.com/bad!",
        ] {
            let error = encode_outbound_control_message(OutboundControlMessage::SpawnSubmit {
                request: spawn_submit_control_request(service_id),
                payload: b"opaque spawn args".to_vec(),
            })
            .expect_err("invalid service ID must fail before frame encoding");

            assert!(matches!(
                &error,
                crate::TransportError::InvalidOutboundServiceId {
                    envelope_type: "spawn.submit.request",
                    ..
                }
            ));
        }
    }

    #[test]
    fn request_cancel_frame_maps_header_and_empty_payload() {
        let header = RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: "request-1".to_string(),
            reason: "caller_cancel".to_string(),
        };

        let frame = request_cancel_frame(header.clone(), &[]).expect("cancel frame encodes");
        let (decoded, decoded_payload): (RequestCancelFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("cancel frame decodes");

        assert_eq!(decoded, header);
        assert!(decoded_payload.is_empty());
    }

    #[test]
    fn outbound_request_cancel_control_encodes_binary_frame() {
        let frame = encode_outbound_control_message(OutboundControlMessage::RequestCancel {
            request: RequestCancelControl {
                request_id: "request-cancel-1".to_string(),
                reason: "caller_cancel".to_string(),
            },
        })
        .expect("outbound cancel encodes");
        let (decoded, decoded_payload): (RequestCancelFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("cancel frame decodes");

        assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(decoded.envelope_type, "request.cancel");
        assert_eq!(decoded.request_id, "request-cancel-1");
        assert_eq!(decoded.reason, "caller_cancel");
        assert!(decoded_payload.is_empty());
    }

    #[test]
    fn outbound_request_cancel_control_maps_internal_reason() {
        let frame = encode_outbound_control_message(OutboundControlMessage::RequestCancel {
            request: RequestCancelControl {
                request_id: "request-cancel-1".to_string(),
                reason: "chunk_seq_mismatch".to_string(),
            },
        })
        .expect("outbound cancel encodes");
        let (decoded, _): (RequestCancelFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("cancel frame decodes");

        assert_eq!(decoded.reason, "protocol_error");
    }

    #[test]
    fn outbound_actor_get_or_create_and_replace_controls_have_distinct_wire_types() {
        let payload = b"canonical actor bootstrap".to_vec();
        let frame = encode_outbound_control_message(OutboundControlMessage::ActorGetOrCreate {
            request: ActorGetOrCreateControlRequest {
                rpc_id: "rpc-get-or-create-1".to_string(),
                runtime_id: "runtime-1".to_string(),
                activation_identity: activation_identity_control(),
                actor_key: actor_key_control(),
                actor_abi_identity: "actor-abi:1".to_string(),
                actor_implementation_identity: "build:1".to_string(),
                bootstrap_encoding_version: "canonical-value-v1".to_string(),
            },
            payload: payload.clone(),
        })
        .expect("outbound actor getOrCreate encodes");
        let (decoded, decoded_payload): (ActorGetOrCreateRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("actor.getOrCreate.request decodes");

        assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(decoded.envelope_type, "actor.getOrCreate.request");
        assert_eq!(decoded.rpc_id, "rpc-get-or-create-1");
        assert_eq!(decoded.runtime_id, "runtime-1");
        assert_eq!(decoded.actor_key, actor_key());
        assert_eq!(decoded.actor_abi_identity, "actor-abi:1");
        assert_eq!(decoded.bootstrap_encoding_version, "canonical-value-v1");
        assert_eq!(decoded_payload, payload);

        let replace_frame = encode_outbound_control_message(OutboundControlMessage::ActorReplace {
            request: ActorReplaceControlRequest {
                rpc_id: "rpc-replace-1".to_string(),
                runtime_id: "runtime-1".to_string(),
                activation_identity: activation_identity_control(),
                actor_key: actor_key_control(),
                actor_abi_identity: "actor-abi:1".to_string(),
                actor_implementation_identity: "build:2".to_string(),
                bootstrap_encoding_version: "canonical-value-v1".to_string(),
            },
            payload: payload.clone(),
        })
        .expect("outbound actor replace encodes");
        let (decoded_replace, decoded_replace_payload): (ActorReplaceRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&replace_frame).expect("actor.replace.request decodes");
        assert_eq!(decoded_replace.envelope_type, "actor.replace.request");
        assert_eq!(decoded_replace_payload, payload);
    }

    fn actor_key() -> ActorKeyFrameMetadata {
        ActorKeyFrameMetadata {
            service_id: "example.com/actor".to_string(),
            actor_type_identity: "actor-type-1".to_string(),
            actor_id_type_identity: "actor-id-type-1".to_string(),
            actor_id_encoding_version: "v1".to_string(),
            canonical_actor_id_key_bytes_base64: "YWN0b3Ita2V5".to_string(),
            actor_id_hash: Some("actor-hash-1".to_string()),
        }
    }

    fn activation_identity_frame() -> ActivationIdentityFrameMetadata {
        ActivationIdentityFrameMetadata {
            assembly_identity:
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            generation: 7,
            runtime_replica_id: "runtime-replica-7".to_string(),
            deployment_revision: "deployment-revision-7".to_string(),
        }
    }

    fn activation_identity_control() -> ActivationIdentityControl {
        ActivationIdentityControl {
            assembly_identity: AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
            generation: 7,
            runtime_replica_id: "runtime-replica-7".to_string(),
            deployment_revision: DeploymentRevision::new("deployment-revision-7"),
        }
    }

    fn actor_key_control() -> ActorKeyControlMetadata {
        ActorKeyControlMetadata {
            service_id: "example.com/actor".to_string(),
            actor_type_identity: "actor-type-1".to_string(),
            actor_id_type_identity: "actor-id-type-1".to_string(),
            actor_id_encoding_version: "v1".to_string(),
            canonical_actor_id_key_bytes_base64: "YWN0b3Ita2V5".to_string(),
            actor_id_hash: Some("actor-hash-1".to_string()),
        }
    }

    fn spawn_submit_control_request(service_id: &str) -> SpawnSubmitControlRequest {
        SpawnSubmitControlRequest {
            rpc_id: "rpc-spawn".to_string(),
            runtime_id: "runtime-1".to_string(),
            target_kind: "operation".to_string(),
            service_id: service_id.to_string(),
            service_version: "1.0.0".to_string(),
            service_protocol_identity: "service-protocol-1".to_string(),
            target: "Worker.run".to_string(),
            spawn_id: Some("spawn-1".to_string()),
            build_id: Some("build-1".to_string()),
            activation_identity: activation_identity_control(),
            caller_request_id: Some("request-1".to_string()),
            trace_id: Some("trace-1".to_string()),
            caller_target: Some("Caller.start".to_string()),
            max_queue_wait_ms: Some(250.0),
        }
    }
}

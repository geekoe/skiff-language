use serde::Serialize;
use skiff_runtime_request_contract::{
    ActorFindControlRequest, ActorKeyControlMetadata, ActorPutControlRequest,
    ActorRemoveControlRequest, ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RequestEffectDoubleControl, RequestStartControl, RuntimeCallerControl, RuntimeDeadlineControl,
    RuntimeTraceContextControl, SpawnSubmitControlRequest,
};

use crate::{
    cancel_reason::request_cancel_wire_reason_for_internal,
    error::TransportResult,
    protocol::{
        encode_binary_frame, ActorFindRequestFrameHeader, ActorKeyFrameMetadata,
        ActorPutRequestFrameHeader, ActorRemoveRequestFrameHeader, ConnectionSendFrameHeader,
        RequestCancelFrameHeader, RequestStartFrameHeader, RequestTestEffectDouble,
        RuntimeCallerFrameHeader, RuntimeDeadlineFrameHeader, RuntimeTraceContextFrameHeader,
        SpawnSubmitRequestFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
    },
};

pub fn encode_outbound_control_message(
    command: OutboundControlMessage,
) -> TransportResult<Vec<u8>> {
    match command {
        OutboundControlMessage::ActorPut { request, payload } => {
            actor_put_request_frame(actor_put_request_frame_header(request), &payload)
        }
        OutboundControlMessage::ActorFind { request } => {
            actor_find_request_frame(actor_find_request_frame_header(request), &[])
        }
        OutboundControlMessage::ActorRemove { request } => {
            actor_remove_request_frame(actor_remove_request_frame_header(request), &[])
        }
        OutboundControlMessage::SpawnSubmit { request, payload } => {
            spawn_submit_request_frame(spawn_submit_request_frame_header(request), &payload)
        }
        OutboundControlMessage::RequestStart { request, payload } => {
            request_start_frame(request_start_frame_header(request), &payload)
        }
        OutboundControlMessage::RequestCancel { request } => {
            request_cancel_frame(request_cancel_frame_header(request), &[])
        }
        OutboundControlMessage::ConnectionSend { request, payload } => {
            connection_send_frame(connection_send_frame_header(request), &payload)
        }
    }
}

pub fn connection_send_frame(
    header: ConnectionSendFrameHeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_control_frame(&header, payload)
}

pub fn actor_put_request_frame(
    header: ActorPutRequestFrameHeader,
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

pub fn request_start_frame(
    header: RequestStartFrameHeader,
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

fn actor_put_request_frame_header(request: ActorPutControlRequest) -> ActorPutRequestFrameHeader {
    ActorPutRequestFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.put.request".to_string(),
        rpc_id: request.rpc_id,
        runtime_id: request.runtime_id,
        actor_key: actor_key_frame_metadata(request.actor_key),
        object_schema_identity: request.object_schema_identity,
        object_encoding_version: request.object_encoding_version,
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

fn spawn_submit_request_frame_header(
    request: SpawnSubmitControlRequest,
) -> SpawnSubmitRequestFrameHeader {
    SpawnSubmitRequestFrameHeader {
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
        activation_identity: request.activation_identity,
        caller_request_id: request.caller_request_id,
        trace_id: request.trace_id,
        caller_target: request.caller_target,
        max_queue_wait_ms: request.max_queue_wait_ms,
    }
}

fn request_start_frame_header(request: RequestStartControl) -> RequestStartFrameHeader {
    RequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "request.start".to_string(),
        request_id: request.request_id,
        mode: request.mode,
        caller: runtime_caller_frame_header(request.caller),
        target: request.target,
        operation_abi_id: request.operation_abi_id,
        selector: request.selector,
        service_id: request.service_id,
        version: request.version,
        build_id: request.build_id,
        service_protocol_identity: request.service_protocol_identity,
        activation_identity: request.activation_identity,
        gateway_entry_identity: request.gateway_entry_identity,
        business_identity: request.business_identity,
        websocket_entry_id: request.websocket_entry_id,
        client_session: request.client_session,
        deadline: request.deadline.map(runtime_deadline_frame_header),
        trace: runtime_trace_context_frame_header(request.trace),
        http_request: None,
        http_adapter: None,
        websocket_adapter: None,
        test_effects_enabled: request.test_effects_enabled,
        test_effect_doubles: request
            .test_effect_doubles
            .into_iter()
            .map(|(target, sequence)| {
                (
                    target,
                    sequence
                        .into_iter()
                        .map(request_test_effect_double_frame_header)
                        .collect(),
                )
            })
            .collect(),
    }
}

fn runtime_caller_frame_header(caller: RuntimeCallerControl) -> RuntimeCallerFrameHeader {
    RuntimeCallerFrameHeader {
        kind: caller.kind,
        target: caller.target,
    }
}

fn runtime_deadline_frame_header(deadline: RuntimeDeadlineControl) -> RuntimeDeadlineFrameHeader {
    RuntimeDeadlineFrameHeader {
        timeout_ms: deadline.timeout_ms,
        expires_at: deadline.expires_at,
    }
}

fn runtime_trace_context_frame_header(
    trace: RuntimeTraceContextControl,
) -> RuntimeTraceContextFrameHeader {
    RuntimeTraceContextFrameHeader {
        trace_id: trace.trace_id,
        span_id: trace.span_id,
        parent_span_id: trace.parent_span_id,
        sampled: trace.sampled,
    }
}

fn request_test_effect_double_frame_header(
    double: RequestEffectDoubleControl,
) -> RequestTestEffectDouble {
    RequestTestEffectDouble {
        expect_request: double.expect_request,
        response: double.response,
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

fn encode_control_frame<THeader: Serialize>(
    header: &THeader,
    payload: &[u8],
) -> TransportResult<Vec<u8>> {
    encode_binary_frame(header, payload)
}

#[cfg(test)]
mod tests {
    use super::{
        actor_find_request_frame, actor_put_request_frame, actor_remove_request_frame,
        connection_send_frame, encode_outbound_control_message, request_cancel_frame,
        request_start_frame, spawn_submit_request_frame,
    };
    use crate::protocol::{
        decode_typed_binary_frame, ActorFindRequestFrameHeader, ActorKeyFrameMetadata,
        ActorPutRequestFrameHeader, ActorRemoveRequestFrameHeader, ConnectionSendFrameHeader,
        RequestCancelFrameHeader, RequestStartFrameHeader, RuntimeCallerFrameHeader,
        RuntimeTraceContextFrameHeader, SpawnSubmitRequestFrameHeader,
        RUNTIME_FRAME_SCHEMA_VERSION,
    };
    use serde_json::json;
    use skiff_runtime_request_contract::{
        ActorKeyControlMetadata, ActorPutControlRequest, OutboundControlMessage,
        RequestCancelControl, RequestEffectDoubleControl, RequestStartControl,
        RuntimeCallerControl, RuntimeClientSessionControl, RuntimeDeadlineControl,
        RuntimeTraceContextControl,
    };
    use std::collections::HashMap;

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
        let put_header = ActorPutRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.put.request".to_string(),
            rpc_id: "rpc-put".to_string(),
            runtime_id: "runtime-1".to_string(),
            actor_key: actor_key.clone(),
            object_schema_identity: "schema:object".to_string(),
            object_encoding_version: "v1".to_string(),
        };
        let put_payload = b"opaque actor object".to_vec();

        let put_frame =
            actor_put_request_frame(put_header.clone(), &put_payload).expect("put frame encodes");
        let (decoded_put, decoded_put_payload): (ActorPutRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&put_frame).expect("put frame decodes");

        assert_eq!(decoded_put, put_header);
        assert_eq!(decoded_put_payload, put_payload);

        let find_header = ActorFindRequestFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "actor.find.request".to_string(),
            rpc_id: "rpc-find".to_string(),
            runtime_id: "runtime-1".to_string(),
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
            activation_identity: Some("activation-1".to_string()),
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
    fn request_start_frame_maps_header_and_opaque_payload() {
        let header = RequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.start".to_string(),
            request_id: "request-1".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "service".to_string(),
                target: "Caller.run".to_string(),
            },
            target: "Worker.run".to_string(),
            operation_abi_id: Some("operation-worker-run".to_string()),
            selector: Some("operation:operation-worker-run".to_string()),
            service_id: Some("example.com/worker".to_string()),
            version: Some("1.0.0".to_string()),
            build_id: "build-1".to_string(),
            service_protocol_identity: "service-protocol-1".to_string(),
            activation_identity: Some("activation-1".to_string()),
            gateway_entry_identity: None,
            business_identity: Some("business-1".to_string()),
            websocket_entry_id: None,
            client_session: None,
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-1".to_string(),
                span_id: "span-1".to_string(),
                parent_span_id: Some("parent-span-1".to_string()),
                sampled: Some(true),
            },
            http_request: None,
            http_adapter: None,
            websocket_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
        };
        let payload = b"opaque service request bytes".to_vec();

        let frame = request_start_frame(header.clone(), &payload).expect("start frame encodes");
        let (decoded, decoded_payload): (RequestStartFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("start frame decodes");

        assert_eq!(decoded, header);
        assert_eq!(decoded_payload, payload);
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
    fn outbound_request_start_control_encodes_binary_frame() {
        let mut test_effect_doubles = HashMap::new();
        test_effect_doubles.insert(
            "Worker.run".to_string(),
            vec![RequestEffectDoubleControl {
                expect_request: Some(json!({ "name": "Ada" })),
                response: json!({ "ok": true }),
            }],
        );
        let payload = br#"{"name":"Ada"}"#.to_vec();

        let frame = encode_outbound_control_message(OutboundControlMessage::RequestStart {
            request: RequestStartControl {
                request_id: "request-start-1".to_string(),
                mode: "unary".to_string(),
                caller: RuntimeCallerControl {
                    kind: "service".to_string(),
                    target: "Caller.run".to_string(),
                },
                target: "Worker.run".to_string(),
                operation_abi_id: Some("operation-worker-run".to_string()),
                selector: Some("operation:operation-worker-run".to_string()),
                service_id: Some("example.com/worker".to_string()),
                version: Some("1.0.0".to_string()),
                build_id: "build-1".to_string(),
                service_protocol_identity: "service-protocol-1".to_string(),
                activation_identity: Some("activation-1".to_string()),
                gateway_entry_identity: Some("gateway-1".to_string()),
                business_identity: Some("business-1".to_string()),
                websocket_entry_id: Some("websocket-1".to_string()),
                client_session: Some(RuntimeClientSessionControl {
                    id: "client-1".to_string(),
                }),
                deadline: Some(RuntimeDeadlineControl {
                    timeout_ms: 5000,
                    expires_at: "2026-07-01T00:00:05Z".to_string(),
                }),
                trace: RuntimeTraceContextControl {
                    trace_id: "trace-1".to_string(),
                    span_id: "span-1".to_string(),
                    parent_span_id: Some("parent-span-1".to_string()),
                    sampled: Some(true),
                },
                test_effects_enabled: true,
                test_effect_doubles,
            },
            payload: payload.clone(),
        })
        .expect("outbound start encodes");
        let (decoded, decoded_payload): (RequestStartFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("start frame decodes");

        assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(decoded.envelope_type, "request.start");
        assert_eq!(decoded.request_id, "request-start-1");
        assert_eq!(decoded.caller.kind, "service");
        assert_eq!(decoded.caller.target, "Caller.run");
        assert_eq!(decoded.target, "Worker.run");
        assert_eq!(decoded.deadline.expect("deadline maps").timeout_ms, 5000);
        assert_eq!(decoded.trace.trace_id, "trace-1");
        assert_eq!(decoded.trace.sampled, Some(true));
        assert!(decoded.http_request.is_none());
        assert!(decoded.http_adapter.is_none());
        assert!(decoded.websocket_adapter.is_none());
        assert!(decoded.test_effects_enabled);
        assert_eq!(
            decoded.test_effect_doubles["Worker.run"][0].expect_request,
            Some(json!({ "name": "Ada" }))
        );
        assert_eq!(
            decoded.test_effect_doubles["Worker.run"][0].response,
            json!({ "ok": true })
        );
        assert_eq!(decoded_payload, payload);
    }

    #[test]
    fn outbound_actor_put_control_encodes_binary_frame() {
        let payload = b"opaque actor object".to_vec();
        let frame = encode_outbound_control_message(OutboundControlMessage::ActorPut {
            request: ActorPutControlRequest {
                rpc_id: "rpc-put-1".to_string(),
                runtime_id: "runtime-1".to_string(),
                actor_key: actor_key_control(),
                object_schema_identity: "schema:object".to_string(),
                object_encoding_version: "v1".to_string(),
            },
            payload: payload.clone(),
        })
        .expect("outbound actor put encodes");
        let (decoded, decoded_payload): (ActorPutRequestFrameHeader, Vec<u8>) =
            decode_typed_binary_frame(&frame).expect("actor.put.request decodes");

        assert_eq!(decoded.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
        assert_eq!(decoded.envelope_type, "actor.put.request");
        assert_eq!(decoded.rpc_id, "rpc-put-1");
        assert_eq!(decoded.runtime_id, "runtime-1");
        assert_eq!(decoded.actor_key, actor_key());
        assert_eq!(decoded.object_schema_identity, "schema:object");
        assert_eq!(decoded.object_encoding_version, "v1");
        assert_eq!(decoded_payload, payload);
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
}

use super::*;
use skiff_artifact_model::WEBSOCKET_GATEWAY_ENTRY_KEY;

struct DecodeTarget {
    key: GatewayEntryKey,
}

impl DecodeTarget {
    fn new() -> Self {
        Self {
            key: GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap(),
        }
    }
}

impl RuntimeWebSocketConnectExecutionTarget for DecodeTarget {
    fn eval_target(&self) -> &RuntimeAssemblyEvalTarget {
        panic!("result decoding does not consult an eval target")
    }

    fn gateway_entry_key(&self) -> &GatewayEntryKey {
        &self.key
    }

    fn gateway_entry_identity(&self) -> &GatewayEntryIdentity {
        panic!("result decoding does not consult the gateway identity")
    }

    fn websocket_entry_id(&self) -> &WebSocketEntryId {
        panic!("result decoding does not consult the internal entry id")
    }

    fn protocol_surface(&self) -> &GatewayEntryProtocolSurface {
        panic!("result decoding does not consult the protocol surface")
    }

    fn adapter_plan(&self) -> &GatewayAdapterPlan {
        panic!("result decoding does not consult the adapter plan")
    }

    fn handler(&self) -> RuntimeWebSocketConnectCallable<'_> {
        panic!("result decoding does not consult the handler")
    }
}

#[test]
fn websocket_connect_result_decodes_accept_with_optional_identity_and_policy() {
    let target = DecodeTarget::new();
    assert_eq!(
        decode_connect_result(
            &target,
            json!({
                "tag": "accept",
                "businessIdentity": "tenant-1",
                "connectionPolicy": {
                    "maxConnections": 3,
                    "overflow": "close-oldest",
                    "closeCode": 4001,
                    "closeReason": "replaced"
                }
            })
        )
        .unwrap(),
        RuntimeWebSocketConnectResult::Accept {
            business_identity: Some("tenant-1".to_string()),
            connection_policy: Some(WebSocketConnectionPolicyControl {
                max_connections: NonZeroU32::new(3).unwrap(),
                overflow: WebSocketConnectionPolicyOverflowControl::CloseOldest,
                close_code: Some(4001),
                close_reason: Some("replaced".to_string()),
            }),
        }
    );
    assert_eq!(
        decode_connect_result(
            &target,
            json!({
                "tag": "accept",
                "businessIdentity": null,
                "connectionPolicy": null
            })
        )
        .unwrap(),
        RuntimeWebSocketConnectResult::Accept {
            business_identity: None,
            connection_policy: None,
        }
    );
}

#[test]
fn websocket_connect_result_decodes_reject_and_refuses_noncanonical_shapes() {
    let target = DecodeTarget::new();
    assert_eq!(
        decode_connect_result(
            &target,
            json!({"tag": "reject", "code": 1008, "reason": "policy"})
        )
        .unwrap(),
        RuntimeWebSocketConnectResult::Reject {
            code: 1008,
            reason: "policy".to_string(),
        }
    );

    for invalid in [
        json!({"tag": "accept", "businessIdentity": null}),
        json!({
            "tag": "accept",
            "businessIdentity": null,
            "connectionPolicy": {
                "maxConnections": 0,
                "overflow": "close-oldest",
                "closeCode": null,
                "closeReason": null
            }
        }),
        json!({"tag": "reject", "code": 65536, "reason": "policy"}),
        json!({"tag": "reject", "code": 1008, "reason": "policy", "legacy": true}),
    ] {
        assert!(decode_connect_result(&target, invalid).is_err());
    }
}

#[test]
fn native_websocket_connect_refuses_jsonrpc_only_sources_before_value_projection() {
    let target = DecodeTarget::new();
    let request = RuntimeWebSocketConnectRequest {
        connection_id: "connection-1".to_string(),
        url: "ws://websocket.test/ws".to_string(),
        query: Vec::new(),
        headers: Vec::new(),
        cookies: Vec::new(),
        version: None,
        websocket_entry_id: WebSocketEntryId::parse(format!(
            "skiff-websocket-entry-v1:sha256:{}",
            "a".repeat(64)
        ))
        .unwrap(),
        gateway_entry_identity: GatewayEntryIdentity::parse(format!(
            "skiff-gateway-entry-v2:sha256:{}",
            "b".repeat(64)
        ))
        .unwrap(),
    };

    for source in [
        GatewayAdapterSource::WebSocketJsonRpcParams,
        GatewayAdapterSource::WebSocketBusinessIdentity,
    ] {
        let error = websocket_connect_source_wire(&request, &target, source)
            .expect_err("connect evaluator must reject JSON-RPC-only sources");
        assert!(
            error
                .to_string()
                .contains("WebSocket JSON-RPC-only adapter sources"),
            "{error}"
        );
    }
}

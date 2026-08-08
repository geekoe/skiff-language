use super::*;
use skiff_artifact_model::{
    GatewayAdapterArg, GatewayAdapterSource, GatewayExternalErrorProjection,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketRpcProfile, GatewayWebSocketShapeVersion, PackageCallableParameter,
    PackageTypeRef, TypeRefIr,
};

fn surface() -> GatewayEntryProtocolSurface {
    GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                connection_close_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectRequest,
                    GatewayAdapterSource::WebSocketConnectionId,
                ],
                close_external_sources: vec![],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Binary,
                    GatewayWebSocketDownlinkFrame::Text,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    }
}

fn signature(parameter: &str) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: parameter.to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    }
}

#[test]
fn websocket_connect_target_requires_real_handler_and_exact_plan() {
    let key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY).unwrap();
    let surface = surface();
    let identity = gateway_entry_identity(&surface).unwrap();
    let plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::WebSocketConnect,
        args: vec![GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::WebSocketConnectRequest,
        }],
    };
    let canonical_signature = signature("request");

    validate_entry_fact_view(WebSocketEntryValidationFacts {
        key: &key,
        identity: &identity,
        surface: &surface,
        plan: &plan,
        handler_signature: Some(&canonical_signature),
        has_pre: false,
        has_guard: false,
    })
    .expect("canonical connect target facts");

    assert!(matches!(
        validate_entry_fact_view(WebSocketEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &plan,
            handler_signature: None,
            has_pre: false,
            has_guard: false,
        }),
        Err(RuntimeAssemblyWebSocketConnectTargetError::HandlerRequired)
    ));

    assert!(matches!(
        validate_entry_fact_view(WebSocketEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &plan,
            handler_signature: Some(&signature("other")),
            has_pre: false,
            has_guard: false,
        }),
        Err(RuntimeAssemblyWebSocketConnectTargetError::HandlerPlanMismatch)
    ));
}

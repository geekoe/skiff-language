use skiff_artifact_model::{
    DeploymentRevision, GatewayAdapterArg, GatewayExternalErrorProjection, GatewayExternalSchema,
    GatewayHttpProtocolSurface, GatewayWebSocketConnectProtocolSurface,
    GatewayWebSocketDownlinkFrame, GatewayWebSocketRpcProfile, GatewayWebSocketShapeVersion,
    PackageCallableParameter, PackageTypeRef, ParamModeIr, TypeRefIr,
};

use super::*;

#[test]
fn runtime_http_gateway_target_fact_mutations_fail_closed() {
    assert!(
        GatewayEntryKey::parse("http typed").is_err(),
        "a non-canonical gateway key cannot enter the typed target"
    );
    let key = GatewayEntryKey::parse("http:typed").unwrap();
    let surface = typed_surface();
    let identity = gateway_entry_identity(&surface).unwrap();
    let plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::TypedJson,
        args: vec![GatewayAdapterArg {
            param: "body".to_string(),
            source: GatewayAdapterSource::HttpBody,
        }],
    };
    let signature = unary_signature("body");

    validate_entry_fact_view(GatewayEntryValidationFacts {
        key: &key,
        identity: &identity,
        surface: &surface,
        plan: &plan,
        handler_signature: &signature,
        pre_signature: None,
        guard_signature: None,
    })
    .expect("canonical gateway facts");

    let wrong_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "0".repeat(64)))
            .unwrap();
    assert!(matches!(
        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &wrong_identity,
            surface: &surface,
            plan: &plan,
            handler_signature: &signature,
            pre_signature: None,
            guard_signature: None,
        }),
        Err(RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity)
    ));

    let mut wrong_mode = surface.clone();
    let GatewayProtocolSurface::Http(http) = &mut wrong_mode.protocol else {
        panic!("typed fixture must use an HTTP surface");
    };
    http.dispatch_mode = GatewayDispatchMode::ServerStream;
    assert!(matches!(
        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &wrong_mode,
            plan: &plan,
            handler_signature: &signature,
            pre_signature: None,
            guard_signature: None,
        }),
        Err(RuntimeAssemblyHttpGatewayTargetError::InvalidProtocolSurface { .. })
            | Err(RuntimeAssemblyHttpGatewayTargetError::InvalidIdentity)
            | Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
    ));

    let mut wrong_kind = plan.clone();
    wrong_kind.kind = GatewayAdapterKind::RawHttp;
    assert!(matches!(
        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &wrong_kind,
            handler_signature: &signature,
            pre_signature: None,
            guard_signature: None,
        }),
        Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
            | Err(RuntimeAssemblyHttpGatewayTargetError::InvalidAdapterPlan { .. })
    ));

    let wrong_signature = unary_signature("other");
    assert!(matches!(
        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &plan,
            handler_signature: &wrong_signature,
            pre_signature: None,
            guard_signature: None,
        }),
        Err(RuntimeAssemblyHttpGatewayTargetError::HandlerPlanMismatch)
    ));
}

#[test]
fn runtime_http_gateway_target_owner_is_exact_deployment_ref() {
    let owner = deployment_ref("revision:a", "identity:a");
    assert!(gateway_owner_matches(&owner, &owner));
    assert!(!gateway_owner_matches(
        &owner,
        &deployment_ref("revision:b", "identity:b")
    ));
}

#[test]
fn runtime_http_gateway_target_refuses_websocket_connect_surface() {
    let key = GatewayEntryKey::parse("websocket").unwrap();
    let surface = websocket_surface();
    let identity = gateway_entry_identity(&surface).unwrap();
    let plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::WebSocketConnect,
        args: vec![GatewayAdapterArg {
            param: "request".to_string(),
            source: GatewayAdapterSource::WebSocketConnectRequest,
        }],
    };
    assert!(matches!(
        validate_entry_fact_view(GatewayEntryValidationFacts {
            key: &key,
            identity: &identity,
            surface: &surface,
            plan: &plan,
            handler_signature: &unary_signature("request"),
            pre_signature: None,
            guard_signature: None,
        }),
        Err(RuntimeAssemblyHttpGatewayTargetError::PlanSurfaceMismatch)
    ));
}

fn typed_surface() -> GatewayEntryProtocolSurface {
    GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(GatewayExternalSchema::String),
            response_schema: Some(GatewayExternalSchema::String),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    }
}

fn websocket_surface() -> GatewayEntryProtocolSurface {
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

fn unary_signature(parameter: &str) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: vec![PackageCallableParameter {
            name: parameter.to_string(),
            ty: PackageTypeRef::Local {
                local_type: TypeRefIr::builtin("string"),
            },
            mode: ParamModeIr::Value,
        }],
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("string"),
        },
        may_suspend: false,
    }
}

fn deployment_ref(revision: &str, identity: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/gateway".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
            identity,
        ),
    }
}

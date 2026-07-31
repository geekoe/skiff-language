use super::*;
use std::collections::BTreeMap;

use skiff_artifact_model::{
    GatewayAdapterArg, GatewayDispatchMode, GatewayExternalErrorProjection, GatewayExternalSchema,
    GatewayWebSocketJsonRpcProtocolSurface, PackageCallableParameter, PackageTypeRef, TypeRefIr,
};

fn surface() -> GatewayEntryProtocolSurface {
    GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketJsonRpc(
            GatewayWebSocketJsonRpcProtocolSurface {
                profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources: vec![GatewayAdapterSource::WebSocketJsonRpcParams],
                params_schema: GatewayExternalSchema::Record {
                    fields: BTreeMap::new(),
                    required: Vec::new(),
                },
                result_schema: GatewayExternalSchema::Null,
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
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend: false,
    }
}

fn selector(method: Option<&str>) -> IngressSelector {
    IngressSelector {
        protocol: IngressProtocol::WebSocket,
        method: method.map(str::to_string),
        path: "/socket".to_string(),
    }
}

#[test]
fn websocket_jsonrpc_target_requires_exact_sibling_surface_handler_and_plan() {
    let key = GatewayEntryKey::parse("status").unwrap();
    let surface = surface();
    let identity = gateway_entry_identity(&surface).unwrap();
    let plan = GatewayAdapterPlan {
        kind: GatewayAdapterKind::WebSocketJsonRpc,
        args: vec![GatewayAdapterArg {
            param: "params".to_string(),
            source: GatewayAdapterSource::WebSocketJsonRpcParams,
        }],
    };
    let canonical_signature = signature("params");

    assert_eq!(
        validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
            key: &key,
            identity: &identity,
            selector: &selector(Some("status.get")),
            physical_selector: &selector(None),
            surface: &surface,
            plan: &plan,
            handler_signature: Some(&canonical_signature),
            has_pre: false,
            has_guard: false,
        })
        .unwrap(),
        GatewayWebSocketRpcProfile::JsonRpc2_0Text
    );

    assert!(matches!(
        validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
            key: &key,
            identity: &identity,
            selector: &IngressSelector {
                path: "/other".to_string(),
                ..selector(Some("status.get"))
            },
            physical_selector: &selector(None),
            surface: &surface,
            plan: &plan,
            handler_signature: Some(&canonical_signature),
            has_pre: false,
            has_guard: false,
        }),
        Err(RuntimeAssemblyWebSocketJsonRpcTargetError::SelectorMismatch)
    ));

    assert!(matches!(
        validate_entry_fact_view(WebSocketJsonRpcEntryValidationFacts {
            key: &key,
            identity: &identity,
            selector: &selector(Some("status.get")),
            physical_selector: &selector(None),
            surface: &surface,
            plan: &plan,
            handler_signature: None,
            has_pre: false,
            has_guard: false,
        }),
        Err(RuntimeAssemblyWebSocketJsonRpcTargetError::HandlerRequired)
    ));
}

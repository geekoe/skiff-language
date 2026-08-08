//! Compiler-owned projection from the strict singleton `websocket.yml`
//! document to one connection entry and zero or more declared JSON-RPC
//! method entries.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{gateway_entry_identity, normalize_gateway_entry_protocol_surface};
use skiff_artifact_model::{
    DeploymentGatewayEntry, DeploymentIngressBinding, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayProtocolSurface,
    GatewayWebSocketConnectProtocolSurface, GatewayWebSocketDownlinkFrame,
    GatewayWebSocketJsonRpcProtocolSurface, GatewayWebSocketRpcProfile,
    GatewayWebSocketShapeVersion, IngressProtocol, IngressSelector, PackageArtifact,
    PackageCallableSignature, PackageSchemaTypeId, PackageSchemaTypeRecord, PackageTypeRef,
    TypeRefIr, WebSocketConnectAuthoring, WebSocketConnectionCloseAuthoring,
    WebSocketGatewayDocumentAuthoring, WebSocketJsonRpcMethodAuthoring,
    WEBSOCKET_CONNECT_REQUEST_V1_TYPE, WEBSOCKET_CONNECT_RESULT_V1_TYPE,
    WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use thiserror::Error;

use crate::http_gateway_projection::{
    resolver::{ExactCallableResolver, ResolvedCallable},
    schema::ExactTypeClassifier,
};

#[derive(Debug, Error)]
pub enum WebSocketGatewayProjectionError {
    #[error("WebSocket gateway field {field} is invalid: {message}")]
    InvalidEntry { field: String, message: String },
}

#[derive(Debug, Default)]
pub(crate) struct ProjectedWebSocketGateway {
    pub gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    pub ingress: Vec<DeploymentIngressBinding>,
}

pub(crate) fn project_websocket_gateway_after_package_validation(
    websocket: Option<&WebSocketGatewayDocumentAuthoring>,
    implementation: &PackageArtifact,
    package_closure: &[PackageArtifact],
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<ProjectedWebSocketGateway, WebSocketGatewayProjectionError> {
    let Some(authoring) = websocket else {
        return Ok(ProjectedWebSocketGateway::default());
    };
    let resolver = ExactCallableResolver::new(implementation);
    let classifier =
        ExactTypeClassifier::new(implementation, package_closure, package_schema_records);
    let mut projected = ProjectedWebSocketGateway::default();

    let connection_key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY)
        .expect("compiler-owned WebSocket gateway key is valid");
    projected.gateway_entries.insert(
        connection_key.clone(),
        project_connection_entry(authoring, &resolver, &classifier)?,
    );
    projected.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: authoring.path.clone(),
        },
        gateway_entry_key: connection_key.clone(),
    });

    for (key, method) in &authoring.json_rpc {
        if key == &connection_key {
            return Err(invalid(
                format!("jsonRpc.{key}"),
                format!(
                    "entry key collides with compiler-owned WebSocket connection key {connection_key}"
                ),
            ));
        }
        let entry = project_json_rpc_entry(key, method, &resolver, &classifier)?;
        if projected
            .gateway_entries
            .insert(key.clone(), entry)
            .is_some()
        {
            return Err(invalid(format!("jsonRpc.{key}"), "entry key is not unique"));
        }
        projected.ingress.push(DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::WebSocket,
                method: Some(method.method.clone()),
                path: authoring.path.clone(),
            },
            gateway_entry_key: key.clone(),
        });
    }
    Ok(projected)
}

fn project_connection_entry(
    authoring: &WebSocketGatewayDocumentAuthoring,
    resolver: &ExactCallableResolver<'_>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<DeploymentGatewayEntry, WebSocketGatewayProjectionError> {
    let (handler, args) = authoring
        .connect
        .as_ref()
        .map(|connect| project_connect(connect, resolver, classifier))
        .transpose()?
        .map_or_else(
            || (None, Vec::new()),
            |(handler, args)| (Some(handler), args),
        );
    let (close_handler, close_args) = authoring
        .close
        .as_ref()
        .map(|close| project_close(close, resolver, classifier))
        .transpose()?
        .map_or_else(
            || (None, Vec::new()),
            |(handler, args)| (Some(handler), args),
        );
    let mut close_external_sources = close_args
        .iter()
        .map(|argument| argument.source)
        .collect::<Vec<_>>();
    close_external_sources.sort_by_key(|source| source.wire_name());
    close_external_sources.dedup();

    let surface = normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketConnect(
            GatewayWebSocketConnectProtocolSurface {
                connect_request_shape: GatewayWebSocketShapeVersion::V1,
                connect_result_shape: GatewayWebSocketShapeVersion::V1,
                connection_policy_shape: GatewayWebSocketShapeVersion::V1,
                external_sources: vec![
                    GatewayAdapterSource::WebSocketConnectRequest,
                    GatewayAdapterSource::WebSocketConnectionId,
                ],
                downlink_frames: vec![
                    GatewayWebSocketDownlinkFrame::Text,
                    GatewayWebSocketDownlinkFrame::Binary,
                ],
                rpc_profiles: vec![GatewayWebSocketRpcProfile::JsonRpc2_0Text],
                connection_close_shape: GatewayWebSocketShapeVersion::V1,
                close_external_sources,
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .map_err(|error| invalid("protocolSurface", error.to_string()))?;
    let gateway_entry_identity = gateway_entry_identity(&surface)
        .map_err(|error| invalid("gatewayEntryIdentity", error.to_string()))?;
    Ok(DeploymentGatewayEntry {
        gateway_entry_identity,
        protocol_surface: surface,
        handler,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args,
        },
        close_handler,
        close_adapter_plan: authoring.close.is_some().then(|| GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnectionClosed,
            args: close_args,
        }),
    })
}

fn project_connect(
    authoring: &WebSocketConnectAuthoring,
    resolver: &ExactCallableResolver<'_>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<
    (
        skiff_artifact_model::PackageCallableId,
        Vec<skiff_artifact_model::GatewayAdapterArg>,
    ),
    WebSocketGatewayProjectionError,
> {
    let callable = resolver
        .resolve(&authoring.handler)
        .map_err(|message| invalid("connect.handler", message))?;
    reject_generic("connect.handler", &callable)?;
    validate_connect_args(authoring, &callable.signature, classifier)?;
    classifier
        .require_std_websocket_type(
            &callable.signature.return_type,
            WEBSOCKET_CONNECT_RESULT_V1_TYPE,
        )
        .map_err(|message| invalid("connect.handler", message))?;
    Ok((callable.callable_id, authoring.adapter_args.clone()))
}

fn project_close(
    authoring: &WebSocketConnectionCloseAuthoring,
    resolver: &ExactCallableResolver<'_>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<
    (
        skiff_artifact_model::PackageCallableId,
        Vec<skiff_artifact_model::GatewayAdapterArg>,
    ),
    WebSocketGatewayProjectionError,
> {
    let callable = resolver
        .resolve(&authoring.handler)
        .map_err(|message| invalid("close.handler", message))?;
    reject_generic("close.handler", &callable)?;
    validate_close_args(authoring, &callable.signature, classifier)?;
    classifier
        .require_builtin_type(&callable.signature.return_type, "void")
        .map_err(|message| invalid("close.handler", message))?;
    Ok((callable.callable_id, authoring.adapter_args.clone()))
}

fn project_json_rpc_entry(
    key: &GatewayEntryKey,
    authoring: &WebSocketJsonRpcMethodAuthoring,
    resolver: &ExactCallableResolver<'_>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<DeploymentGatewayEntry, WebSocketGatewayProjectionError> {
    let label = format!("jsonRpc.{key}");
    let callable = resolver
        .resolve(&authoring.handler)
        .map_err(|message| invalid(format!("{label}.handler"), message))?;
    reject_generic(&format!("{label}.handler"), &callable)?;
    let params_schema = validate_json_rpc_args(&label, authoring, &callable.signature, classifier)?;
    let result_schema =
        project_json_rpc_return(&label, &callable.signature.return_type, classifier)?;
    let mut external_sources = authoring
        .adapter_args
        .iter()
        .map(|argument| argument.source)
        .collect::<Vec<_>>();
    external_sources.sort_by_key(|source| source.wire_name());
    external_sources.dedup();
    let surface = normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::WebSocketJsonRpc(
            GatewayWebSocketJsonRpcProtocolSurface {
                profile: GatewayWebSocketRpcProfile::JsonRpc2_0Text,
                dispatch_mode: GatewayDispatchMode::Unary,
                external_sources,
                params_schema,
                result_schema,
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .map_err(|error| invalid(format!("{label}.protocolSurface"), error.to_string()))?;
    let gateway_entry_identity = gateway_entry_identity(&surface)
        .map_err(|error| invalid(format!("{label}.gatewayEntryIdentity"), error.to_string()))?;
    Ok(DeploymentGatewayEntry {
        gateway_entry_identity,
        protocol_surface: surface,
        handler: Some(callable.callable_id),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketJsonRpc,
            args: authoring.adapter_args.clone(),
        },
        close_handler: None,
        close_adapter_plan: None,
    })
}

fn reject_generic(
    field: &str,
    callable: &ResolvedCallable,
) -> Result<(), WebSocketGatewayProjectionError> {
    if callable.signature.type_params.is_empty() {
        return Ok(());
    }
    Err(invalid(
        field,
        format!(
            "{} declares generic parameters {:?}",
            callable.selector, callable.signature.type_params
        ),
    ))
}

fn validate_connect_args(
    authoring: &WebSocketConnectAuthoring,
    signature: &PackageCallableSignature,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<(), WebSocketGatewayProjectionError> {
    validate_exact_formal_coverage("connect.adapterArgs", &authoring.adapter_args, signature)?;
    let formals = signature
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    for arg in &authoring.adapter_args {
        let ty = &formals[arg.param.as_str()].ty;
        match arg.source {
            GatewayAdapterSource::WebSocketConnectRequest => classifier
                .require_std_websocket_type(ty, WEBSOCKET_CONNECT_REQUEST_V1_TYPE)
                .map_err(|message| invalid("connect.adapterArgs", message))?,
            GatewayAdapterSource::WebSocketConnectionId => classifier
                .require_builtin_type(ty, "string")
                .map_err(|message| invalid("connect.adapterArgs", message))?,
            GatewayAdapterSource::HttpRequest
            | GatewayAdapterSource::HttpBody
            | GatewayAdapterSource::HttpContext
            | GatewayAdapterSource::WebSocketJsonRpcParams
            | GatewayAdapterSource::WebSocketBusinessIdentity
            | GatewayAdapterSource::WebSocketCloseCode
            | GatewayAdapterSource::WebSocketCloseReason => {
                return Err(invalid(
                    "connect.adapterArgs",
                    format!(
                        "source {} is not allowed for websocketConnect",
                        arg.source.wire_name()
                    ),
                ))
            }
        }
    }
    Ok(())
}

fn validate_close_args(
    authoring: &WebSocketConnectionCloseAuthoring,
    signature: &PackageCallableSignature,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<(), WebSocketGatewayProjectionError> {
    let field = "close.adapterArgs";
    validate_exact_formal_coverage(field, &authoring.adapter_args, signature)?;
    let formals = signature
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    let mut counts = BTreeMap::new();
    for arg in &authoring.adapter_args {
        let ty = &formals[arg.param.as_str()].ty;
        match arg.source {
            GatewayAdapterSource::WebSocketConnectionId => {
                classifier
                    .require_builtin_type(ty, "string")
                    .map_err(|message| invalid(field, message))?;
            }
            GatewayAdapterSource::WebSocketCloseCode => {
                classifier
                    .require_builtin_type(ty, "integer")
                    .map_err(|message| invalid(field, message))?;
            }
            GatewayAdapterSource::WebSocketCloseReason => {
                classifier
                    .require_builtin_type(ty, "string")
                    .map_err(|message| invalid(field, message))?;
            }
            GatewayAdapterSource::WebSocketBusinessIdentity => {
                classifier
                    .require_nullable_builtin_type(ty, "string")
                    .map_err(|message| invalid(field, message))?;
            }
            GatewayAdapterSource::HttpRequest
            | GatewayAdapterSource::HttpBody
            | GatewayAdapterSource::HttpContext
            | GatewayAdapterSource::WebSocketConnectRequest
            | GatewayAdapterSource::WebSocketJsonRpcParams => {
                return Err(invalid(
                    field,
                    format!(
                        "source {} is not allowed for websocketConnectionClosed",
                        arg.source.wire_name()
                    ),
                ))
            }
        }
        *counts.entry(arg.source).or_insert(0usize) += 1;
    }
    for (source, expected) in [
        (
            GatewayAdapterSource::WebSocketConnectionId,
            "websocket.connectionId may be bound at most once",
        ),
        (
            GatewayAdapterSource::WebSocketCloseCode,
            "websocket.closeCode may be bound at most once",
        ),
        (
            GatewayAdapterSource::WebSocketCloseReason,
            "websocket.closeReason may be bound at most once",
        ),
        (
            GatewayAdapterSource::WebSocketBusinessIdentity,
            "websocket.businessIdentity may be bound at most once",
        ),
    ] {
        if counts.get(&source).copied().unwrap_or(0) > 1 {
            return Err(invalid(field, expected));
        }
    }
    Ok(())
}

fn validate_json_rpc_args(
    label: &str,
    authoring: &WebSocketJsonRpcMethodAuthoring,
    signature: &PackageCallableSignature,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<GatewayExternalSchema, WebSocketGatewayProjectionError> {
    let field = format!("{label}.adapterArgs");
    validate_exact_formal_coverage(&field, &authoring.adapter_args, signature)?;
    let formals = signature
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    let mut params_schema = None;
    let mut connection_count = 0usize;
    let mut business_identity_count = 0usize;
    for arg in &authoring.adapter_args {
        let ty = &formals[arg.param.as_str()].ty;
        match arg.source {
            GatewayAdapterSource::WebSocketJsonRpcParams => {
                if params_schema.is_some() {
                    return Err(invalid(
                        &field,
                        "websocket.jsonRpcParams must be bound exactly once",
                    ));
                }
                let schema = classifier
                    .project(ty)
                    .map_err(|message| invalid(&field, message))?;
                if !json_rpc_params_are_structured(&schema) {
                    return Err(invalid(
                        &field,
                        "websocket.jsonRpcParams must project to a top-level object or array",
                    ));
                }
                params_schema = Some(schema);
            }
            GatewayAdapterSource::WebSocketConnectionId => {
                connection_count += 1;
                classifier
                    .require_builtin_type(ty, "string")
                    .map_err(|message| invalid(&field, message))?;
            }
            GatewayAdapterSource::WebSocketBusinessIdentity => {
                business_identity_count += 1;
                classifier
                    .require_nullable_builtin_type(ty, "string")
                    .map_err(|message| invalid(&field, message))?;
            }
            GatewayAdapterSource::HttpRequest
            | GatewayAdapterSource::HttpBody
            | GatewayAdapterSource::HttpContext
            | GatewayAdapterSource::WebSocketConnectRequest
            | GatewayAdapterSource::WebSocketCloseCode
            | GatewayAdapterSource::WebSocketCloseReason => {
                return Err(invalid(
                    &field,
                    format!(
                        "source {} is not allowed for websocketJsonRpc",
                        arg.source.wire_name()
                    ),
                ))
            }
        }
    }
    if connection_count > 1 {
        return Err(invalid(
            &field,
            "websocket.connectionId may be bound at most once",
        ));
    }
    if business_identity_count > 1 {
        return Err(invalid(
            &field,
            "websocket.businessIdentity may be bound at most once",
        ));
    }
    params_schema
        .ok_or_else(|| invalid(field, "websocket.jsonRpcParams must be bound exactly once"))
}

fn json_rpc_params_are_structured(schema: &GatewayExternalSchema) -> bool {
    match schema {
        GatewayExternalSchema::Record { .. } | GatewayExternalSchema::Array { .. } => true,
        GatewayExternalSchema::ClosedUnion { branches } => {
            !branches.is_empty() && branches.iter().all(json_rpc_params_are_structured)
        }
        GatewayExternalSchema::Null
        | GatewayExternalSchema::String
        | GatewayExternalSchema::Number
        | GatewayExternalSchema::Integer
        | GatewayExternalSchema::Boolean
        | GatewayExternalSchema::Bytes
        | GatewayExternalSchema::StringLiteral { .. }
        | GatewayExternalSchema::Nullable { .. } => false,
    }
}

fn validate_exact_formal_coverage(
    field: &str,
    args: &[skiff_artifact_model::GatewayAdapterArg],
    signature: &PackageCallableSignature,
) -> Result<(), WebSocketGatewayProjectionError> {
    let expected_order = signature
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    let actual_order = args
        .iter()
        .map(|argument| argument.param.as_str())
        .collect::<Vec<_>>();
    let expected = expected_order.iter().copied().collect::<BTreeSet<_>>();
    let actual = actual_order.iter().copied().collect::<BTreeSet<_>>();
    if expected.len() != signature.parameters.len() {
        return Err(invalid(
            field,
            "handler signature repeats a formal parameter name",
        ));
    }
    if actual_order != expected_order || actual.len() != args.len() {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unknown = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(invalid(
            field,
            format!(
                "adapter args must cover every handler formal exactly once in signature order; expected={expected_order:?}, actual={actual_order:?}, missing={missing:?}, unknown={unknown:?}"
            ),
        ));
    }
    Ok(())
}

fn project_json_rpc_return(
    label: &str,
    return_type: &PackageTypeRef,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<GatewayExternalSchema, WebSocketGatewayProjectionError> {
    let field = format!("{label}.handler");
    if is_stream(return_type) {
        return Err(invalid(
            field,
            "websocketJsonRpc supports only unary handler returns",
        ));
    }
    if classifier.require_builtin_type(return_type, "void").is_ok() {
        return Ok(GatewayExternalSchema::Null);
    }
    classifier
        .project(return_type)
        .map_err(|message| invalid(field, message))
}

fn is_stream(ty: &PackageTypeRef) -> bool {
    matches!(
        ty,
        PackageTypeRef::Container { name, .. } if name == "Stream"
    ) || matches!(
        ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, .. },
        } if name == "Stream"
    )
}

fn invalid(
    field: impl Into<String>,
    message: impl Into<String>,
) -> WebSocketGatewayProjectionError {
    WebSocketGatewayProjectionError::InvalidEntry {
        field: field.into(),
        message: message.into(),
    }
}

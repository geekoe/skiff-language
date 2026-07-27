//! Compiler-owned projection from the strict singleton WebSocket authoring
//! shape to one connect-only deployment gateway entry.

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{gateway_entry_identity, normalize_gateway_entry_protocol_surface};
use skiff_artifact_model::{
    DeploymentGatewayEntry, DeploymentIngressBinding, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayProtocolSurface, GatewayWebSocketConnectProtocolSurface,
    GatewayWebSocketDownlinkFrame, GatewayWebSocketShapeVersion, IngressProtocol, IngressSelector,
    PackageArtifact, PackageCallableSignature, PackageSchemaTypeId, PackageSchemaTypeRecord,
    ServiceManifestAuthoring, WebSocketConnectAuthoring, WEBSOCKET_CONNECT_REQUEST_V1_TYPE,
    WEBSOCKET_CONNECT_RESULT_V1_TYPE, WEBSOCKET_GATEWAY_ENTRY_KEY,
};
use thiserror::Error;

use crate::http_gateway_projection::{
    resolver::{ExactCallableResolver, ResolvedCallable},
    schema::ExactTypeClassifier,
};

#[derive(Debug, Error)]
pub enum WebSocketGatewayProjectionError {
    #[error("WebSocket gateway field {field} is invalid: {message}")]
    InvalidEntry {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct ProjectedWebSocketGateway {
    pub gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    pub ingress: Vec<DeploymentIngressBinding>,
}

pub(crate) fn project_websocket_gateway(
    service: &ServiceManifestAuthoring,
    implementation: &PackageArtifact,
    package_closure: &[PackageArtifact],
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<ProjectedWebSocketGateway, WebSocketGatewayProjectionError> {
    let Some(authoring) = service.websocket.as_ref() else {
        return Ok(ProjectedWebSocketGateway::default());
    };
    for artifact in package_closure
        .iter()
        .chain(std::iter::once(implementation))
    {
        skiff_artifact_identity::validate_package_artifact_identities(artifact)
            .map_err(|error| invalid("packageArtifact", error.to_string()))?;
    }
    skiff_artifact_identity::validate_package_schema_records(package_schema_records)
        .map_err(|error| invalid("packageSchemaRecords", error.to_string()))?;

    let resolver = ExactCallableResolver::new(implementation);
    let classifier =
        ExactTypeClassifier::new(implementation, package_closure, package_schema_records);
    let (handler, args) = authoring
        .connect
        .as_ref()
        .map(|connect| project_connect(connect, &resolver, &classifier))
        .transpose()?
        .map_or_else(
            || (None, Vec::new()),
            |(handler, args)| (Some(handler), args),
        );

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
            },
        ),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .map_err(|error| invalid("protocolSurface", error.to_string()))?;
    let gateway_entry_identity = gateway_entry_identity(&surface)
        .map_err(|error| invalid("gatewayEntryIdentity", error.to_string()))?;
    let key = GatewayEntryKey::parse(WEBSOCKET_GATEWAY_ENTRY_KEY)
        .expect("compiler-owned WebSocket gateway key is valid");
    let entry = DeploymentGatewayEntry {
        gateway_entry_identity,
        protocol_surface: surface,
        handler,
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::WebSocketConnect,
            args,
        },
    };
    let binding = DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            host: authoring.host.clone(),
            method: None,
            path: authoring.path.clone(),
        },
        gateway_entry_key: key.clone(),
    };
    Ok(ProjectedWebSocketGateway {
        gateway_entries: BTreeMap::from([(key, entry)]),
        ingress: vec![binding],
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
    reject_generic(&callable)?;
    validate_connect_args(authoring, &callable.signature, classifier)?;
    classifier
        .require_std_websocket_type(
            &callable.signature.return_type,
            WEBSOCKET_CONNECT_RESULT_V1_TYPE,
        )
        .map_err(|message| invalid("connect.handler", message))?;
    Ok((callable.callable_id, authoring.adapter_args.clone()))
}

fn reject_generic(callable: &ResolvedCallable) -> Result<(), WebSocketGatewayProjectionError> {
    if callable.signature.type_params.is_empty() {
        return Ok(());
    }
    Err(invalid(
        "connect.handler",
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
    let formals = signature
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    if formals.len() != signature.parameters.len() {
        return Err(invalid(
            "connect.handler",
            "handler signature repeats a formal parameter name",
        ));
    }
    let actual_order = authoring
        .adapter_args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<Vec<_>>();
    let expected_order = signature
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    let actual = actual_order.iter().copied().collect::<BTreeSet<_>>();
    let expected = expected_order.iter().copied().collect::<BTreeSet<_>>();
    if actual_order != expected_order || actual.len() != authoring.adapter_args.len() {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unknown = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(invalid(
            "connect.adapterArgs",
            format!(
                "adapter args must cover every handler formal exactly once in signature order; expected={expected_order:?}, actual={actual_order:?}, missing={missing:?}, unknown={unknown:?}"
            ),
        ));
    }

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
            | GatewayAdapterSource::HttpContext => {
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

fn invalid(field: &'static str, message: impl Into<String>) -> WebSocketGatewayProjectionError {
    WebSocketGatewayProjectionError::InvalidEntry {
        field,
        message: message.into(),
    }
}

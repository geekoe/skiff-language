//! Compiler-owned projection from strict HTTP authoring to deployment gateway facts.
//!
//! This leaf deliberately consumes the exact implementation ABI rather than
//! widening private ingress callables into the package public surface.

pub(crate) mod resolver;
pub(crate) mod schema;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{gateway_entry_identity, normalize_gateway_entry_protocol_surface};
use skiff_artifact_model::{
    DeploymentGatewayEntry, DeploymentIngressBinding, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryKey, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, HttpGatewayEntryAuthoring, IngressProtocol, IngressSelector,
    PackageArtifact, PackageCallableSignature, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageTypeRef, ServiceManifestAuthoring,
};
use thiserror::Error;

use resolver::{ExactCallableResolver, ResolvedCallable};
use schema::ExactTypeClassifier;

#[derive(Debug, Error)]
pub enum HttpGatewayProjectionError {
    #[error("HTTP gateway entry {entry} field {field} is invalid: {message}")]
    InvalidEntry {
        entry: String,
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Default)]
pub(crate) struct ProjectedHttpGateway {
    pub gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    pub ingress: Vec<DeploymentIngressBinding>,
}

pub(crate) fn project_http_gateway(
    service: &ServiceManifestAuthoring,
    implementation: &PackageArtifact,
    package_closure: &[PackageArtifact],
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<ProjectedHttpGateway, HttpGatewayProjectionError> {
    let Some(entries) = service.http.as_ref() else {
        return Ok(ProjectedHttpGateway::default());
    };
    for artifact in package_closure
        .iter()
        .chain(std::iter::once(implementation))
    {
        skiff_artifact_identity::validate_package_artifact_identities(artifact).map_err(
            |error| HttpGatewayProjectionError::InvalidEntry {
                entry: "<package-closure>".to_string(),
                field: "packageArtifact",
                message: error.to_string(),
            },
        )?;
    }
    skiff_artifact_identity::validate_package_schema_records(package_schema_records).map_err(
        |error| HttpGatewayProjectionError::InvalidEntry {
            entry: "<schema-closure>".to_string(),
            field: "packageSchemaRecords",
            message: error.to_string(),
        },
    )?;
    let resolver = ExactCallableResolver::new(implementation);
    let classifier =
        ExactTypeClassifier::new(implementation, package_closure, package_schema_records);
    let mut projected = ProjectedHttpGateway::default();
    for (key, authoring) in entries {
        let entry = project_entry(key, authoring, &resolver, &classifier)?;
        let binding = DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: authoring.host.clone(),
                method: Some(authoring.method.clone()),
                path: authoring.path.clone(),
            },
            gateway_entry_key: key.clone(),
        };
        projected.gateway_entries.insert(key.clone(), entry);
        projected.ingress.push(binding);
    }
    Ok(projected)
}

fn project_entry(
    key: &GatewayEntryKey,
    authoring: &HttpGatewayEntryAuthoring,
    resolver: &ExactCallableResolver<'_>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<DeploymentGatewayEntry, HttpGatewayProjectionError> {
    let handler = resolve(key, "handler", &authoring.handler, resolver)?;
    reject_generic(key, "handler", &handler)?;
    let guard = authoring
        .guard
        .as_deref()
        .map(|selector| resolve(key, "guard", selector, resolver))
        .transpose()?;
    if let Some(guard) = &guard {
        reject_generic(key, "guard", guard)?;
        validate_guard(key, guard, classifier)?;
    }
    let pre = authoring
        .pre
        .as_deref()
        .map(|selector| resolve(key, "pre", selector, resolver))
        .transpose()?;
    if let Some(pre) = &pre {
        reject_generic(key, "pre", pre)?;
        validate_pre(key, pre, classifier)?;
    }

    let sources =
        validate_handler_args(key, authoring, &handler.signature, pre.as_ref(), classifier)?;
    let (dispatch_mode, response_schema, stream_item_schema) = project_handler_return(
        key,
        authoring.kind,
        &handler.signature.return_type,
        classifier,
    )?;
    let request_body_schema = body_schema(key, authoring, &handler.signature, classifier)?;
    let surface = normalize_gateway_entry_protocol_surface(GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: authoring.kind,
            dispatch_mode,
            external_sources: sources,
            request_body_schema,
            response_schema,
            stream_item_schema,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    })
    .map_err(|error| invalid(key, "protocolSurface", error.to_string()))?;
    let gateway_entry_identity = gateway_entry_identity(&surface)
        .map_err(|error| invalid(key, "gatewayEntryIdentity", error.to_string()))?;
    Ok(DeploymentGatewayEntry {
        gateway_entry_identity,
        protocol_surface: surface,
        handler: Some(handler.callable_id),
        pre: pre.map(|callable| callable.callable_id),
        guard: guard.map(|callable| callable.callable_id),
        adapter_plan: GatewayAdapterPlan {
            kind: authoring.kind,
            args: authoring.adapter_args.clone(),
        },
    })
}

fn resolve(
    key: &GatewayEntryKey,
    field: &'static str,
    selector: &str,
    resolver: &ExactCallableResolver<'_>,
) -> Result<ResolvedCallable, HttpGatewayProjectionError> {
    resolver
        .resolve(selector)
        .map_err(|message| invalid(key, field, message))
}

fn reject_generic(
    key: &GatewayEntryKey,
    field: &'static str,
    callable: &ResolvedCallable,
) -> Result<(), HttpGatewayProjectionError> {
    if callable.signature.type_params.is_empty() {
        return Ok(());
    }
    Err(invalid(
        key,
        field,
        format!(
            "{} declares generic parameters {:?}",
            callable.selector, callable.signature.type_params
        ),
    ))
}

fn validate_guard(
    key: &GatewayEntryKey,
    guard: &ResolvedCallable,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<(), HttpGatewayProjectionError> {
    let [parameter] = guard.signature.parameters.as_slice() else {
        return Err(invalid(
            key,
            "guard",
            "guard must declare exactly one HttpRequest parameter",
        ));
    };
    classifier
        .require_std_http_type(&parameter.ty, "std.http.HttpRequest")
        .map_err(|message| invalid(key, "guard", message))?;
    let Some(inner) = nullable_inner(&guard.signature.return_type) else {
        return Err(invalid(
            key,
            "guard",
            "guard must return exact std.http.HttpResponse?",
        ));
    };
    classifier
        .require_std_http_exact(inner, "std.http.HttpResponse")
        .map_err(|message| invalid(key, "guard", message))
}

fn validate_pre(
    key: &GatewayEntryKey,
    pre: &ResolvedCallable,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<(), HttpGatewayProjectionError> {
    let [parameter] = pre.signature.parameters.as_slice() else {
        return Err(invalid(
            key,
            "pre",
            "pre must declare exactly one HttpRequest parameter",
        ));
    };
    classifier
        .require_std_http_type(&parameter.ty, "std.http.HttpRequest")
        .map_err(|message| invalid(key, "pre", message))
}

fn validate_handler_args(
    key: &GatewayEntryKey,
    authoring: &HttpGatewayEntryAuthoring,
    signature: &PackageCallableSignature,
    pre: Option<&ResolvedCallable>,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<Vec<GatewayAdapterSource>, HttpGatewayProjectionError> {
    let formals = signature
        .parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    if formals.len() != signature.parameters.len() {
        return Err(invalid(
            key,
            "handler",
            "handler signature repeats a formal parameter name",
        ));
    }
    let actual = authoring
        .adapter_args
        .iter()
        .map(|arg| arg.param.as_str())
        .collect::<BTreeSet<_>>();
    let expected = formals.keys().copied().collect::<BTreeSet<_>>();
    if actual != expected || actual.len() != authoring.adapter_args.len() {
        let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
        let unknown = actual.difference(&expected).copied().collect::<Vec<_>>();
        return Err(invalid(
            key,
            "adapterArgs",
            format!(
                "adapter args must cover every handler formal exactly once; missing={missing:?}, unknown={unknown:?}"
            ),
        ));
    }

    let mut source_types: BTreeMap<
        GatewayAdapterSource,
        (&PackageTypeRef, Option<GatewayExternalSchema>),
    > = BTreeMap::new();
    for arg in &authoring.adapter_args {
        let ty = &formals[arg.param.as_str()].ty;
        let schema = match arg.source {
            GatewayAdapterSource::HttpRequest => {
                classifier
                    .require_std_http_type(ty, "std.http.HttpRequest")
                    .map_err(|message| invalid(key, "adapterArgs", message))?;
                None
            }
            GatewayAdapterSource::HttpBody => Some(
                classifier
                    .project(ty)
                    .map_err(|message| invalid(key, "adapterArgs", message))?,
            ),
            GatewayAdapterSource::HttpContext => {
                let pre = pre.ok_or_else(|| {
                    invalid(
                        key,
                        "adapterArgs",
                        "http.context requires an entry-local pre",
                    )
                })?;
                if ty != &pre.signature.return_type {
                    return Err(invalid(
                        key,
                        "adapterArgs",
                        format!(
                            "http.context formal {} does not exactly match pre return type",
                            arg.param
                        ),
                    ));
                }
                None
            }
            GatewayAdapterSource::WebSocketConnectRequest
            | GatewayAdapterSource::WebSocketConnectionId => {
                return Err(invalid(
                    key,
                    "adapterArgs",
                    format!(
                        "source {} is not allowed for an HTTP gateway entry",
                        arg.source.wire_name()
                    ),
                ))
            }
        };
        if let Some((existing_type, existing_schema)) = source_types.get(&arg.source) {
            if *existing_type != ty || *existing_schema != schema {
                return Err(invalid(
                    key,
                    "adapterArgs",
                    format!(
                        "source {:?} is bound to incompatible exact formal types or schemas",
                        arg.source
                    ),
                ));
            }
        } else {
            source_types.insert(arg.source, (ty, schema));
        }
    }
    Ok(source_types
        .keys()
        .copied()
        .filter(|source| *source != GatewayAdapterSource::HttpContext)
        .collect())
}

fn body_schema(
    key: &GatewayEntryKey,
    authoring: &HttpGatewayEntryAuthoring,
    signature: &PackageCallableSignature,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<Option<GatewayExternalSchema>, HttpGatewayProjectionError> {
    let body_args = authoring
        .adapter_args
        .iter()
        .filter(|arg| arg.source == GatewayAdapterSource::HttpBody)
        .collect::<Vec<_>>();
    match authoring.kind {
        GatewayAdapterKind::TypedJson if body_args.is_empty() => Err(invalid(
            key,
            "adapterArgs",
            "typedJson requires at least one http.body formal",
        )),
        GatewayAdapterKind::RawHttp if !body_args.is_empty() => Err(invalid(
            key,
            "adapterArgs",
            "rawHttp cannot consume http.body",
        )),
        GatewayAdapterKind::RawHttp => Ok(None),
        GatewayAdapterKind::WebSocketConnect => Err(invalid(
            key,
            "kind",
            "websocketConnect is not an HTTP adapter kind",
        )),
        GatewayAdapterKind::TypedJson => {
            let by_name = signature
                .parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), &parameter.ty))
                .collect::<BTreeMap<_, _>>();
            let mut schemas = body_args
                .iter()
                .map(|arg| {
                    classifier
                        .project(by_name[arg.param.as_str()])
                        .map_err(|message| invalid(key, "adapterArgs", message))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let first = schemas.remove(0);
            if schemas.iter().any(|schema| schema != &first) {
                return Err(invalid(
                    key,
                    "adapterArgs",
                    "http.body formals do not project to one canonical external schema",
                ));
            }
            Ok(Some(first))
        }
    }
}

fn project_handler_return(
    key: &GatewayEntryKey,
    kind: GatewayAdapterKind,
    return_type: &PackageTypeRef,
    classifier: &ExactTypeClassifier<'_>,
) -> Result<
    (
        GatewayDispatchMode,
        Option<GatewayExternalSchema>,
        Option<GatewayExternalSchema>,
    ),
    HttpGatewayProjectionError,
> {
    if let Some(item) =
        stream_item(return_type).map_err(|message| invalid(key, "handler", message))?
    {
        return match kind {
            GatewayAdapterKind::TypedJson => Err(invalid(
                key,
                "handler",
                "typedJson supports only unary handler returns; HTTP streaming requires rawHttp + Stream<std.http.HttpResponseStreamEvent>",
            )),
            GatewayAdapterKind::RawHttp => {
                classifier
                    .require_std_http_exact(item, "std.http.HttpResponseStreamEvent")
                    .map_err(|message| invalid(key, "handler", message))?;
                Ok((
                    GatewayDispatchMode::ServerStream,
                    None,
                    Some(
                        classifier
                            .canonical_std_http_schema("std.http.HttpResponseStreamEvent")
                            .map_err(|message| invalid(key, "handler", message))?,
                    ),
                ))
            }
            GatewayAdapterKind::WebSocketConnect => Err(invalid(
                key,
                "kind",
                "websocketConnect is not an HTTP adapter kind",
            )),
        };
    }
    match kind {
        GatewayAdapterKind::TypedJson => Ok((
            GatewayDispatchMode::Unary,
            Some(
                classifier
                    .project(return_type)
                    .map_err(|message| invalid(key, "handler", message))?,
            ),
            None,
        )),
        GatewayAdapterKind::RawHttp => {
            classifier
                .require_std_http_type(return_type, "std.http.HttpResponse")
                .map_err(|message| invalid(key, "handler", message))?;
            Ok((GatewayDispatchMode::Unary, None, None))
        }
        GatewayAdapterKind::WebSocketConnect => Err(invalid(
            key,
            "kind",
            "websocketConnect is not an HTTP adapter kind",
        )),
    }
}

fn stream_item(ty: &PackageTypeRef) -> Result<Option<schema::ExactTypeRef<'_>>, String> {
    match ty {
        PackageTypeRef::Container { name, arguments } if name == "Stream" => {
            let [item] = arguments.as_slice() else {
                return Err("Stream return must have exactly one item type".to_string());
            };
            Ok(Some(schema::ExactTypeRef::Package(item)))
        }
        PackageTypeRef::Local {
            local_type: skiff_artifact_model::TypeRefIr::Builtin { name, args },
        } if name == "Stream" => {
            let [item] = args.as_slice() else {
                return Err("Stream return must have exactly one item type".to_string());
            };
            Ok(Some(schema::ExactTypeRef::Local(item)))
        }
        _ => Ok(None),
    }
}

fn nullable_inner(ty: &PackageTypeRef) -> Option<schema::ExactTypeRef<'_>> {
    match ty {
        PackageTypeRef::Nullable { inner } => Some(schema::ExactTypeRef::Package(inner)),
        PackageTypeRef::Local {
            local_type: skiff_artifact_model::TypeRefIr::Nullable { inner },
        } => Some(schema::ExactTypeRef::Local(inner)),
        _ => None,
    }
}

fn invalid(
    key: &GatewayEntryKey,
    field: &'static str,
    message: impl Into<String>,
) -> HttpGatewayProjectionError {
    HttpGatewayProjectionError::InvalidEntry {
        entry: key.as_str().to_string(),
        field,
        message: message.into(),
    }
}

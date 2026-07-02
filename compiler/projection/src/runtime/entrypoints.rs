use std::collections::{BTreeMap, BTreeSet};

use crate::{
    contract::{ContractProjection, ContractProjectionIndex},
    error::ProjectionError,
    runtime_manifest_model::ArtifactOperation,
    runtime_manifest_model::{
        http_ingress_identity, RuntimeGatewayAdapterArgManifest,
        RuntimeGatewayAdapterSourceManifest, RuntimeHttpRawGatewayManifest,
        RuntimeHttpRouteAdapterCallableManifest, RuntimeHttpRouteAdapterKind,
        RuntimeHttpRouteAdapterManifest, RuntimeHttpRouteGatewayManifest,
        RuntimeHttpRouteHandlerManifest, RuntimeHttpRouteTypedBodyManifest,
        RuntimeHttpRouteTypedManifest, RuntimeHttpRouteTypedResponseManifest,
        RuntimeOperationManifest, RuntimeOperationParameter,
    },
    typed_artifacts::public_function_operation_abi_id,
    {
        WebSocketContextProjectionConfig, WebSocketGatewayProjectionConfig,
        WebSocketOperationProjectionConfig,
    },
};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, FunctionTypeParamIr, InterfaceInstantiationRef,
    OperationAbiRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::{
    id::SKIFF_STD_PUBLICATION_ID,
    package_export_resolver::package_public_path,
    type_syntax::{generic_parts, GenericParts},
};
use skiff_compiler_projection_input::{
    EntryFunctionSignature, EntryParamSpec, EntryTypeSpec, PackageAbiType, PackageProjectionInput,
    ProjectionEntrypointAbiIndex, ProjectionSyntheticEntrypointExecutableKind,
    ProjectionSyntheticEntrypointIndex, ProjectionSyntheticEntrypointModule, ProjectionView,
    ServiceHttpRouteIngressProjection, ServiceIngressHandlerProjection, ServiceIngressProjection,
    ServiceWebSocketIngressProjection,
};

use super::{
    entry_function_type_ref_source_text, entry_type_source_text_with_named_types,
    is_connection_message_type, is_gateway_connect_result_type, is_http_request_type,
    is_http_response_stream_event_type, is_http_response_type, is_nullable_http_response_type,
    is_websocket_connect_request_type, is_websocket_receive_event_root, normalize_type_name,
    package_runtime_schema_for_type_ref, package_runtime_schema_for_type_spec, response_type_ir,
};

#[derive(Debug, Clone)]
pub struct EntryOperationSpec {
    pub operation: String,
    pub target: String,
    pub implementation_module: String,
    pub callable: EntryOperationCallable,
    pub params: Vec<EntryParamSpec>,
    pub return_type: EntryTypeSpec,
}

#[derive(Debug, Clone)]
pub enum EntryOperationCallable {
    ImplMethod { type_name: String, method: String },
    Function { name: String },
}

impl EntryOperationCallable {
    pub fn display_symbol(&self) -> String {
        match self {
            EntryOperationCallable::ImplMethod { type_name, method } => {
                format!("{type_name}.{method}")
            }
            EntryOperationCallable::Function { name } => name.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct EntryPointArtifacts {
    pub artifact_operations: Vec<ArtifactOperation>,
    pub runtime_operations: Vec<RuntimeOperationManifest>,
    pub service_operations: Vec<EntryOperationSpec>,
    pub raw_http: Option<RuntimeHttpRawGatewayManifest>,
    pub http_routes: Vec<RuntimeHttpRouteGatewayManifest>,
    pub websocket: Option<WebSocketGatewayArtifact>,
}

#[derive(Debug, Clone)]
pub struct WebSocketGatewayArtifact {
    pub config: WebSocketGatewayProjectionConfig,
    pub context_type: Option<WebSocketContextArtifact>,
}

#[derive(Debug, Clone)]
pub struct WebSocketContextArtifact {
    pub source_module: String,
    pub ty: EntryTypeSpec,
    pub schema_types: BTreeMap<String, PackageAbiType>,
    pub service_type_names: BTreeMap<String, String>,
}

pub fn build_entry_point_artifacts(
    service_id: &str,
    service_version: &str,
    service_target_component: &str,
    service_ingress: &ServiceIngressProjection,
    input: ProjectionView<'_>,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
    existing_operations: &BTreeSet<String>,
    package_gateway_projection: &PackageGatewayProjection,
) -> Result<EntryPointArtifacts, ProjectionError> {
    let mut artifacts = EntryPointArtifacts::default();
    let mut used_operations = existing_operations.clone();
    let runtime_index = input.lowering().synthetic_entrypoints();

    if let Some(target_name) = service_ingress
        .http()
        .and_then(|http| http.entry_target.as_deref())
    {
        let target = find_entry_target(runtime_index, "http", target_name)?;
        let method = entry_method("http", &target, "handle")?;
        validate_http_handle(&target, &method)?;
        let spec = entry_operation_spec(
            &format!("entry.{service_target_component}.http.handle"),
            &target,
            &method,
        );
        reject_duplicate_entry_operation(&spec.operation, &mut used_operations)?;
        artifacts.raw_http = Some(RuntimeHttpRawGatewayManifest {
            operation: spec.operation.clone(),
            target: format!("gateway.{service_target_component}.http.raw"),
        });
        push_entry_operation(
            &mut artifacts,
            spec,
            service_id,
            service_version,
            contract_projection,
            projection_index,
            protocol_identity,
        );
    }

    if service_ingress.http().is_some() {
        build_http_route_artifacts(
            service_id,
            service_version,
            service_target_component,
            service_ingress,
            runtime_index,
            contract_projection,
            projection_index,
            protocol_identity,
            package_gateway_projection,
            &mut used_operations,
            &mut artifacts,
        )?;
    }

    if let Some(websocket) = service_ingress.websocket() {
        if let Some(target_name) = websocket.target.as_deref() {
            let target = find_entry_target(runtime_index, "websocket", target_name)?;
            let connect = optional_entry_method(&target, "connect")?;
            let receive = entry_method("websocket", &target, "receive")?;
            let context_type = connect
                .as_ref()
                .map(validate_websocket_connect)
                .transpose()?;
            validate_websocket_receive(&receive)?;

            let connect_operation = connect
                .map(|method| {
                    let spec = entry_operation_spec(
                        &format!("entry.{service_target_component}.websocket.connect"),
                        &target,
                        &method,
                    );
                    reject_duplicate_entry_operation(&spec.operation, &mut used_operations)?;
                    let operation = WebSocketOperationProjectionConfig {
                        operation: spec.operation.clone(),
                        adapter_args: websocket_connect_adapter_args(&spec.params)?,
                    };
                    push_entry_operation(
                        &mut artifacts,
                        spec,
                        service_id,
                        service_version,
                        contract_projection,
                        projection_index,
                        protocol_identity,
                    );
                    Ok::<WebSocketOperationProjectionConfig, ProjectionError>(operation)
                })
                .transpose()?;
            let receive_spec = entry_operation_spec(
                &format!("entry.{service_target_component}.websocket.receive"),
                &target,
                &receive,
            );
            reject_duplicate_entry_operation(&receive_spec.operation, &mut used_operations)?;
            let receive_operation = WebSocketOperationProjectionConfig {
                operation: receive_spec.operation.clone(),
                adapter_args: websocket_receive_adapter_args(
                    &receive_spec.params,
                    context_type.as_ref(),
                )?,
            };
            push_entry_operation(
                &mut artifacts,
                receive_spec,
                service_id,
                service_version,
                contract_projection,
                projection_index,
                protocol_identity,
            );

            artifacts.websocket = Some(WebSocketGatewayArtifact {
                config: WebSocketGatewayProjectionConfig {
                    id: "client".to_string(),
                    path: None,
                    service_param: Some("service".to_string()),
                    context: context_type.as_ref().map(|context_type| {
                        WebSocketContextProjectionConfig {
                            context_type: context_type.name.clone(),
                            source_module: Some(target.module_path.clone()),
                        }
                    }),
                    connect: connect_operation,
                    receive: receive_operation,
                },
                context_type: context_type.map(|context_type| WebSocketContextArtifact {
                    source_module: target.module_path.clone(),
                    ty: context_type,
                    schema_types: BTreeMap::new(),
                    service_type_names: BTreeMap::new(),
                }),
            });
        } else {
            build_websocket_event_artifacts(
                service_id,
                service_version,
                service_target_component,
                websocket,
                runtime_index,
                contract_projection,
                projection_index,
                protocol_identity,
                package_gateway_projection,
                &mut used_operations,
                &mut artifacts,
            )?;
        }
    }

    Ok(artifacts)
}

#[allow(clippy::too_many_arguments)]
fn build_websocket_event_artifacts(
    service_id: &str,
    service_version: &str,
    service_target_component: &str,
    websocket: &ServiceWebSocketIngressProjection,
    runtime_index: &ProjectionSyntheticEntrypointIndex,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
    package_gateway_projection: &PackageGatewayProjection,
    used_operations: &mut BTreeSet<String>,
    artifacts: &mut EntryPointArtifacts,
) -> Result<(), ProjectionError> {
    let mut pushed_operations = BTreeSet::new();

    let connect_handler = websocket.connect.as_ref().ok_or_else(|| {
        entry_error(
            "websocket.connect is required for fixed websocket event handler config".to_string(),
        )
    })?;
    let (connect_operation, context, context_type, raw_context_type, manifest_context_type) = {
        let target = find_websocket_function_target(
            "websocket.connect",
            connect_handler,
            runtime_index,
            package_gateway_projection,
        )?;
        let context_type =
            validate_websocket_connect_event_function(target.source(), target.function())?;
        let context_artifact_type = target.service_visible_type_spec(&context_type);
        let operation = target.websocket_operation_name(WebSocketHandlerKind::Connect);
        let target_name = target.websocket_target_name(service_target_component, "connect");
        let context_source_module = target.schema_module_path();
        let raw_context_source_module = target.raw_schema_module_path();
        let (manifest_context_source_module, manifest_schema_types, manifest_service_type_names) =
            target.context_schema_projection();
        let context = Some(WebSocketContextProjectionConfig {
            context_type: context_artifact_type.name.clone(),
            source_module: Some(context_source_module.clone()),
        });
        let config = WebSocketOperationProjectionConfig {
            operation: operation.clone(),
            adapter_args: websocket_connect_adapter_args(&target.function().params)?,
        };
        push_websocket_function_operation(
            artifacts,
            operation,
            target_name,
            target,
            service_id,
            service_version,
            contract_projection,
            projection_index,
            protocol_identity,
            used_operations,
            &mut pushed_operations,
        )?;
        (
            Some(config),
            context,
            Some(WebSocketContextArtifact {
                source_module: context_source_module,
                ty: context_artifact_type.clone(),
                schema_types: BTreeMap::new(),
                service_type_names: BTreeMap::new(),
            }),
            Some(WebSocketContextArtifact {
                source_module: raw_context_source_module,
                ty: context_type,
                schema_types: BTreeMap::new(),
                service_type_names: BTreeMap::new(),
            }),
            Some(WebSocketContextArtifact {
                source_module: manifest_context_source_module,
                ty: context_artifact_type,
                schema_types: manifest_schema_types,
                service_type_names: manifest_service_type_names,
            }),
        )
    };

    let receive_handler = websocket.receive.as_ref().ok_or_else(|| {
        entry_error("websocket.receive is required for websocket route config".to_string())
    })?;
    let receive_target = find_websocket_function_target(
        "websocket.receive",
        receive_handler,
        runtime_index,
        package_gateway_projection,
    )?;
    let connect_context = receive_target.receive_validation_context(
        context_type.as_ref().expect("connect context is required"),
        raw_context_type
            .as_ref()
            .expect("raw connect context is required"),
    );
    validate_websocket_receive_event_function(
        receive_target.source(),
        &receive_target.raw_schema_module_path(),
        receive_target.function(),
        connect_context,
    )?;
    let receive_operation_name =
        receive_target.websocket_operation_name(WebSocketHandlerKind::Receive);
    let receive_target_name =
        receive_target.websocket_target_name(service_target_component, "receive");
    let receive_operation = WebSocketOperationProjectionConfig {
        operation: receive_operation_name.clone(),
        adapter_args: websocket_receive_event_adapter_args(&receive_target.function().params),
    };
    push_websocket_function_operation(
        artifacts,
        receive_operation_name,
        receive_target_name,
        receive_target,
        service_id,
        service_version,
        contract_projection,
        projection_index,
        protocol_identity,
        used_operations,
        &mut pushed_operations,
    )?;

    artifacts.websocket = Some(WebSocketGatewayArtifact {
        config: WebSocketGatewayProjectionConfig {
            id: "client".to_string(),
            path: None,
            service_param: Some("service".to_string()),
            context,
            connect: connect_operation,
            receive: receive_operation,
        },
        context_type: manifest_context_type,
    });

    Ok(())
}

fn push_websocket_function_operation(
    artifacts: &mut EntryPointArtifacts,
    operation: String,
    target: String,
    function: WebSocketFunctionTarget,
    service_id: &str,
    service_version: &str,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
    used_operations: &mut BTreeSet<String>,
    pushed_operations: &mut BTreeSet<String>,
) -> Result<(), ProjectionError> {
    if !pushed_operations.insert(operation.clone()) {
        return Ok(());
    }
    reject_duplicate_entry_operation(&operation, used_operations)?;
    match function {
        WebSocketFunctionTarget::Service(function) => {
            push_entry_operation(
                artifacts,
                EntryOperationSpec {
                    operation,
                    target,
                    implementation_module: function.module_path,
                    callable: EntryOperationCallable::Function {
                        name: function.symbol,
                    },
                    params: function.function.params,
                    return_type: function.function.return_type,
                },
                service_id,
                service_version,
                contract_projection,
                projection_index,
                protocol_identity,
            );
        }
        WebSocketFunctionTarget::Package(function) => {
            push_runtime_package_websocket_operation(
                artifacts,
                operation,
                target,
                &function,
                service_id,
                service_version,
                contract_projection,
                projection_index,
                protocol_identity,
            );
        }
    }
    Ok(())
}

fn push_runtime_package_websocket_operation(
    artifacts: &mut EntryPointArtifacts,
    operation: String,
    target: String,
    function: &PackageWebSocketFunctionTarget,
    _service_id: &str,
    _service_version: &str,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
) {
    let params = function.service_visible_params();
    let return_type = function.service_visible_type_spec(&function.function.return_type);
    let raw_response_type_ir = response_type_ir(&function.function.return_type);
    artifacts.artifact_operations.push(ArtifactOperation {
        operation: operation.clone(),
        target: Some(target.clone()),
        function: operation.clone(),
        parameters: params.iter().map(|param| param.name.clone()).collect(),
    });
    artifacts.runtime_operations.push(RuntimeOperationManifest {
        operation: operation.clone(),
        operation_abi_id: entry_operation_abi_id(&operation, &params, &return_type),
        target,
        mode: operation_response_mode(&return_type),
        parameters: params
            .iter()
            .zip(function.function.params.iter())
            .map(|(parameter, raw_parameter)| RuntimeOperationParameter {
                name: parameter.name.clone(),
                schema: package_runtime_schema_for_type_spec(
                    contract_projection,
                    projection_index,
                    &function.source_module,
                    &raw_parameter.ty,
                    &function.schema_types,
                    &function.service_type_names,
                ),
            })
            .collect(),
        response: package_runtime_schema_for_type_ref(
            contract_projection,
            projection_index,
            &function.source_module,
            &raw_response_type_ir,
            &function.function.return_type.local_type_names,
            &function.schema_types,
            &function.service_type_names,
        ),
        service_protocol_identity: protocol_identity.to_string(),
    });
}

fn push_runtime_package_http_route_operation(
    artifacts: &mut EntryPointArtifacts,
    spec: EntryOperationSpec,
    function: &PackageGatewayHandlerProjection,
    _service_id: &str,
    _service_version: &str,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
) {
    let params = service_visible_package_params(function);
    let return_type = service_visible_package_return_type(function);
    let raw_response_type_ir = response_type_ir(&function.signature.operation.return_type);
    artifacts.artifact_operations.push(ArtifactOperation {
        operation: spec.operation.clone(),
        target: Some(spec.target.clone()),
        function: spec.operation.clone(),
        parameters: params.iter().map(|param| param.name.clone()).collect(),
    });
    artifacts.runtime_operations.push(RuntimeOperationManifest {
        operation: spec.operation.clone(),
        operation_abi_id: function.signature.operation_ref.operation_abi_id.clone(),
        target: spec.target,
        mode: operation_response_mode(&return_type),
        parameters: params
            .iter()
            .zip(function.signature.operation.params.iter())
            .map(|(parameter, raw_parameter)| RuntimeOperationParameter {
                name: parameter.name.clone(),
                schema: package_runtime_schema_for_type_spec(
                    contract_projection,
                    projection_index,
                    &function.signature.source_module,
                    &raw_parameter.ty,
                    &function.signature.schema_types,
                    &function.signature.service_type_names,
                ),
            })
            .collect(),
        response: package_runtime_schema_for_type_ref(
            contract_projection,
            projection_index,
            &function.signature.source_module,
            &raw_response_type_ir,
            &function.signature.operation.return_type.local_type_names,
            &function.signature.schema_types,
            &function.signature.service_type_names,
        ),
        service_protocol_identity: protocol_identity.to_string(),
    });
}

fn service_visible_package_params(
    function: &PackageGatewayHandlerProjection,
) -> Vec<EntryParamSpec> {
    function
        .signature
        .operation
        .params
        .iter()
        .map(|param| EntryParamSpec {
            name: param.name.clone(),
            ty: EntryTypeSpec {
                name: service_visible_package_type_name(
                    &param.ty,
                    &function.signature.service_type_names,
                ),
                ir: service_visible_package_type_ir(
                    &param.ty.ir,
                    &param.ty.local_type_names,
                    &function.signature.service_type_names,
                ),
                local_type_names: BTreeMap::new(),
            },
        })
        .collect()
}

fn service_visible_package_return_type(
    function: &PackageGatewayHandlerProjection,
) -> EntryTypeSpec {
    EntryTypeSpec {
        name: service_visible_package_type_name(
            &function.signature.operation.return_type,
            &function.signature.service_type_names,
        ),
        ir: service_visible_package_type_ir(
            &function.signature.operation.return_type.ir,
            &function.signature.operation.return_type.local_type_names,
            &function.signature.service_type_names,
        ),
        local_type_names: BTreeMap::new(),
    }
}

#[derive(Debug, Clone)]
struct ParsedHttpRoute {
    index: usize,
    route: ServiceHttpRouteIngressProjection,
    handler: ServiceIngressHandlerProjection,
    guard: Option<ServiceIngressHandlerProjection>,
    pre: Option<ServiceIngressHandlerProjection>,
    kind: HttpRouteKind,
}

#[derive(Debug, Clone)]
enum HttpRouteKind {
    Raw {
        method: String,
        request: EntryParamSpec,
        context: Option<EntryParamSpec>,
        streaming: bool,
    },
    Typed {
        body: Option<EntryParamSpec>,
        context: Option<EntryParamSpec>,
        response: EntryTypeSpec,
    },
}

impl HttpRouteKind {
    fn effective_method(&self) -> &str {
        match self {
            Self::Raw { method, .. } => method,
            Self::Typed { .. } => "POST",
        }
    }
}

fn http_route_plan(
    service_ingress: &ServiceIngressProjection,
    function_index: &(impl EntrypointFunctionIndex + ?Sized),
    projection: &PackageGatewayProjection,
) -> Result<Vec<ParsedHttpRoute>, ProjectionError> {
    let Some(http) = service_ingress.http() else {
        return Ok(Vec::new());
    };
    if http.routes.is_empty() {
        return Ok(Vec::new());
    }

    let pre = http.pre.clone();
    let pre_return_type = pre
        .as_ref()
        .map(|pre| http_pre_return_type(pre, function_index, projection))
        .transpose()?;
    let guard = http.guard.clone();

    http.routes
        .iter()
        .enumerate()
        .map(|(index, route)| {
            let handler = route.handler.clone();
            let kind = classify_http_route(
                index,
                route,
                &handler,
                pre_return_type.as_ref(),
                guard.is_some(),
                function_index,
                projection,
            )?;
            Ok(ParsedHttpRoute {
                index,
                route: route.clone(),
                handler,
                guard: guard.clone(),
                pre: pre.clone(),
                kind,
            })
        })
        .collect()
}

fn http_pre_return_type(
    pre: &ServiceIngressHandlerProjection,
    function_index: &(impl EntrypointFunctionIndex + ?Sized),
    _projection: &PackageGatewayProjection,
) -> Result<EntryTypeSpec, ProjectionError> {
    match pre {
        ServiceIngressHandlerProjection::ServiceFunction {
            source,
            module_path,
            symbol,
        } => {
            let target = find_function_target("http.pre", source, module_path, symbol, function_index)?;
            validate_http_pre_function(source, &target.function)?;
            Ok(target.function.return_type)
        }
        ServiceIngressHandlerProjection::PackageFunction { source, .. } => Err(entry_error(format!(
            "http.pre {source}: service-level pre must be a service function; call package helpers from that function"
        ))),
    }
}

fn classify_http_route(
    index: usize,
    route: &ServiceHttpRouteIngressProjection,
    handler: &ServiceIngressHandlerProjection,
    pre_return_type: Option<&EntryTypeSpec>,
    has_guard: bool,
    function_index: &(impl EntrypointFunctionIndex + ?Sized),
    projection: &PackageGatewayProjection,
) -> Result<HttpRouteKind, ProjectionError> {
    match handler {
        ServiceIngressHandlerProjection::ServiceFunction {
            source,
            module_path,
            symbol,
        } => {
            let target = find_function_target(
                &format!("http.routes[{index}].handler"),
                source,
                module_path,
                symbol,
                function_index,
            )?;
            if target
                .function
                .params
                .first()
                .is_some_and(|param| is_http_request_type(&param.ty.name))
            {
                classify_raw_http_route(route, source, &target.function, pre_return_type, has_guard)
            } else {
                classify_typed_http_route(route, source, &target.function, pre_return_type)
            }
        }
        ServiceIngressHandlerProjection::PackageFunction { source, .. } => {
            let projected = projection.http_handlers.get(source).ok_or_else(|| {
                entry_error(format!(
                    "http route handler {source}: package handler is not resolved"
                ))
            })?;
            if projected
                .signature
                .operation
                .params
                .first()
                .is_some_and(|param| is_http_request_type(&param.ty.name))
            {
                classify_raw_http_route(
                    route,
                    source,
                    &projected.signature.operation,
                    pre_return_type,
                    has_guard,
                )
            } else {
                classify_typed_http_route(
                    route,
                    source,
                    &projected.signature.operation,
                    pre_return_type,
                )
            }
        }
    }
}

fn classify_raw_http_route(
    route: &ServiceHttpRouteIngressProjection,
    source: &str,
    function: &EntryFunctionSignature,
    pre_return_type: Option<&EntryTypeSpec>,
    has_guard: bool,
) -> Result<HttpRouteKind, ProjectionError> {
    let method = route.method.clone().ok_or_else(|| {
        entry_error(format!(
            "raw HTTP route handler {source} must configure method"
        ))
    })?;
    if !(1..=2).contains(&function.params.len()) {
        return Err(entry_error(format!(
            "raw HTTP route handler {source} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse or function(request: std.http.HttpRequest, context: C) -> std.http.HttpResponse"
        )));
    }
    let context = function.params.get(1);
    if let Some(context) = &context {
        validate_context_type(source, &context.ty, pre_return_type)?;
    }
    let streaming = is_http_response_stream_return_type(&function.return_type.name);
    if !is_http_response_type(&function.return_type.name) && !streaming {
        return Err(entry_error(format!(
            "raw HTTP route handler {source} must return std.http.HttpResponse or Stream<std.http.HttpResponseStreamEvent>"
        )));
    }
    if streaming && has_guard {
        return Err(entry_error(format!(
            "raw streaming HTTP route handler {source} cannot be used with legacy http.guard; use http.pre"
        )));
    }
    Ok(HttpRouteKind::Raw {
        method,
        request: function.params[0].clone(),
        context: function.params.get(1).cloned(),
        streaming,
    })
}

fn classify_typed_http_route(
    route: &ServiceHttpRouteIngressProjection,
    source: &str,
    function: &EntryFunctionSignature,
    pre_return_type: Option<&EntryTypeSpec>,
) -> Result<HttpRouteKind, ProjectionError> {
    if route.method.is_some() {
        return Err(entry_error(format!(
            "typed HTTP route handler {source} must not configure method; typed routes are POST"
        )));
    }
    if function.params.len() > 2 {
        return Err(entry_error(format!(
            "typed HTTP route handler {source} must be function() -> Response, function(input: Body) -> Response, function(context: C) -> Response, or function(input: Body, context: C) -> Response"
        )));
    }
    if is_http_response_type(&function.return_type.name)
        || is_http_response_stream_return_type(&function.return_type.name)
        || is_void_type(&function.return_type.name)
    {
        return Err(entry_error(format!(
            "typed HTTP route handler {source} must return a JSON response schema, not {}",
            function.return_type.name
        )));
    }

    let (body, context) = match function.params.as_slice() {
        [] => (None, None),
        [param] if param.name == "context" => {
            validate_context_type(source, &param.ty, pre_return_type)?;
            (None, Some(param.clone()))
        }
        [param] => (Some(param.clone()), None),
        [input, context] => {
            if context.name != "context" {
                return Err(entry_error(format!(
                    "typed HTTP route handler {source} second parameter must be named context"
                )));
            }
            validate_context_type(source, &context.ty, pre_return_type)?;
            (Some(input.clone()), Some(context.clone()))
        }
        _ => unreachable!("typed HTTP route parameter count checked"),
    };

    Ok(HttpRouteKind::Typed {
        body,
        context,
        response: function.return_type.clone(),
    })
}

fn validate_context_type(
    source: &str,
    context: &EntryTypeSpec,
    pre_return_type: Option<&EntryTypeSpec>,
) -> Result<(), ProjectionError> {
    let Some(pre_return_type) = pre_return_type else {
        return Err(entry_error(format!(
            "HTTP route handler {source} declares context but http.pre is not configured"
        )));
    };
    if is_void_type(&pre_return_type.name) {
        return Err(entry_error(format!(
            "HTTP route handler {source} declares context but http.pre returns void"
        )));
    }
    if !type_refs_match(context, pre_return_type) {
        return Err(entry_error(format!(
            "HTTP route handler {source} context type {} must match http.pre return type {}",
            context.name, pre_return_type.name
        )));
    }
    Ok(())
}

fn type_refs_match(left: &EntryTypeSpec, right: &EntryTypeSpec) -> bool {
    normalize_type_name(&left.name) == normalize_type_name(&right.name)
}

fn is_void_type(value: &str) -> bool {
    normalize_type_name(value) == "void"
}

fn is_http_response_stream_return_type(value: &str) -> bool {
    generic_parts(&normalize_type_name(value)).is_some_and(|parts| {
        parts.root == "Stream"
            && parts.args.len() == 1
            && is_http_response_stream_event_type(parts.args[0])
    })
}

#[derive(Debug)]
pub struct PackageGatewayProjection {
    http_handlers: BTreeMap<String, PackageGatewayHandlerProjection>,
    http_guards: BTreeMap<String, PackageGatewayHandlerProjection>,
    websocket_handlers: BTreeMap<String, PackageGatewayHandlerProjection>,
}

impl PackageGatewayProjection {
    pub fn build(
        service_ingress: &ServiceIngressProjection,
        package_publications: &[PackageProjectionInput],
    ) -> Result<Self, ProjectionError> {
        let package_by_id = package_publications
            .iter()
            .map(|package| (package.manifest().id(), package))
            .collect::<BTreeMap<_, _>>();
        let requests = package_gateway_handler_requests(service_ingress);
        let mut normalized_requests = Vec::new();
        let mut requested_symbol_paths = BTreeMap::<(String, String), BTreeSet<String>>::new();

        for request in requests {
            let Some(_package) = package_by_id.get(request.package_id.as_str()) else {
                return Err(entry_error(format!(
                    "{} {}: package {} is not resolved",
                    request.field, request.source, request.package_id
                )));
            };
            let lookup_symbol_path = request.symbol_path.clone();
            requested_symbol_paths
                .entry((request.package_id.clone(), request.alias.clone()))
                .or_default()
                .insert(lookup_symbol_path.clone());
            normalized_requests.push((request, lookup_symbol_path));
        }

        let mut package_projections = BTreeMap::new();
        for ((package_id, alias), symbol_paths) in requested_symbol_paths {
            let package = package_by_id
                .get(package_id.as_str())
                .expect("requested package already checked");
            let projection = PackageAbiProjection::build(package, &alias, &symbol_paths)
                .map_err(|message| entry_error(message))?;
            package_projections.insert((package_id, alias), projection);
        }

        let mut projection = Self {
            http_handlers: BTreeMap::new(),
            http_guards: BTreeMap::new(),
            websocket_handlers: BTreeMap::new(),
        };
        for (request, lookup_symbol_path) in normalized_requests {
            let package_projection = package_projections
                .get(&(request.package_id.clone(), request.alias.clone()))
                .expect("package projection should exist for requested handler");
            let signature = package_projection
                .function_signature(&lookup_symbol_path, &request.package_id)
                .map_err(|message| {
                    entry_error(format!("{} {}: {message}", request.field, request.source))
                })?;
            match request.kind {
                PackageGatewayHandlerKind::HttpRoute => {
                    validate_http_package_route_function(
                        &request.field,
                        &request.source,
                        &signature.operation,
                    )?;
                    projection.http_handlers.entry(request.source).or_insert(
                        PackageGatewayHandlerProjection {
                            package_id: request.package_id,
                            alias: request.alias,
                            symbol_path: lookup_symbol_path,
                            signature,
                        },
                    );
                }
                PackageGatewayHandlerKind::HttpGuard => {
                    validate_http_package_guard_function(
                        &request.field,
                        &request.source,
                        &signature.operation,
                    )?;
                    projection.http_guards.entry(request.source).or_insert(
                        PackageGatewayHandlerProjection {
                            package_id: request.package_id,
                            alias: request.alias,
                            symbol_path: lookup_symbol_path,
                            signature,
                        },
                    );
                }
                PackageGatewayHandlerKind::WebSocket(WebSocketHandlerKind::Connect) => {
                    validate_websocket_connect_event_shape(&request.source, &signature.operation)?;
                    projection
                        .websocket_handlers
                        .entry(request.source)
                        .or_insert(PackageGatewayHandlerProjection {
                            package_id: request.package_id,
                            alias: request.alias,
                            symbol_path: lookup_symbol_path,
                            signature,
                        });
                }
                PackageGatewayHandlerKind::WebSocket(WebSocketHandlerKind::Receive) => {
                    validate_websocket_receive_event_shape(&request.source, &signature.operation)?;
                    projection
                        .websocket_handlers
                        .entry(request.source)
                        .or_insert(PackageGatewayHandlerProjection {
                            package_id: request.package_id,
                            alias: request.alias,
                            symbol_path: lookup_symbol_path,
                            signature,
                        });
                }
            }
        }

        Ok(projection)
    }
}

#[derive(Debug)]
struct PackageGatewayHandlerRequest {
    field: String,
    source: String,
    package_id: String,
    alias: String,
    symbol_path: String,
    kind: PackageGatewayHandlerKind,
}

#[derive(Debug, Clone, Copy)]
enum PackageGatewayHandlerKind {
    HttpRoute,
    HttpGuard,
    WebSocket(WebSocketHandlerKind),
}

#[derive(Debug)]
struct PackageGatewayHandlerProjection {
    package_id: String,
    alias: String,
    symbol_path: String,
    signature: PackageFunctionSignature,
}

fn package_gateway_handler_requests(
    service_ingress: &ServiceIngressProjection,
) -> Vec<PackageGatewayHandlerRequest> {
    let mut requests = Vec::new();
    if let Some(http) = service_ingress.http() {
        if let Some(ServiceIngressHandlerProjection::PackageFunction {
            source,
            package_id,
            alias,
            symbol_path,
        }) = &http.guard
        {
            requests.push(PackageGatewayHandlerRequest {
                field: "http guard".to_string(),
                source: source.clone(),
                package_id: package_id.clone(),
                alias: alias.clone(),
                symbol_path: symbol_path.clone(),
                kind: PackageGatewayHandlerKind::HttpGuard,
            });
        }
        for route in &http.routes {
            let ServiceIngressHandlerProjection::PackageFunction {
                source,
                package_id,
                alias,
                symbol_path,
            } = &route.handler
            else {
                continue;
            };
            requests.push(PackageGatewayHandlerRequest {
                field: format!("http route {} handler", route.path),
                source: source.clone(),
                package_id: package_id.clone(),
                alias: alias.clone(),
                symbol_path: symbol_path.clone(),
                kind: PackageGatewayHandlerKind::HttpRoute,
            });
        }
    }

    let Some(websocket) = service_ingress.websocket() else {
        return requests;
    };
    if websocket.target.is_some() {
        return requests;
    }
    for (field, kind, handler) in websocket_handler_configs(websocket) {
        let ServiceIngressHandlerProjection::PackageFunction {
            source,
            package_id,
            alias,
            symbol_path,
        } = handler
        else {
            continue;
        };
        requests.push(PackageGatewayHandlerRequest {
            field: field.to_string(),
            source: source.clone(),
            package_id: package_id.clone(),
            alias: alias.clone(),
            symbol_path: symbol_path.clone(),
            kind: PackageGatewayHandlerKind::WebSocket(kind),
        });
    }

    requests
}

#[derive(Debug, Clone, Copy)]
enum WebSocketHandlerKind {
    Connect,
    Receive,
}

fn websocket_handler_configs(
    websocket: &ServiceWebSocketIngressProjection,
) -> Vec<(
    &'static str,
    WebSocketHandlerKind,
    &ServiceIngressHandlerProjection,
)> {
    let mut handlers = Vec::new();
    if let Some(connect) = &websocket.connect {
        handlers.push(("websocket.connect", WebSocketHandlerKind::Connect, connect));
    }
    if let Some(receive) = &websocket.receive {
        handlers.push(("websocket.receive", WebSocketHandlerKind::Receive, receive));
    }
    handlers
}

#[derive(Debug, Clone)]
struct PackageFunctionSignature {
    source_module: String,
    operation: EntryFunctionSignature,
    operation_ref: OperationAbiRef,
    schema_types: BTreeMap<String, PackageAbiType>,
    service_type_names: BTreeMap<String, String>,
}

#[derive(Debug)]
pub struct PackageAbiProjection {
    functions: BTreeMap<String, PackageFunctionSignature>,
}

impl PackageAbiProjection {
    pub fn build(
        package_publication: &PackageProjectionInput,
        alias: &str,
        symbol_paths: &BTreeSet<String>,
    ) -> Result<Self, String> {
        let manifest = package_publication.manifest();
        let compiled = package_publication.compiled();
        let entrypoints = compiled.lowering().package_entrypoints();
        let mut functions = BTreeMap::new();

        for symbol_path in symbol_paths {
            let Some(function_projection) = entrypoints.function(symbol_path) else {
                continue;
            };
            let source_module = function_projection.source_module.clone();
            let function = function_projection.signature.clone();
            let public_path = compiled
                .source()
                .publication_api_seed()
                .public_modules
                .iter()
                .find_map(|(public_path, module)| {
                    (module == &source_module).then_some(public_path.as_str())
                })
                .unwrap_or(&source_module);
            let service_type_names = package_service_visible_type_names(
                manifest.id(),
                &source_module,
                public_path,
                alias,
                compiled,
            );
            let schema_types = entrypoints
                .schema_abi_types_for_module(&source_module)
                .ok_or_else(|| {
                    format!(
                        "api module {} not found in compiled package projection input",
                        source_module
                    )
                })?
                .iter()
                .cloned()
                .map(|ty| (ty.name.clone(), ty))
                .collect();
            let operation_ref =
                package_gateway_operation_ref(manifest.id(), symbol_path, &function);
            if functions.contains_key(symbol_path) {
                continue;
            }
            functions.insert(
                symbol_path.clone(),
                PackageFunctionSignature {
                    source_module,
                    operation: function,
                    operation_ref,
                    schema_types,
                    service_type_names,
                },
            );
        }

        Ok(Self { functions })
    }

    fn function_signature(
        &self,
        symbol_path: &str,
        package_id: &str,
    ) -> Result<PackageFunctionSignature, String> {
        self.functions.get(symbol_path).cloned().ok_or_else(|| {
            format!("exported function {symbol_path} not found in package {package_id}")
        })
    }
}

fn package_gateway_operation_ref(
    package_id: &str,
    symbol_path: &str,
    function: &EntryFunctionSignature,
) -> OperationAbiRef {
    let public_path = package_gateway_public_operation_path(package_id, symbol_path);
    let public_signature = CanonicalPublicCallableSignature {
        params: function
            .params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: param.ty.ir.clone(),
            })
            .collect(),
        return_type: function.return_type.ir.clone(),
        may_suspend: operation_response_mode(&function.return_type) == "serverStream",
    };
    OperationAbiRef {
        operation_abi_id: public_function_operation_abi_id(
            &public_path,
            &public_signature,
            &[],
            &Default::default(),
        ),
        kind: skiff_artifact_model::PublicationOperationKind::PublicFunction,
        public_path: public_path.clone(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: public_path,
    }
}

fn package_gateway_public_operation_path(package_id: &str, symbol_path: &str) -> String {
    if package_id == SKIFF_STD_PUBLICATION_ID && !symbol_path.starts_with("std.") {
        format!("std.{symbol_path}")
    } else {
        symbol_path.to_string()
    }
}

fn package_service_visible_type_names(
    package_id: &str,
    module_path: &str,
    export_path: &str,
    alias: &str,
    compiled: ProjectionView<'_>,
) -> BTreeMap<String, String> {
    let mut mappings = BTreeMap::new();
    let exported_type_names = compiled
        .lowering()
        .package_entrypoints()
        .schema_type_names_for_module(module_path)
        .iter();
    for name in exported_type_names {
        let service_name =
            package_service_visible_symbol_name(package_id, export_path, alias, name);
        mappings.insert(name.clone(), service_name.clone());
        mappings.insert(format!("{module_path}.{name}"), service_name.clone());
        mappings.insert(format!("root.{module_path}.{name}"), service_name.clone());
        if !export_path.is_empty() {
            mappings.insert(format!("{export_path}.{name}"), service_name.clone());
            mappings.insert(
                format!("{}.{name}", package_public_path(package_id, export_path)),
                service_name,
            );
        }
    }
    mappings
}

fn package_service_visible_module_path(package_id: &str, export_path: &str, alias: &str) -> String {
    let relative_path = package_alias_relative_export_path(package_id, export_path);
    if relative_path.is_empty() {
        format!("__skiff.package_types.{alias}")
    } else {
        format!("__skiff.package_types.{alias}.{relative_path}")
    }
}

fn package_service_visible_symbol_name(
    package_id: &str,
    export_path: &str,
    alias: &str,
    symbol: &str,
) -> String {
    format!(
        "{}.{symbol}",
        package_service_visible_module_path(package_id, export_path, alias)
    )
}

fn package_alias_relative_export_path(_package_id: &str, export_path: &str) -> String {
    if export_path.is_empty() {
        return String::new();
    }
    export_path.to_string()
}

fn service_visible_package_type_name(
    ty: &EntryTypeSpec,
    mappings: &BTreeMap<String, String>,
) -> String {
    entry_type_source_text_with_named_types(ty, &|name| {
        mappings
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    })
}

fn service_visible_package_type_ir(
    ty: &TypeRefIr,
    local_type_names: &BTreeMap<u32, String>,
    mappings: &BTreeMap<String, String>,
) -> TypeRefIr {
    match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| service_visible_package_type_ir(arg, local_type_names, mappings))
                .collect(),
        },
        TypeRefIr::LocalType { type_index } => local_type_names
            .get(type_index)
            .and_then(|name| mappings.get(name))
            .and_then(|name| service_symbol_type_ref_from_qualified_name(name))
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => mappings
            .get(&symbol.symbol_path())
            .or_else(|| mappings.get(&symbol.symbol))
            .and_then(|name| service_symbol_type_ref_from_qualified_name(name))
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::PackageSymbol { symbol } => mappings
            .get(&symbol.symbol_path)
            .or_else(|| {
                symbol
                    .symbol_path
                    .rsplit_once('.')
                    .and_then(|(_, name)| mappings.get(name))
            })
            .and_then(|name| service_symbol_type_ref_from_qualified_name(name))
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        service_visible_package_type_ir(ty, local_type_names, mappings),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| service_visible_package_type_ir(item, local_type_names, mappings))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(service_visible_package_type_ir(
                inner,
                local_type_names,
                mappings,
            )),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| service_visible_package_type_ir(arg, local_type_names, mappings))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: service_visible_package_type_ir(&param.ty, local_type_names, mappings),
                })
                .collect(),
            return_type: Box::new(service_visible_package_type_ir(
                return_type,
                local_type_names,
                mappings,
            )),
        },
        TypeRefIr::PublicationType { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn service_symbol_type_ref_from_qualified_name(name: &str) -> Option<TypeRefIr> {
    let name = name.strip_prefix("root.").unwrap_or(name);
    let (module_path, symbol) = name.rsplit_once('.')?;
    Some(TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    })
}

fn validate_http_guard_config(
    service_ingress: &ServiceIngressProjection,
    runtime_index: &ProjectionSyntheticEntrypointIndex,
    projection: &PackageGatewayProjection,
) -> Result<(), ProjectionError> {
    let Some(guard) = service_ingress.http().and_then(|http| http.guard.as_ref()) else {
        return Ok(());
    };
    match guard {
        ServiceIngressHandlerProjection::ServiceFunction {
            source,
            module_path,
            symbol,
        } => {
            let target =
                find_function_target("http.guard", &source, &module_path, &symbol, runtime_index)?;
            validate_http_guard_function(&source, &target.function)
        }
        ServiceIngressHandlerProjection::PackageFunction {
            source, package_id, ..
        } => {
            if projection.http_guards.contains_key(source.as_str()) {
                Ok(())
            } else {
                Err(entry_error(format!(
                    "http guard {source}: package {package_id} is not resolved"
                )))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_http_route_artifacts(
    service_id: &str,
    service_version: &str,
    service_target_component: &str,
    service_ingress: &ServiceIngressProjection,
    runtime_index: &ProjectionSyntheticEntrypointIndex,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
    projection: &PackageGatewayProjection,
    used_operations: &mut BTreeSet<String>,
    artifacts: &mut EntryPointArtifacts,
) -> Result<(), ProjectionError> {
    let route_plan = http_route_plan(service_ingress, runtime_index, projection)?;
    let mut pushed_route_operations = BTreeSet::new();
    validate_http_guard_config(service_ingress, runtime_index, projection)?;

    for parsed_route in route_plan {
        let index = parsed_route.index;
        let route = parsed_route.route;
        let kind = parsed_route.kind;
        let handler_ref = parsed_route.handler.clone();
        let guard_ref = parsed_route.guard.clone();
        let pre_ref = parsed_route.pre.clone();
        let handler_manifest = route_handler_manifest(&handler_ref);
        match parsed_route.handler {
            ServiceIngressHandlerProjection::ServiceFunction {
                source,
                module_path,
                symbol,
            } => {
                let target = find_function_target(
                    &format!("http.routes[{index}].handler"),
                    &source,
                    &module_path,
                    &symbol,
                    runtime_index,
                )?;
                validate_projected_http_route_function(&source, &target.function, &kind)?;
                let operation = http_route_operation_name(&module_path, &symbol);
                let typed = http_route_typed_manifest(
                    service_id,
                    &route.path,
                    &kind,
                    &handler_ref,
                    guard_ref.as_ref(),
                    pre_ref.as_ref(),
                    contract_projection,
                    projection_index,
                    &target.module_path,
                );
                let adapter = http_route_raw_adapter_manifest(
                    &kind,
                    &handler_ref,
                    guard_ref.as_ref(),
                    pre_ref.as_ref(),
                    false,
                );
                let target_name =
                    http_route_target_name(service_target_component, &module_path, &symbol);
                artifacts.http_routes.push(RuntimeHttpRouteGatewayManifest {
                    method: kind.effective_method().to_string(),
                    path: route.path.clone(),
                    operation: operation.clone(),
                    operation_abi_id: Some(entry_operation_abi_id(
                        &operation,
                        &target.function.params,
                        &target.function.return_type,
                    )),
                    target: target_name.clone(),
                    handler: Some(handler_manifest),
                    adapter,
                    typed,
                });
                if pushed_route_operations.insert(operation.clone()) {
                    reject_duplicate_entry_operation(&operation, used_operations)?;
                    push_entry_operation(
                        artifacts,
                        EntryOperationSpec {
                            operation,
                            target: target_name,
                            implementation_module: module_path,
                            callable: EntryOperationCallable::Function { name: symbol },
                            params: target.function.params,
                            return_type: target.function.return_type,
                        },
                        service_id,
                        service_version,
                        contract_projection,
                        projection_index,
                        protocol_identity,
                    );
                }
            }
            ServiceIngressHandlerProjection::PackageFunction {
                source,
                package_id,
                alias,
                symbol_path,
            } => {
                let projected = projection.http_handlers.get(&source).ok_or_else(|| {
                    entry_error(format!(
                        "http route handler {source}: package {package_id} is not resolved"
                    ))
                })?;
                validate_projected_http_route_function(
                    &source,
                    &projected.signature.operation,
                    &kind,
                )?;
                let operation = http_route_package_operation_name(&alias, &symbol_path);
                if matches!(kind, HttpRouteKind::Typed { .. }) {
                    let target_name = package_handler_target(&package_id, &symbol_path);
                    let operation_abi_id =
                        projected.signature.operation_ref.operation_abi_id.clone();
                    let typed = http_route_typed_manifest(
                        service_id,
                        &route.path,
                        &kind,
                        &handler_ref,
                        guard_ref.as_ref(),
                        pre_ref.as_ref(),
                        contract_projection,
                        projection_index,
                        projected.signature.source_module.as_str(),
                    );
                    artifacts.http_routes.push(RuntimeHttpRouteGatewayManifest {
                        method: kind.effective_method().to_string(),
                        path: route.path.clone(),
                        operation: operation.clone(),
                        operation_abi_id: Some(operation_abi_id),
                        target: target_name.clone(),
                        handler: Some(handler_manifest),
                        adapter: None,
                        typed,
                    });
                    if pushed_route_operations.insert(operation.clone()) {
                        reject_duplicate_entry_operation(&operation, used_operations)?;
                        push_runtime_package_http_route_operation(
                            artifacts,
                            EntryOperationSpec {
                                operation,
                                target: target_name,
                                implementation_module: projected.signature.source_module.clone(),
                                callable: EntryOperationCallable::Function {
                                    name: symbol_path.clone(),
                                },
                                params: projected.signature.operation.params.clone(),
                                return_type: projected.signature.operation.return_type.clone(),
                            },
                            projected,
                            service_id,
                            service_version,
                            contract_projection,
                            projection_index,
                            protocol_identity,
                        );
                    }
                    continue;
                }
                let target_name = package_handler_target(&package_id, &symbol_path);
                let adapter = http_route_raw_adapter_manifest(
                    &kind,
                    &handler_ref,
                    guard_ref.as_ref(),
                    pre_ref.as_ref(),
                    true,
                );
                artifacts.http_routes.push(RuntimeHttpRouteGatewayManifest {
                    method: kind.effective_method().to_string(),
                    path: route.path.clone(),
                    operation: operation.clone(),
                    operation_abi_id: Some(
                        projected.signature.operation_ref.operation_abi_id.clone(),
                    ),
                    target: target_name.clone(),
                    handler: Some(handler_manifest),
                    adapter,
                    typed: None,
                });
                if pushed_route_operations.insert(operation.clone()) {
                    reject_duplicate_entry_operation(&operation, used_operations)?;
                    push_runtime_package_http_route_operation(
                        artifacts,
                        EntryOperationSpec {
                            operation,
                            target: target_name,
                            implementation_module: projected.signature.source_module.clone(),
                            callable: EntryOperationCallable::Function {
                                name: symbol_path.clone(),
                            },
                            params: projected.signature.operation.params.clone(),
                            return_type: projected.signature.operation.return_type.clone(),
                        },
                        projected,
                        service_id,
                        service_version,
                        contract_projection,
                        projection_index,
                        protocol_identity,
                    );
                }
            }
        }
    }

    Ok(())
}

fn route_handler_manifest(
    handler: &ServiceIngressHandlerProjection,
) -> RuntimeHttpRouteHandlerManifest {
    match handler {
        ServiceIngressHandlerProjection::ServiceFunction {
            source,
            module_path,
            symbol,
        } => RuntimeHttpRouteHandlerManifest::ServiceFunction {
            source: Some(source.clone()),
            module_path: module_path.clone(),
            symbol: symbol.clone(),
        },
        ServiceIngressHandlerProjection::PackageFunction {
            source,
            package_id,
            alias,
            symbol_path,
        } => RuntimeHttpRouteHandlerManifest::PackageFunction {
            source: Some(source.clone()),
            package_id: package_id.clone(),
            alias: Some(alias.clone()),
            symbol_path: symbol_path.clone(),
        },
    }
}

fn adapter_callable_manifest(
    handler: &ServiceIngressHandlerProjection,
) -> RuntimeHttpRouteAdapterCallableManifest {
    match handler {
        ServiceIngressHandlerProjection::ServiceFunction {
            module_path,
            symbol,
            ..
        } => RuntimeHttpRouteAdapterCallableManifest::ServiceFunction {
            module_path: module_path.clone(),
            symbol: symbol.clone(),
        },
        ServiceIngressHandlerProjection::PackageFunction {
            package_id,
            symbol_path,
            ..
        } => RuntimeHttpRouteAdapterCallableManifest::PackageFunction {
            package_id: package_id.clone(),
            symbol_path: symbol_path.clone(),
        },
    }
}

#[derive(Debug)]
struct FunctionTarget {
    source: String,
    module_path: String,
    symbol: String,
    function: EntryFunctionSignature,
}

#[derive(Debug)]
enum WebSocketFunctionTarget {
    Service(FunctionTarget),
    Package(PackageWebSocketFunctionTarget),
}

#[derive(Debug)]
struct PackageWebSocketFunctionTarget {
    source: String,
    package_id: String,
    alias: String,
    source_module: String,
    symbol_path: String,
    function: EntryFunctionSignature,
    schema_types: BTreeMap<String, PackageAbiType>,
    service_type_names: BTreeMap<String, String>,
}

impl WebSocketFunctionTarget {
    fn source(&self) -> &str {
        match self {
            WebSocketFunctionTarget::Service(target) => &target.source,
            WebSocketFunctionTarget::Package(target) => &target.source,
        }
    }

    fn function(&self) -> &EntryFunctionSignature {
        match self {
            WebSocketFunctionTarget::Service(target) => &target.function,
            WebSocketFunctionTarget::Package(target) => &target.function,
        }
    }

    fn schema_module_path(&self) -> String {
        match self {
            WebSocketFunctionTarget::Service(target) => target.module_path.clone(),
            WebSocketFunctionTarget::Package(_) => String::new(),
        }
    }

    fn raw_schema_module_path(&self) -> String {
        match self {
            WebSocketFunctionTarget::Service(target) => target.module_path.clone(),
            WebSocketFunctionTarget::Package(target) => target.source_module.clone(),
        }
    }

    fn receive_validation_context<'a>(
        &self,
        service_visible_context: &'a WebSocketContextArtifact,
        raw_context: &'a WebSocketContextArtifact,
    ) -> &'a WebSocketContextArtifact {
        match self {
            WebSocketFunctionTarget::Service(_) => service_visible_context,
            WebSocketFunctionTarget::Package(_) => raw_context,
        }
    }

    fn websocket_operation_name(&self, kind: WebSocketHandlerKind) -> String {
        match self {
            WebSocketFunctionTarget::Service(target) => match kind {
                WebSocketHandlerKind::Connect => {
                    websocket_connect_operation_name(&target.module_path, &target.symbol)
                }
                WebSocketHandlerKind::Receive => {
                    websocket_receive_operation_name(&target.module_path, &target.symbol)
                }
            },
            WebSocketFunctionTarget::Package(target) => {
                websocket_package_operation_name(kind, &target.alias, &target.symbol_path)
            }
        }
    }

    fn websocket_target_name(&self, service_target_component: &str, kind: &str) -> String {
        match self {
            WebSocketFunctionTarget::Service(target) => websocket_target_name(
                service_target_component,
                kind,
                &target.module_path,
                &target.symbol,
            ),
            WebSocketFunctionTarget::Package(target) => {
                package_handler_target(&target.package_id, &target.symbol_path)
            }
        }
    }

    fn service_visible_type_spec(&self, ty: &EntryTypeSpec) -> EntryTypeSpec {
        match self {
            WebSocketFunctionTarget::Service(_) => ty.clone(),
            WebSocketFunctionTarget::Package(target) => target.service_visible_type_spec(ty),
        }
    }

    fn context_schema_projection(
        &self,
    ) -> (
        String,
        BTreeMap<String, PackageAbiType>,
        BTreeMap<String, String>,
    ) {
        match self {
            WebSocketFunctionTarget::Service(target) => {
                (target.module_path.clone(), BTreeMap::new(), BTreeMap::new())
            }
            WebSocketFunctionTarget::Package(target) => (
                target.source_module.clone(),
                target.schema_types.clone(),
                target.service_type_names.clone(),
            ),
        }
    }
}

impl PackageWebSocketFunctionTarget {
    fn service_visible_params(&self) -> Vec<EntryParamSpec> {
        self.function
            .params
            .iter()
            .map(|param| EntryParamSpec {
                name: param.name.clone(),
                ty: self.service_visible_type_spec(&param.ty),
            })
            .collect()
    }

    fn service_visible_type_spec(&self, ty: &EntryTypeSpec) -> EntryTypeSpec {
        EntryTypeSpec {
            name: service_visible_package_type_name(ty, &self.service_type_names),
            ir: service_visible_package_type_ir(
                &ty.ir,
                &ty.local_type_names,
                &self.service_type_names,
            ),
            local_type_names: BTreeMap::new(),
        }
    }
}

trait EntrypointFunctionIndex {
    fn find_function_target(
        &self,
        field: &str,
        source_text: &str,
        module_path: &str,
        symbol: &str,
    ) -> Result<FunctionTarget, ProjectionError>;
}

impl EntrypointFunctionIndex for ProjectionEntrypointAbiIndex {
    fn find_function_target(
        &self,
        field: &str,
        source_text: &str,
        module_path: &str,
        symbol: &str,
    ) -> Result<FunctionTarget, ProjectionError> {
        let Some(function) = self.function_signature(module_path, symbol) else {
            return Err(entry_error(format!(
                "{field} {source_text}: function {symbol} not found in service entrypoint ABI module {module_path}"
            )));
        };
        Ok(FunctionTarget {
            source: source_text.to_string(),
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
            function,
        })
    }
}

fn find_function_target(
    field: &str,
    source_text: &str,
    module_path: &str,
    symbol: &str,
    function_index: &(impl EntrypointFunctionIndex + ?Sized),
) -> Result<FunctionTarget, ProjectionError> {
    function_index.find_function_target(field, source_text, module_path, symbol)
}

fn find_websocket_function_target(
    field: &str,
    handler: &ServiceIngressHandlerProjection,
    runtime_index: &ProjectionSyntheticEntrypointIndex,
    projection: &PackageGatewayProjection,
) -> Result<WebSocketFunctionTarget, ProjectionError> {
    match handler {
        ServiceIngressHandlerProjection::ServiceFunction {
            source,
            module_path,
            symbol,
        } => find_function_target(field, &source, &module_path, &symbol, runtime_index)
            .map(WebSocketFunctionTarget::Service),
        ServiceIngressHandlerProjection::PackageFunction {
            source, package_id, ..
        } => {
            let Some(projected) = projection.websocket_handlers.get(source.as_str()) else {
                return Err(entry_error(format!(
                    "{field} {source}: package {package_id} is not resolved"
                )));
            };
            Ok(WebSocketFunctionTarget::Package(
                PackageWebSocketFunctionTarget {
                    source: source.clone(),
                    package_id: projected.package_id.clone(),
                    alias: projected.alias.clone(),
                    source_module: projected.signature.source_module.clone(),
                    symbol_path: projected.symbol_path.clone(),
                    function: projected.signature.operation.clone(),
                    schema_types: projected.signature.schema_types.clone(),
                    service_type_names: projected.signature.service_type_names.clone(),
                },
            ))
        }
    }
}

fn validate_projected_http_route_function(
    source: &str,
    function: &EntryFunctionSignature,
    kind: &HttpRouteKind,
) -> Result<(), ProjectionError> {
    match kind {
        HttpRouteKind::Raw {
            request,
            context,
            streaming,
            ..
        } => {
            let expected_params = if context.is_some()
                && function
                    .params
                    .get(1)
                    .is_some_and(|param| type_refs_match(&param.ty, &context.as_ref().unwrap().ty))
            {
                2
            } else {
                1
            };
            if function.params.len() != expected_params
                || function.params[0].name != request.name
                || !is_http_request_type(&function.params[0].ty.name)
                || (*streaming && !is_http_response_stream_return_type(&function.return_type.name))
                || (!*streaming && !is_http_response_type(&function.return_type.name))
            {
                return Err(entry_error(format!(
                    "http route handler {source} must be a raw HTTP route entry"
                )));
            }
        }
        HttpRouteKind::Typed { .. } => {
            let is_wrapper = function.params.len() == 1
                && is_http_request_type(&function.params[0].ty.name)
                && is_http_response_type(&function.return_type.name);
            if !is_wrapper
                && function
                    .params
                    .first()
                    .is_some_and(|param| is_http_request_type(&param.ty.name))
            {
                return Err(entry_error(format!(
                    "typed HTTP route handler {source} must not receive std.http.HttpRequest"
                )));
            }
        }
    }
    Ok(())
}

fn validate_http_pre_function(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    if function.params.len() != 1 || !is_http_request_type(&function.params[0].ty.name) {
        return Err(entry_error(format!(
            "http.pre {source} must be function(request: std.http.HttpRequest) -> C"
        )));
    }
    Ok(())
}

fn http_route_typed_manifest(
    service_id: &str,
    path: &str,
    kind: &HttpRouteKind,
    handler: &ServiceIngressHandlerProjection,
    guard: Option<&ServiceIngressHandlerProjection>,
    pre: Option<&ServiceIngressHandlerProjection>,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    handler_module: &str,
) -> Option<RuntimeHttpRouteTypedManifest> {
    let HttpRouteKind::Typed {
        body,
        context,
        response,
    } = kind
    else {
        return None;
    };
    let body_schema = body.as_ref().map(|body| {
        contract_projection.schema_for_source_type_ref(
            projection_index,
            handler_module,
            &body.ty.ir,
        )
    });
    let response_schema = contract_projection.schema_for_source_type_ref(
        projection_index,
        handler_module,
        &response.ir,
    );
    let ingress_identity = http_ingress_identity(
        service_id,
        "POST",
        path,
        body_schema.as_ref(),
        &response_schema,
    );
    Some(RuntimeHttpRouteTypedManifest {
        body: body_schema.map(|schema| RuntimeHttpRouteTypedBodyManifest { schema }),
        response: RuntimeHttpRouteTypedResponseManifest {
            schema: response_schema,
        },
        ingress_identity,
        adapter: Some(RuntimeHttpRouteAdapterManifest {
            kind: RuntimeHttpRouteAdapterKind::TypedJson,
            handler: adapter_callable_manifest(handler),
            guard: guard.map(adapter_callable_manifest),
            pre: pre.map(adapter_callable_manifest),
            adapter_args: typed_http_adapter_args(body.as_ref(), context.as_ref()),
        }),
    })
}

fn http_route_raw_adapter_manifest(
    kind: &HttpRouteKind,
    handler: &ServiceIngressHandlerProjection,
    guard: Option<&ServiceIngressHandlerProjection>,
    pre: Option<&ServiceIngressHandlerProjection>,
    force: bool,
) -> Option<RuntimeHttpRouteAdapterManifest> {
    let HttpRouteKind::Raw { .. } = kind else {
        return None;
    };
    if !force && guard.is_none() && pre.is_none() {
        return None;
    }
    Some(RuntimeHttpRouteAdapterManifest {
        kind: RuntimeHttpRouteAdapterKind::RawHttp,
        handler: adapter_callable_manifest(handler),
        guard: guard.map(adapter_callable_manifest),
        pre: pre.map(adapter_callable_manifest),
        adapter_args: raw_http_adapter_args(kind),
    })
}

fn typed_http_adapter_args(
    body: Option<&EntryParamSpec>,
    context: Option<&EntryParamSpec>,
) -> Vec<RuntimeGatewayAdapterArgManifest> {
    let mut args = Vec::new();
    if let Some(body) = body {
        args.push(gateway_adapter_arg(
            &body.name,
            RuntimeGatewayAdapterSourceManifest::HttpBody,
        ));
    }
    if let Some(context) = context {
        args.push(gateway_adapter_arg(
            &context.name,
            RuntimeGatewayAdapterSourceManifest::HttpContext,
        ));
    }
    args
}

fn raw_http_adapter_args(kind: &HttpRouteKind) -> Vec<RuntimeGatewayAdapterArgManifest> {
    let HttpRouteKind::Raw {
        request, context, ..
    } = kind
    else {
        return Vec::new();
    };
    let mut args = vec![gateway_adapter_arg(
        &request.name,
        RuntimeGatewayAdapterSourceManifest::HttpRequest,
    )];
    if let Some(context) = context {
        args.push(gateway_adapter_arg(
            &context.name,
            RuntimeGatewayAdapterSourceManifest::HttpContext,
        ));
    }
    args
}

fn gateway_adapter_arg(
    param: &str,
    source: RuntimeGatewayAdapterSourceManifest,
) -> RuntimeGatewayAdapterArgManifest {
    RuntimeGatewayAdapterArgManifest {
        param: param.to_string(),
        source,
    }
}

fn validate_http_guard_function(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    if function.params.len() != 1
        || !is_http_request_type(&function.params[0].ty.name)
        || !is_nullable_http_response_type(&function.return_type.name)
    {
        return Err(entry_error(format!(
            "http guard {source} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse?"
        )));
    }
    Ok(())
}

fn validate_http_package_route_function(
    field: &str,
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    if (1..=2).contains(&function.params.len())
        && is_http_request_type(&function.params[0].ty.name)
        && (is_http_response_type(&function.return_type.name)
            || is_http_response_stream_return_type(&function.return_type.name))
    {
        return Ok(());
    }
    if function.params.len() <= 2
        && !function
            .params
            .first()
            .is_some_and(|param| is_http_request_type(&param.ty.name))
        && !is_http_response_type(&function.return_type.name)
        && !is_http_response_stream_return_type(&function.return_type.name)
        && !is_void_type(&function.return_type.name)
    {
        return Ok(());
    }
    Err(entry_error(format!(
        "{field} {source} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse or a typed JSON HTTP route handler"
    )))
}

fn validate_http_package_guard_function(
    field: &str,
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    if function.params.len() != 1
        || !is_http_request_type(&function.params[0].ty.name)
        || !is_nullable_http_response_type(&function.return_type.name)
    {
        return Err(entry_error(format!(
            "{field} {source} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse?"
        )));
    }
    Ok(())
}

fn http_route_operation_name(module_path: &str, symbol: &str) -> String {
    format!("http.route.{module_path}.{symbol}")
}

fn http_route_package_operation_name(alias: &str, symbol_path: &str) -> String {
    format!("http.route.{alias}.{symbol_path}")
}

fn package_handler_target(package_id: &str, symbol_path: &str) -> String {
    format!(
        "package.{}.{}",
        encode_package_target_segment(package_id),
        encode_package_target_segment(symbol_path)
    )
}

fn encode_package_target_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn http_route_target_name(
    service_target_component: &str,
    module_path: &str,
    symbol: &str,
) -> String {
    format!("entry.{service_target_component}.http.route.{module_path}.{symbol}")
}

#[derive(Debug)]
struct EntryTarget<'a> {
    module_path: String,
    type_name: String,
    module: &'a ProjectionSyntheticEntrypointModule,
}

fn entry_method(
    field: &str,
    target: &EntryTarget<'_>,
    method_name: &str,
) -> Result<EntryFunctionSignature, ProjectionError> {
    optional_entry_method(target, method_name)?.ok_or_else(|| {
        entry_error(format!(
            "{field} entry target {}.{}: impl {} missing method {method_name}",
            target.module_path, target.type_name, target.type_name
        ))
    })
}

fn optional_entry_method(
    target: &EntryTarget<'_>,
    method_name: &str,
) -> Result<Option<EntryFunctionSignature>, ProjectionError> {
    let declaration_name = format!("{}.{}", target.type_name, method_name);
    let Some(executable) = target.module.executable(&declaration_name) else {
        if target.module.has_type(&target.type_name) {
            return Ok(None);
        }
        return Err(entry_error(format!(
            "entry target {}.{}: impl {} not found in module {}",
            target.module_path, target.type_name, target.type_name, target.module_path
        )));
    };
    if executable.kind() != ProjectionSyntheticEntrypointExecutableKind::ImplMethod {
        return Err(entry_error(format!(
            "entry target {}.{} method {method_name}: expected impl method executable",
            target.module_path, target.type_name
        )));
    }
    let mut signature = executable.signature().clone();
    signature.name = method_name.to_string();
    Ok(Some(signature))
}

fn entry_operation_spec(
    runtime_target: &str,
    target: &EntryTarget<'_>,
    method: &EntryFunctionSignature,
) -> EntryOperationSpec {
    let operation = format!("{}.{}", target.type_name, method.name);
    EntryOperationSpec {
        target: runtime_target.to_string(),
        operation,
        implementation_module: target.module_path.clone(),
        callable: EntryOperationCallable::ImplMethod {
            type_name: target.type_name.clone(),
            method: method.name.clone(),
        },
        params: method.params.clone(),
        return_type: method.return_type.clone(),
    }
}

fn push_entry_operation(
    artifacts: &mut EntryPointArtifacts,
    spec: EntryOperationSpec,
    _service_id: &str,
    _service_version: &str,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    protocol_identity: &str,
) {
    artifacts.artifact_operations.push(ArtifactOperation {
        operation: spec.operation.clone(),
        target: Some(spec.target.clone()),
        function: spec.operation.clone(),
        parameters: spec.params.iter().map(|param| param.name.clone()).collect(),
    });
    let mode = operation_response_mode(&spec.return_type);
    let response_type_ir = response_type_ir(&spec.return_type);
    artifacts.runtime_operations.push(RuntimeOperationManifest {
        operation: spec.operation.clone(),
        operation_abi_id: entry_operation_abi_id(&spec.operation, &spec.params, &spec.return_type),
        target: spec.target.clone(),
        mode,
        parameters: spec
            .params
            .iter()
            .map(|parameter| RuntimeOperationParameter {
                name: parameter.name.clone(),
                schema: contract_projection.schema_for_source_type_ref(
                    projection_index,
                    &spec.implementation_module,
                    &parameter.ty.ir,
                ),
            })
            .collect(),
        response: contract_projection.schema_for_source_type_ref(
            projection_index,
            &spec.implementation_module,
            &response_type_ir,
        ),
        service_protocol_identity: protocol_identity.to_string(),
    });
    artifacts.service_operations.push(spec);
}

fn reject_duplicate_entry_operation(
    operation: &str,
    used_operations: &mut BTreeSet<String>,
) -> Result<(), ProjectionError> {
    if used_operations.insert(operation.to_string()) {
        return Ok(());
    }
    Err(entry_error(format!(
        "entry operation {operation} conflicts with an existing service operation"
    )))
}

fn operation_response_mode(return_type: &EntryTypeSpec) -> String {
    match &return_type.ir {
        TypeRefIr::Native { name, args } if name == "Stream" && args.len() == 1 => {
            "serverStream".to_string()
        }
        _ => "unary".to_string(),
    }
}

pub fn entry_operation_abi_id(
    public_path: &str,
    params: &[EntryParamSpec],
    return_type: &EntryTypeSpec,
) -> String {
    let public_signature = CanonicalPublicCallableSignature {
        params: params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: param.ty.ir.clone(),
            })
            .collect(),
        return_type: return_type.ir.clone(),
        may_suspend: operation_response_mode(return_type) == "serverStream",
    };
    public_function_operation_abi_id(public_path, &public_signature, &[], &Default::default())
}

fn validate_http_handle(
    _target: &EntryTarget<'_>,
    method: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    let params = &method.params;
    if params.len() != 1
        || !is_http_request_type(&params[0].ty.name)
        || !is_http_response_type(&method.return_type.name)
    {
        return Err(entry_error(
            "http entry method handle must be handle(request: HttpRequest) -> HttpResponse"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_websocket_connect(
    method: &EntryFunctionSignature,
) -> Result<EntryTypeSpec, ProjectionError> {
    let Some(ir) = websocket_connect_context_type_ir(&method.return_type.ir) else {
        return Err(entry_error(format!(
            "websocket entry connect method must return WebSocketConnectResult<T>, found {}",
            method.return_type.name
        )));
    };
    let name = entry_function_type_ref_source_text(method, &ir);
    Ok(EntryTypeSpec {
        name,
        ir,
        local_type_names: method.local_type_names.clone(),
    })
}

fn validate_websocket_receive(method: &EntryFunctionSignature) -> Result<(), ProjectionError> {
    let return_type = normalize_type_name(&method.return_type.name);
    if return_type == "null" || return_type == "void" {
        return Ok(());
    }
    Err(entry_error(format!(
        "websocket entry receive method must return null or void, found {}",
        method.return_type.name
    )))
}

fn validate_websocket_connect_event_function(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<EntryTypeSpec, ProjectionError> {
    validate_websocket_connect_event_request_shape(source, function)?;
    validate_websocket_connect(function).map_err(|_| {
        entry_error(format!(
            "websocket connect handler {source} must return std.websocket.WebSocketConnectResult<C>"
        ))
    })
}

fn validate_websocket_connect_event_shape(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    validate_websocket_connect_event_request_shape(source, function)?;
    if !is_gateway_connect_result_type(&function.return_type.name) {
        return Err(entry_error(format!(
            "websocket connect handler {source} must return std.websocket.WebSocketConnectResult<C>"
        )));
    }
    Ok(())
}

fn validate_websocket_connect_event_request_shape(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    if function.params.len() != 1
        || function.params[0].name != "request"
        || !is_websocket_connect_request_type(&function.params[0].ty.name)
    {
        return Err(entry_error(format!(
            "websocket connect handler {source} must be function(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<C>"
        )));
    }
    Ok(())
}

fn websocket_connect_context_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Native { name, args }
            if args.len() == 1 && is_websocket_connect_result_type_ir_name(name) =>
        {
            Some(args[0].clone())
        }
        _ => None,
    }
}

fn is_websocket_connect_result_type_ir_name(name: &str) -> bool {
    matches!(
        normalize_type_name(name).as_str(),
        "WebSocketConnectResult" | "std.websocket.WebSocketConnectResult"
    )
}

fn validate_websocket_receive_event_function(
    source: &str,
    source_module: &str,
    function: &EntryFunctionSignature,
    context: &WebSocketContextArtifact,
) -> Result<(), ProjectionError> {
    let Some(receive_context) = validate_websocket_receive_event_ir_shape(source, function)? else {
        return Err(entry_error(format!(
            "websocket receive handler {source} must be function(event: std.websocket.WebSocketReceiveEvent<C>) -> null/void"
        )));
    };
    if !websocket_context_type_ir_matches(
        &context.source_module,
        &context.ty.ir,
        source_module,
        &receive_context,
    ) {
        return Err(entry_error(format!(
            "websocket receive handler {source} event context type {} must match connect context {}",
            entry_function_type_ref_source_text(function, &receive_context),
            context.ty.name
        )));
    }
    Ok(())
}

fn validate_websocket_receive_event_ir_shape(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<Option<TypeRefIr>, ProjectionError> {
    validate_websocket_receive(function)?;
    if function.params.len() == 1 && function.params[0].name == "event" {
        return Ok(websocket_receive_event_context_type_ir(
            &function.params[0].ty.ir,
        ));
    }
    Err(entry_error(format!(
        "websocket receive handler {source} must be function(event: std.websocket.WebSocketReceiveEvent<C>) -> null/void"
    )))
}

fn websocket_receive_event_context_type_ir(ty: &TypeRefIr) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Native { name, args }
            if args.len() == 1 && is_websocket_receive_event_root(name) =>
        {
            Some(args[0].clone())
        }
        _ => None,
    }
}

fn websocket_context_type_ir_matches(
    expected_module: &str,
    expected: &TypeRefIr,
    actual_module: &str,
    actual: &TypeRefIr,
) -> bool {
    match (expected, actual) {
        (
            TypeRefIr::Native {
                name: expected_name,
                args: expected_args,
            },
            TypeRefIr::Native {
                name: actual_name,
                args: actual_args,
            },
        ) => {
            normalize_type_name(expected_name) == normalize_type_name(actual_name)
                && expected_args.len() == actual_args.len()
                && expected_args
                    .iter()
                    .zip(actual_args)
                    .all(|(expected_arg, actual_arg)| {
                        websocket_context_type_ir_matches(
                            expected_module,
                            expected_arg,
                            actual_module,
                            actual_arg,
                        )
                    })
        }
        (
            TypeRefIr::LocalType {
                type_index: expected_index,
            },
            TypeRefIr::LocalType {
                type_index: actual_index,
            },
        ) => expected_module == actual_module && expected_index == actual_index,
        (
            TypeRefIr::Record {
                fields: expected_fields,
            },
            TypeRefIr::Record {
                fields: actual_fields,
            },
        ) => {
            expected_fields.len() == actual_fields.len()
                && expected_fields.iter().all(|(field, expected_ty)| {
                    actual_fields.get(field).is_some_and(|actual_ty| {
                        websocket_context_type_ir_matches(
                            expected_module,
                            expected_ty,
                            actual_module,
                            actual_ty,
                        )
                    })
                })
        }
        (
            TypeRefIr::Union {
                items: expected_items,
            },
            TypeRefIr::Union {
                items: actual_items,
            },
        ) => {
            expected_items.len() == actual_items.len()
                && expected_items
                    .iter()
                    .zip(actual_items)
                    .all(|(expected_item, actual_item)| {
                        websocket_context_type_ir_matches(
                            expected_module,
                            expected_item,
                            actual_module,
                            actual_item,
                        )
                    })
        }
        (
            TypeRefIr::Nullable {
                inner: expected_inner,
            },
            TypeRefIr::Nullable {
                inner: actual_inner,
            },
        ) => websocket_context_type_ir_matches(
            expected_module,
            expected_inner,
            actual_module,
            actual_inner,
        ),
        (
            TypeRefIr::Function {
                params: expected_params,
                return_type: expected_return,
            },
            TypeRefIr::Function {
                params: actual_params,
                return_type: actual_return,
            },
        ) => {
            expected_params.len() == actual_params.len()
                && expected_params.iter().zip(actual_params).all(
                    |(expected_param, actual_param)| {
                        expected_param.name == actual_param.name
                            && websocket_context_type_ir_matches(
                                expected_module,
                                &expected_param.ty,
                                actual_module,
                                &actual_param.ty,
                            )
                    },
                )
                && websocket_context_type_ir_matches(
                    expected_module,
                    expected_return,
                    actual_module,
                    actual_return,
                )
        }
        _ => expected == actual,
    }
}

fn validate_websocket_receive_event_shape(
    source: &str,
    function: &EntryFunctionSignature,
) -> Result<(), ProjectionError> {
    validate_websocket_receive(function)?;
    if function.params.len() == 1
        && function.params[0].name == "event"
        && websocket_event_type_args(&function.params[0].ty.name, WebSocketEventType::Receive)
            .is_some_and(|args| args.len() == 1)
    {
        return Ok(());
    }
    Err(entry_error(format!(
        "websocket receive handler {source} must be function(event: std.websocket.WebSocketReceiveEvent<C>) -> null/void"
    )))
}

#[derive(Debug, Clone, Copy)]
enum WebSocketEventType {
    Receive,
}

fn websocket_event_type_args(value: &str, expected: WebSocketEventType) -> Option<Vec<String>> {
    let value = normalize_type_name(value);
    let GenericParts { root, args, .. } = generic_parts(&value)?;
    let matches_root = match expected {
        WebSocketEventType::Receive => is_websocket_receive_event_root(root),
    };
    matches_root.then(|| args.into_iter().map(ToString::to_string).collect())
}

fn websocket_receive_event_adapter_args(
    params: &[EntryParamSpec],
) -> Vec<RuntimeGatewayAdapterArgManifest> {
    params
        .iter()
        .map(|param| {
            gateway_adapter_arg(
                &param.name,
                RuntimeGatewayAdapterSourceManifest::WebSocketReceiveEvent,
            )
        })
        .collect()
}

fn websocket_connect_adapter_args(
    params: &[EntryParamSpec],
) -> Result<Vec<RuntimeGatewayAdapterArgManifest>, ProjectionError> {
    if params.len() != 1
        || params[0].name != "request"
        || !is_websocket_connect_request_type(&params[0].ty.name)
    {
        return Err(entry_error(
            "websocket entry connect method must be connect(request: std.websocket.WebSocketConnectRequest) -> std.websocket.WebSocketConnectResult<C>"
                .to_string(),
        ));
    }
    Ok(vec![gateway_adapter_arg(
        &params[0].name,
        RuntimeGatewayAdapterSourceManifest::WebSocketConnectRequest,
    )])
}

fn websocket_receive_adapter_args(
    params: &[EntryParamSpec],
    context_type: Option<&EntryTypeSpec>,
) -> Result<Vec<RuntimeGatewayAdapterArgManifest>, ProjectionError> {
    let mut adapter_args = Vec::new();
    let mut saw_message = false;
    let mut violations = Vec::new();
    for param in params {
        let ty = normalize_type_name(&param.ty.name);
        let source = if websocket_receive_event_context_type_ir(&param.ty.ir).is_some() {
            saw_message = true;
            RuntimeGatewayAdapterSourceManifest::WebSocketReceiveEvent
        } else if is_websocket_connection_type_ir(&param.ty.ir) {
            RuntimeGatewayAdapterSourceManifest::WebSocketConnection
        } else if context_type.is_some_and(|context_type| type_refs_match(&param.ty, context_type))
        {
            RuntimeGatewayAdapterSourceManifest::WebSocketConnectionContext
        } else if is_connection_message_type(&ty) {
            saw_message = true;
            RuntimeGatewayAdapterSourceManifest::WebSocketMessage
        } else {
            saw_message = true;
            RuntimeGatewayAdapterSourceManifest::WebSocketMessageBody
        };
        adapter_args.push(gateway_adapter_arg(&param.name, source));
    }
    if !saw_message {
        violations.push(
            "websocket entry receive must include a message, message body, or receive event parameter"
                .to_string(),
        );
    }
    if violations.is_empty() {
        Ok(adapter_args)
    } else {
        violations.sort();
        violations.dedup();
        Err(ProjectionError::ContractValidation {
            message: violations
                .into_iter()
                .map(|violation| format!("- {violation}"))
                .collect::<Vec<_>>()
                .join("\n"),
        })
    }
}

fn is_websocket_connection_type_ir(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Native { name, args } => {
            args.len() == 1
                && matches!(
                    normalize_type_name(name).as_str(),
                    "WebSocketConnection" | "std.websocket.WebSocketConnection"
                )
        }
        _ => false,
    }
}

fn websocket_connect_operation_name(module_path: &str, symbol: &str) -> String {
    format!("websocket.connect.{module_path}.{symbol}")
}

fn websocket_receive_operation_name(module_path: &str, symbol: &str) -> String {
    format!("websocket.receive.{module_path}.{symbol}")
}

fn websocket_package_operation_name(
    kind: WebSocketHandlerKind,
    alias: &str,
    symbol_path: &str,
) -> String {
    let kind = match kind {
        WebSocketHandlerKind::Connect => "connect",
        WebSocketHandlerKind::Receive => "receive",
    };
    format!("websocket.{kind}.package.{alias}.{symbol_path}")
}

fn websocket_target_name(
    service_target_component: &str,
    kind: &str,
    module_path: &str,
    symbol: &str,
) -> String {
    format!("entry.{service_target_component}.websocket.{kind}.{module_path}.{symbol}")
}

fn find_entry_target<'a>(
    entrypoints: &'a ProjectionSyntheticEntrypointIndex,
    field: &str,
    target_text: &str,
) -> Result<EntryTarget<'a>, ProjectionError> {
    let Some((module_path, type_name)) = target_text.rsplit_once('.') else {
        return Err(entry_error(format!(
            "{field} entry target {target_text}: expected module.TypeName"
        )));
    };
    let Some(module) = entrypoints.module(module_path) else {
        return Err(entry_error(format!(
            "{field} entry target {target_text}: module {module_path} not found in compiled publication"
        )));
    };
    if !module.has_type(type_name) {
        return Err(entry_error(format!(
            "{field} entry target {target_text}: type {type_name} not found in module {module_path}"
        )));
    }
    Ok(EntryTarget {
        module_path: module_path.to_string(),
        type_name: type_name.to_string(),
        module,
    })
}

impl EntrypointFunctionIndex for ProjectionSyntheticEntrypointIndex {
    fn find_function_target(
        &self,
        field: &str,
        source_text: &str,
        module_path: &str,
        symbol: &str,
    ) -> Result<FunctionTarget, ProjectionError> {
        let Some(module) = self.module(module_path) else {
            return Err(entry_error(format!(
                "{field} {source_text}: module {module_path} not found in compiled publication"
            )));
        };
        let Some(executable) = module.executable(symbol) else {
            return Err(entry_error(format!(
                "{field} {source_text}: function {symbol} not found in module {module_path}"
            )));
        };
        match executable.kind() {
            ProjectionSyntheticEntrypointExecutableKind::Function
            | ProjectionSyntheticEntrypointExecutableKind::ImplMethod => {}
        }
        Ok(FunctionTarget {
            source: source_text.to_string(),
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
            function: executable.signature().clone(),
        })
    }
}

pub(super) fn entry_error(message: String) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: format!("- {message}"),
    }
}

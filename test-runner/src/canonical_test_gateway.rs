use skiff_artifact_identity::gateway_entry_identity;
use skiff_artifact_model::{
    DeploymentGatewayEntry, GatewayAdapterArg, GatewayAdapterKind, GatewayAdapterPlan,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryProtocolSurface,
    GatewayExternalErrorProjection, GatewayExternalSchema, GatewayHttpProtocolSurface,
    GatewayProtocolSurface, OperationCallableKind, PackageArtifact, PackageLocalAbiSymbol,
    PackageTypeRef, TypeRefIr,
};

pub(crate) fn canonical_typed_null_gateway(
    implementation: &PackageArtifact,
    handler_selector: &str,
    response_schema: GatewayExternalSchema,
) -> Result<DeploymentGatewayEntry, String> {
    let symbol = implementation
        .package_local_abi
        .implementation_symbols
        .get(handler_selector)
        .ok_or_else(|| {
            format!("implementationSymbols has no exact private gateway handler {handler_selector}")
        })?;
    let PackageLocalAbiSymbol::Callable {
        callable_id,
        signature,
    } = symbol
    else {
        return Err(format!(
            "private gateway handler {handler_selector} is not a top-level function"
        ));
    };
    if !signature.type_params.is_empty()
        || signature.parameters.len() != 1
        || signature.parameters[0].name != "body"
        || !is_builtin(&signature.parameters[0].ty, "null")
        || !schema_matches_type(&response_schema, &signature.return_type)
    {
        return Err(format!(
            "private gateway handler {handler_selector} must have exact signature (body: null) -> {}",
            schema_type_name(&response_schema)
        ));
    }
    let link = implementation
        .callable_links
        .get(callable_id)
        .ok_or_else(|| format!("gateway handler {callable_id} has no exact callableLinks entry"))?;
    if link.callable_id != *callable_id
        || link.target.callable_abi_id != callable_id.as_str()
        || link.target.callable_kind != OperationCallableKind::InternalFunction
        || !implementation.files.iter().any(|file| {
            file.file_ir_identity == link.target.file_ref.file_ir_identity
                && file.module_path == link.target.file_ref.module_path
                && file.source_ast_hash == link.target.file_ref.source_ast_hash
        })
        || !implementation
            .callable_semantic_facts
            .contains_key(callable_id)
    {
        return Err(format!(
            "gateway handler {callable_id} is not an exact private InternalFunction"
        ));
    }
    if implementation
        .package_local_abi
        .public_symbols
        .values()
        .filter_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable { callable_id, .. } => {
                implementation.callable_links.get(callable_id)
            }
            _ => None,
        })
        .any(|public| {
            public.target.file_ref.file_ir_identity == link.target.file_ref.file_ir_identity
                && public.target.executable_index == link.target.executable_index
        })
    {
        return Err(format!(
            "gateway handler {handler_selector} must not enter the package public API"
        ));
    }

    let protocol_surface = GatewayEntryProtocolSurface {
        protocol: GatewayProtocolSurface::Http(GatewayHttpProtocolSurface {
            adapter_kind: GatewayAdapterKind::TypedJson,
            dispatch_mode: GatewayDispatchMode::Unary,
            external_sources: vec![GatewayAdapterSource::HttpBody],
            request_body_schema: Some(GatewayExternalSchema::Null),
            response_schema: Some(response_schema),
            stream_item_schema: None,
        }),
        external_error_projection: GatewayExternalErrorProjection::FIXED_V1,
    };
    let identity = gateway_entry_identity(&protocol_surface).map_err(|error| error.to_string())?;
    Ok(DeploymentGatewayEntry {
        gateway_entry_identity: identity,
        protocol_surface,
        handler: callable_id.clone(),
        pre: None,
        guard: None,
        adapter_plan: GatewayAdapterPlan {
            kind: GatewayAdapterKind::TypedJson,
            args: vec![GatewayAdapterArg {
                param: "body".to_string(),
                source: GatewayAdapterSource::HttpBody,
            }],
        },
    })
}

fn is_builtin(ty: &PackageTypeRef, expected: &str) -> bool {
    matches!(
        ty,
        PackageTypeRef::Local {
            local_type: TypeRefIr::Builtin { name, args }
        } if name == expected && args.is_empty()
    )
}

fn schema_matches_type(schema: &GatewayExternalSchema, ty: &PackageTypeRef) -> bool {
    match schema {
        GatewayExternalSchema::Null => is_builtin(ty, "null"),
        GatewayExternalSchema::String => is_builtin(ty, "string"),
        _ => false,
    }
}

fn schema_type_name(schema: &GatewayExternalSchema) -> &'static str {
    match schema {
        GatewayExternalSchema::Null => "null",
        GatewayExternalSchema::String => "string",
        _ => "the requested external schema",
    }
}

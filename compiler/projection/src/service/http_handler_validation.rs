use std::collections::BTreeMap as StdBTreeMap;

use crate::error::ProjectionError;
use crate::package_unit_artifacts::ProjectedPackageIrArtifacts;
use skiff_artifact_model::{ExecutableSignatureIr, TypeRefIr};
use skiff_compiler_projection_input::{ServiceIngressHandlerProjection, ServiceIngressProjection};

pub struct HttpHandlerValidationInput<'a> {
    pub ingress: &'a ServiceIngressProjection,
    pub package_artifacts: &'a [ProjectedPackageIrArtifacts],
}

pub fn validate_http_route_package_handlers(
    input: HttpHandlerValidationInput<'_>,
) -> Result<(), ProjectionError> {
    validate_http_route_package_handlers_inner(input.ingress, input.package_artifacts)
}

fn validate_http_route_package_handlers_inner(
    service_ingress: &ServiceIngressProjection,
    packages: &[ProjectedPackageIrArtifacts],
) -> Result<(), ProjectionError> {
    let Some(http) = service_ingress.http() else {
        return Ok(());
    };
    if http.guard.is_none() && http.routes.is_empty() {
        return Ok(());
    }

    let package_by_id = packages
        .iter()
        .map(|package| (package.unit.package_id.as_str(), package))
        .collect::<StdBTreeMap<_, _>>();

    if let Some(ServiceIngressHandlerProjection::PackageFunction {
        source,
        package_id,
        symbol_path,
        ..
    }) = &http.guard
    {
        let signature = package_http_function_signature(
            &package_by_id,
            package_id,
            symbol_path,
            "http guard",
            source,
        )?;
        if signature.self_type.is_some()
            || signature.params.len() != 1
            || !is_http_request_type_ref(&signature.params[0].ty)
            || !is_nullable_http_response_type_ref(&signature.return_type)
        {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "http guard {} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse?",
                    source
                ),
            });
        }
    }

    for route in &http.routes {
        let ServiceIngressHandlerProjection::PackageFunction {
            source,
            package_id,
            symbol_path,
            ..
        } = &route.handler
        else {
            continue;
        };
        let signature = package_http_function_signature(
            &package_by_id,
            package_id,
            symbol_path,
            &format!("http route {} handler", route.path),
            source,
        )?;
        if !is_valid_http_package_route_signature(&signature) {
            return Err(ProjectionError::ContractValidation {
                message: format!(
                    "http route {} handler {} must be function(request: std.http.HttpRequest) -> std.http.HttpResponse or a typed JSON HTTP route handler",
                    route.path, source
                ),
            });
        }
    }

    Ok(())
}

fn package_http_function_signature(
    package_by_id: &StdBTreeMap<&str, &ProjectedPackageIrArtifacts>,
    package_id: &str,
    symbol_path: &str,
    field: &str,
    source: &str,
) -> Result<ExecutableSignatureIr, ProjectionError> {
    let Some(package) = package_by_id.get(package_id) else {
        return Err(ProjectionError::ContractValidation {
            message: format!(
                "{field} {source} references package {package_id} but no package unit was produced"
            ),
        });
    };
    let Some(function) = package.unit.implementation_links.functions.get(symbol_path) else {
        return Err(ProjectionError::ContractValidation {
            message: format!("{field} {source} references missing package function {symbol_path}"),
        });
    };
    Ok(function.signature.clone())
}

fn is_http_request_type_ref(ty: &TypeRefIr) -> bool {
    is_http_envelope_type_ref(ty, "HttpRequest")
}

fn is_valid_http_package_route_signature(signature: &ExecutableSignatureIr) -> bool {
    if signature.self_type.is_some() {
        return false;
    }
    if signature.params.len() == 1
        && is_http_request_type_ref(&signature.params[0].ty)
        && is_http_response_type_ref(&signature.return_type)
    {
        return true;
    }
    signature.params.len() <= 2
        && !signature
            .params
            .first()
            .is_some_and(|param| is_http_request_type_ref(&param.ty))
        && !is_http_response_type_ref(&signature.return_type)
        && !is_http_response_stream_type_ref(&signature.return_type)
        && !is_void_type_ref(&signature.return_type)
}

fn is_http_response_type_ref(ty: &TypeRefIr) -> bool {
    is_http_envelope_type_ref(ty, "HttpResponse")
}

fn is_http_response_stream_type_ref(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Native { name, args }
            if name == "Stream"
                && args.len() == 1
                && is_http_envelope_type_ref(&args[0], "HttpResponseStreamEvent")
    )
}

fn is_void_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if name == "void" && args.is_empty())
}

fn is_nullable_http_response_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Nullable { inner } if is_http_response_type_ref(inner))
}

fn is_http_envelope_type_ref(ty: &TypeRefIr, expected: &str) -> bool {
    match ty {
        TypeRefIr::Native { name, .. } => {
            name == expected || name == &format!("std.http.{expected}")
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            (symbol.module_path.is_empty() && symbol.symbol == expected)
                || (symbol.module_path == "std.http" && symbol.symbol == expected)
                || symbol.symbol == format!("std.http.{expected}")
        }
        TypeRefIr::PackageSymbol { symbol } => {
            symbol.symbol_path == expected
                || symbol.symbol_path == format!("http.{expected}")
                || symbol.symbol_path == format!("std.http.{expected}")
        }
        _ => false,
    }
}

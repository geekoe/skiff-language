use std::collections::BTreeMap;

use crate::{
    contract::{
        ContractPackageRefKey, ContractProjection, ContractProjectionIndex, ContractTypeKey,
    },
    error::ProjectionError,
    runtime_manifest_model::ArtifactOperation,
    runtime_manifest_model::{
        RuntimeHttpRawGatewayManifest, RuntimeOperationManifest, RuntimeOperationParameter,
        RUNTIME_OPERATION_MODE_SERVER_STREAM, RUNTIME_OPERATION_MODE_UNARY,
    },
    typed_artifacts::{
        public_function_operation_abi_id, PublicInstanceExport, PublicInstanceOperation,
    },
};
use skiff_artifact_model::{
    interface_instantiation_ref, CanonicalPublicCallableSignature, FunctionTypeParamIr, LiteralIr,
    PackageRefIr, PackageSymbolRef, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::prelude_registry::PRELUDE_REGISTRY_ID;
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref;

pub fn build_artifact_operations(
    service_target_component: &str,
    contract: &ContractProjection,
) -> Vec<ArtifactOperation> {
    contract.artifact_operations(service_target_component)
}

pub fn build_public_instance_artifact_operations(
    service_target_component: &str,
    _contract: &ContractProjection,
    public_instances: &[PublicInstanceExport],
) -> Result<Vec<ArtifactOperation>, ProjectionError> {
    public_instance_operations(public_instances)
        .map(|(_instance, operation)| {
            Ok(ArtifactOperation {
                operation: operation.operation.public_path.clone(),
                target: Some(format!(
                    "service.{service_target_component}.{}",
                    operation.operation.public_path
                )),
                function: public_instance_operation_executable_symbol(operation),
                parameters: Vec::new(),
            })
        })
        .collect()
}

pub fn build_runtime_operations(
    _service_id: &str,
    _service_version: &str,
    operations: &[ArtifactOperation],
    contract: &ContractProjection,
    protocol_identity: &str,
) -> Vec<RuntimeOperationManifest> {
    operations
        .iter()
        .filter_map(|artifact_operation| {
            let (interface_name, method_name) =
                contract.split_operation_name(&artifact_operation.operation)?;
            let interface = contract.interfaces.get(interface_name)?;
            let operation = interface
                .operations
                .iter()
                .find(|operation| operation.name == method_name)?;
            let (mode, response_type) =
                projection_operation_response_mode_and_type(&operation.return_type);
            Some(RuntimeOperationManifest {
                operation: artifact_operation.operation.clone(),
                operation_abi_id: contract_public_function_operation_abi_id(
                    &artifact_operation.operation,
                    operation,
                ),
                target: artifact_operation.target.clone().unwrap(),
                mode,
                parameters: operation
                    .params
                    .iter()
                    .map(|parameter| RuntimeOperationParameter {
                        name: parameter.name.clone(),
                        schema: contract.schema_for_type_key(&parameter.ty),
                    })
                    .collect(),
                response: contract.schema_for_type_key(&response_type),
                service_protocol_identity: protocol_identity.to_string(),
            })
        })
        .collect()
}

pub fn build_public_instance_runtime_operations(
    _service_id: &str,
    _service_version: &str,
    service_target_component: &str,
    public_instances: &[PublicInstanceExport],
    contract: &ContractProjection,
    projection_index: Option<&ContractProjectionIndex<'_>>,
    protocol_identity: &str,
) -> Result<Vec<RuntimeOperationManifest>, ProjectionError> {
    public_instance_operations(public_instances)
        .map(|(instance, operation)| {
            let (mode, parameters, response) =
                public_instance_runtime_surface(instance, operation, contract, projection_index)?;
            Ok(RuntimeOperationManifest {
                operation: operation.operation.public_path.clone(),
                operation_abi_id: runtime_operation_abi_id(instance, operation),
                target: format!(
                    "service.{service_target_component}.{}",
                    operation.operation.public_path
                ),
                mode,
                parameters,
                response,
                service_protocol_identity: protocol_identity.to_string(),
            })
        })
        .collect()
}

pub fn raw_http_gateway_operation(
    service_target_component: &str,
    contract: &ContractProjection,
    operations: &[RuntimeOperationManifest],
) -> Result<Option<RuntimeHttpRawGatewayManifest>, ProjectionError> {
    let matches = operations
        .iter()
        .filter(|operation| is_raw_http_operation(operation, contract))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Ok(None),
        [operation] => Ok(Some(RuntimeHttpRawGatewayManifest {
            operation: operation.operation.clone(),
            target: format!("gateway.{service_target_component}.http.raw"),
        })),
        _ => Err(ProjectionError::ContractValidation {
            message: format!(
                "- raw HTTP dispatch requires at most one HttpRequest -> HttpResponse operation; found {}",
                matches
                    .iter()
                    .map(|operation| operation.operation.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }),
    }
}

fn is_raw_http_operation(
    operation: &RuntimeOperationManifest,
    contract: &ContractProjection,
) -> bool {
    let Some(contract_operation) = contract.operation(&operation.operation) else {
        return false;
    };
    operation.mode == RUNTIME_OPERATION_MODE_UNARY
        && operation.parameters.len() == 1
        && contract_operation.params.len() == 1
        && is_projection_http_request_type(contract, &contract_operation.params[0].ty)
        && is_projection_http_response_type(contract, &contract_operation.return_type)
}

fn projection_operation_response_mode_and_type(
    return_type: &ContractTypeKey,
) -> (String, ContractTypeKey) {
    match return_type {
        ContractTypeKey::Builtin { name, args }
            if bare_type_name(name) == "Stream" && args.len() == 1 =>
        {
            ("serverStream".to_string(), args[0].clone())
        }
        _ => (
            RUNTIME_OPERATION_MODE_UNARY.to_string(),
            return_type.clone(),
        ),
    }
}

fn public_instance_operations(
    public_instances: &[PublicInstanceExport],
) -> impl Iterator<Item = (&PublicInstanceExport, &PublicInstanceOperation)> {
    public_instances.iter().flat_map(|instance| {
        instance
            .operations
            .iter()
            .map(move |operation| (instance, operation))
    })
}

pub fn runtime_operation_abi_id(
    instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
) -> String {
    let _ = instance;
    operation.operation.operation_abi_id.clone()
}

pub fn contract_public_function_operation_abi_id(
    public_path: &str,
    operation: &crate::contract::ContractInterfaceOperationProjection,
) -> String {
    let public_signature = CanonicalPublicCallableSignature {
        params: operation
            .params
            .iter()
            .map(|param| FunctionTypeParamIr {
                name: param.name.clone(),
                ty: contract_type_key_to_type_ref(&param.ty),
            })
            .collect(),
        return_type: contract_type_key_to_type_ref(&operation.return_type),
        may_suspend: matches!(
            projection_operation_response_mode_and_type(&operation.return_type)
                .0
                .as_str(),
            RUNTIME_OPERATION_MODE_SERVER_STREAM
        ),
    };
    public_function_operation_abi_id(public_path, &public_signature, &[], &Default::default())
}

fn contract_type_key_to_type_ref(ty: &ContractTypeKey) -> TypeRefIr {
    match ty {
        ContractTypeKey::Builtin { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args.iter().map(contract_type_key_to_type_ref).collect(),
        },
        ContractTypeKey::Named(name) => TypeRefIr::ServiceSymbol {
            symbol: contract_named_type_key_symbol(name),
        },
        ContractTypeKey::PackageSymbol {
            package,
            symbol_path,
            abi_expectation,
        } => TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: contract_package_ref_key_to_ref(package),
                symbol_path: symbol_path.clone(),
                abi_expectation: abi_expectation.clone(),
            },
        },
        ContractTypeKey::AnyInterface {
            interface,
            canonical_type_args,
        } => TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(
                contract_type_key_to_type_ref(interface),
                canonical_type_args
                    .iter()
                    .map(contract_type_key_to_type_ref)
                    .collect(),
            ),
        },
        ContractTypeKey::DbObjectSymbol {
            module_path,
            symbol,
        } => TypeRefIr::DbObjectSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.clone(),
                symbol: symbol.clone(),
            },
        },
        ContractTypeKey::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), contract_type_key_to_type_ref(ty)))
                .collect(),
        },
        ContractTypeKey::Union { items } => TypeRefIr::Union {
            items: items.iter().map(contract_type_key_to_type_ref).collect(),
        },
        ContractTypeKey::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(contract_type_key_to_type_ref(inner)),
        },
        ContractTypeKey::Literal(literal) => TypeRefIr::Literal {
            value: contract_literal_key_to_literal(literal),
        },
        ContractTypeKey::TypeParam { name } => TypeRefIr::TypeParam { name: name.clone() },
        ContractTypeKey::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: contract_type_key_to_type_ref(&param.ty),
                })
                .collect(),
            return_type: Box::new(contract_type_key_to_type_ref(return_type)),
        },
    }
}

fn contract_named_type_key_symbol(
    name: &crate::contract::ContractNamedTypeKey,
) -> ServiceSymbolRef {
    match name {
        crate::contract::ContractNamedTypeKey::Public { symbol } => ServiceSymbolRef {
            module_path: String::new(),
            symbol: symbol.clone(),
        },
        crate::contract::ContractNamedTypeKey::Source { source } => ServiceSymbolRef {
            module_path: source.module_path().to_string(),
            symbol: source.symbol().to_string(),
        },
    }
}

fn contract_package_ref_key_to_ref(package: &ContractPackageRefKey) -> PackageRefIr {
    match package {
        ContractPackageRefKey::PackageId { package_id } => PackageRefIr::PackageId {
            package_id: package_id.clone(),
        },
        ContractPackageRefKey::Dependency { dependency_ref } => PackageRefIr::Dependency {
            dependency_ref: dependency_ref.clone(),
        },
    }
}

fn contract_literal_key_to_literal(literal: &crate::contract::ContractLiteralKey) -> LiteralIr {
    match literal {
        crate::contract::ContractLiteralKey::Null => LiteralIr::Null,
        crate::contract::ContractLiteralKey::Bool(value) => LiteralIr::Bool { value: *value },
        crate::contract::ContractLiteralKey::Number(value) => LiteralIr::Number {
            value: serde_json::from_str(value)
                .expect("contract number literal should be valid JSON number"),
        },
        crate::contract::ContractLiteralKey::String(value) => LiteralIr::String {
            value: value.clone(),
        },
    }
}

fn public_instance_runtime_surface(
    instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
    contract: &ContractProjection,
    projection_index: Option<&ContractProjectionIndex<'_>>,
) -> Result<
    (
        String,
        Vec<RuntimeOperationParameter>,
        crate::runtime_manifest_model::JsonSchema,
    ),
    ProjectionError,
> {
    if public_instance_interface_name(operation).is_some() {
        let (_interface_name, _method_name, method) =
            public_instance_contract_operation(contract, operation)?;
        let (mode, response_type) =
            projection_operation_response_mode_and_type(&method.return_type);
        return Ok((
            mode,
            method
                .params
                .iter()
                .map(|parameter| RuntimeOperationParameter {
                    name: parameter.name.clone(),
                    schema: contract.schema_for_type_key(&parameter.ty),
                })
                .collect(),
            contract.schema_for_type_key(&response_type),
        ));
    }

    if let Some(projection_index) = projection_index {
        if let Some((module_path, params, return_type)) =
            public_instance_source_interface_signature(operation, projection_index)
        {
            let (mode, response_type) = operation_response_mode_and_type(&return_type);
            return Ok((
                mode.to_string(),
                params
                    .iter()
                    .map(|parameter| RuntimeOperationParameter {
                        name: parameter.name.clone(),
                        schema: public_instance_source_schema(
                            contract,
                            Some(projection_index),
                            &module_path,
                            &parameter.ty,
                        ),
                    })
                    .collect(),
                public_instance_source_schema(
                    contract,
                    Some(projection_index),
                    &module_path,
                    &response_type,
                ),
            ));
        }

        // Package-interface public instance: no service-side interface decl to
        // project from, so recover the public surface from the bound receiver
        // impl method (self stripped), matching the serviceUnit route and the
        // operation ABI id.
        if let Some((module_path, params, return_type)) =
            public_instance_receiver_executable_signature(operation, projection_index)
        {
            let (mode, response_type) = operation_response_mode_and_type(&return_type);
            return Ok((
                mode.to_string(),
                params
                    .iter()
                    .map(|parameter| RuntimeOperationParameter {
                        name: parameter.name.clone(),
                        schema: public_instance_source_schema(
                            contract,
                            Some(projection_index),
                            &module_path,
                            &parameter.ty,
                        ),
                    })
                    .collect(),
                public_instance_source_schema(
                    contract,
                    Some(projection_index),
                    &module_path,
                    &response_type,
                ),
            ));
        }
    }

    if operation.operation.interface.is_some() {
        return Ok((
            RUNTIME_OPERATION_MODE_UNARY.to_string(),
            Vec::new(),
            public_instance_source_schema(
                contract,
                projection_index,
                &instance.module_path,
                &TypeRefIr::native("unit"),
            ),
        ));
    }

    Err(ProjectionError::ContractValidation {
        message: format!(
            "public instance operation {} interface must be a public service interface symbol or package interface symbol",
            operation.operation.public_path
        ),
    })
}

fn public_instance_source_schema(
    contract: &ContractProjection,
    projection_index: Option<&ContractProjectionIndex<'_>>,
    module_path: &str,
    ty: &TypeRefIr,
) -> crate::runtime_manifest_model::JsonSchema {
    projection_index
        .map(|index| contract.schema_for_source_type_ref(index, module_path, ty))
        .unwrap_or_else(crate::runtime_manifest_model::JsonSchema::any)
}

fn public_instance_contract_operation<'a>(
    contract: &'a ContractProjection,
    operation: &PublicInstanceOperation,
) -> Result<
    (
        String,
        String,
        &'a crate::contract::ContractInterfaceOperationProjection,
    ),
    ProjectionError,
> {
    let interface_name = public_instance_interface_name(operation).ok_or_else(|| {
        ProjectionError::ContractValidation {
            message: format!(
                "public instance operation {} interface must be a public service interface symbol",
                operation.operation.public_path
            ),
        }
    })?;
    let interface = contract.interfaces.get(&interface_name).ok_or_else(|| {
        ProjectionError::ContractValidation {
            message: format!(
                "public instance operation {} references missing interface {interface_name}",
                operation.operation.public_path
            ),
        }
    })?;
    let method_name = public_instance_operation_method_name(operation);
    let method = interface
        .operations
        .iter()
        .find(|candidate| candidate.name == method_name)
        .ok_or_else(|| ProjectionError::ContractValidation {
            message: format!(
                "public instance operation {} references missing method {}.{}",
                operation.operation.public_path, interface_name, method_name
            ),
        })?;
    Ok((interface_name, method_name, method))
}

pub fn public_instance_source_interface_signature(
    operation: &PublicInstanceOperation,
    projection_index: &ContractProjectionIndex<'_>,
) -> Option<(String, Vec<FunctionTypeParamIr>, TypeRefIr)> {
    let interface = operation.operation.interface.as_ref()?;
    let ty: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
    let TypeRefIr::ServiceSymbol { symbol } = ty else {
        return None;
    };
    if symbol.module_path.is_empty() {
        return None;
    }
    let interface_decl = projection_index
        .interface_decl_by_module_local_name(&symbol.module_path, &symbol.symbol)?;
    let method_name = public_instance_operation_method_name(operation);
    let method = interface_decl
        .operations
        .iter()
        .find(|candidate| candidate.name == method_name)?;
    let substitutions = interface_decl
        .type_params
        .iter()
        .cloned()
        .zip(interface.canonical_type_args.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    let params = method
        .params
        .iter()
        .filter(|param| param.name != "self")
        .map(|param| FunctionTypeParamIr {
            name: param.name.clone(),
            ty: substitute_type_params_in_type_ref(param.ty.clone(), &substitutions),
        })
        .collect();
    let return_type =
        substitute_type_params_in_type_ref(method.return_type.clone(), &substitutions);
    Some((symbol.module_path, params, return_type))
}

/// Recover a public-instance operation's public surface from its bound receiver
/// impl method when the implemented interface is a package interface (which the
/// projection index does not carry as a decl). Strips the explicit leading
/// `self` receiver so the result matches both the serviceUnit route and the
/// operation ABI id.
pub fn public_instance_receiver_executable_signature(
    operation: &PublicInstanceOperation,
    projection_index: &ContractProjectionIndex<'_>,
) -> Option<(String, Vec<FunctionTypeParamIr>, TypeRefIr)> {
    let target = &operation.receiver_executable.executable_target;
    let module_path = target.file_ref.module_path.as_str();
    let unit = projection_index.unit_by_module_path(module_path)?;
    let executable = unit.executables.get(target.executable_index as usize)?;
    let params = executable
        .params
        .iter()
        .skip(usize::from(
            executable
                .params
                .first()
                .is_some_and(|param| param.name == "self"),
        ))
        .map(|param| FunctionTypeParamIr {
            name: param.name.clone(),
            ty: param.ty.clone(),
        })
        .collect();
    Some((
        module_path.to_string(),
        params,
        executable.return_type.clone(),
    ))
}

fn public_instance_interface_name(operation: &PublicInstanceOperation) -> Option<String> {
    let interface = operation.operation.interface.as_ref()?;
    let ty: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
    match &ty {
        TypeRefIr::ServiceSymbol { symbol } if symbol.module_path.is_empty() => {
            Some(symbol.symbol.clone())
        }
        _ => None,
    }
}

fn public_instance_operation_method_name(operation: &PublicInstanceOperation) -> String {
    operation
        .operation
        .display_name
        .rsplit('.')
        .next()
        .filter(|method| !method.is_empty())
        .or_else(|| {
            operation
                .operation
                .public_path
                .rsplit('.')
                .next()
                .filter(|method| !method.is_empty())
        })
        .unwrap_or(operation.operation.operation_abi_id.as_str())
        .to_string()
}

fn public_instance_operation_executable_symbol(operation: &PublicInstanceOperation) -> String {
    let target = &operation.receiver_executable.executable_target;
    local_symbol_from_abi_id(
        "callable:",
        &target.file_ref.module_path,
        &target.callable_abi_id,
    )
    .unwrap_or_else(|| operation.operation.display_name.clone())
}

fn local_symbol_from_abi_id(prefix: &str, module_path: &str, abi_id: &str) -> Option<String> {
    let qualified = abi_id.strip_prefix(prefix)?;
    let qualified = qualified
        .split_once(':')
        .map_or(qualified, |(head, _)| head);
    let module_prefix = format!("{module_path}.");
    let local = qualified.strip_prefix(&module_prefix).unwrap_or(qualified);
    (!local.is_empty()).then(|| local.to_string())
}

fn bare_type_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

fn operation_response_mode_and_type(return_type: &TypeRefIr) -> (&'static str, TypeRefIr) {
    match return_type {
        TypeRefIr::Native { name, args } if bare_type_name(name) == "Stream" && args.len() == 1 => {
            (RUNTIME_OPERATION_MODE_SERVER_STREAM, args[0].clone())
        }
        _ => (RUNTIME_OPERATION_MODE_UNARY, return_type.clone()),
    }
}

fn is_projection_http_request_type(contract: &ContractProjection, ty: &ContractTypeKey) -> bool {
    is_projection_type_name(contract, ty, &["HttpRequest", "std.http.HttpRequest"])
}

fn is_projection_http_response_type(contract: &ContractProjection, ty: &ContractTypeKey) -> bool {
    is_projection_type_name(contract, ty, &["HttpResponse", "std.http.HttpResponse"])
}

fn is_projection_type_name(
    contract: &ContractProjection,
    ty: &ContractTypeKey,
    names: &[&str],
) -> bool {
    match ty {
        ContractTypeKey::Builtin { name, args } if args.is_empty() => {
            names.iter().any(|expected| name == expected)
        }
        ContractTypeKey::Named(name) => {
            let symbol = name.canonical_symbol_ref();
            names.iter().any(|expected| symbol.as_ref() == *expected)
        }
        ContractTypeKey::PackageSymbol {
            package,
            symbol_path,
            ..
        } => standard_library_package_symbol(contract, package, symbol_path)
            .is_some_and(|symbol| names.iter().any(|expected| symbol == *expected)),
        _ => false,
    }
}

fn standard_library_package_symbol(
    contract: &ContractProjection,
    package: &ContractPackageRefKey,
    symbol_path: &str,
) -> Option<String> {
    let is_standard_package = match package {
        ContractPackageRefKey::PackageId { package_id } => {
            package_id == PRELUDE_REGISTRY_ID || package_id == "skiff.run/std"
        }
        ContractPackageRefKey::Dependency { dependency_ref } => {
            dependency_ref == "std"
                || dependency_ref == PRELUDE_REGISTRY_ID
                || dependency_ref == "skiff.run/std"
        }
    };
    if !is_standard_package && !symbol_path.starts_with("std.") {
        return None;
    }

    contract
        .prelude()
        .known_type_symbol(symbol_path)
        .or_else(|| {
            contract
                .prelude()
                .known_type_symbol(&format!("std.{symbol_path}"))
        })
}

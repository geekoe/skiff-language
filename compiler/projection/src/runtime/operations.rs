use std::collections::BTreeMap;

use crate::{
    error::ProjectionError,
    runtime_manifest_model::ArtifactOperation,
    runtime_manifest_model::JsonSchema,
    typed_artifacts::{OperationParam, PublicInstanceExport, PublicInstanceOperation},
    {
        contract::{
            ContractFunctionParamProjection, ContractProjection, ContractProjectionIndex,
            ContractTypeKey,
        },
        contract_schema::descriptor::RuntimeTypeDescriptorIr,
    },
};
use serde::Serialize;
use skiff_artifact_model::{ExecutableKind, FileIrUnit, FunctionTypeParamIr, TypeRefIr};
use skiff_compiler_core::source_role::PublicationSourceRole as CompilerSourceRole;
use skiff_compiler_projection_input::{EntryParamSpec, ProjectionSourceMetadata};

use super::effect_summary_for_signature;
use super::entrypoints::entry_operation_abi_id;
use super::operation_effects::effect_summary;
use super::operation_effects::EffectSummary;
use super::service_operations::{
    contract_public_function_operation_abi_id, public_instance_receiver_executable_signature,
    public_instance_source_interface_signature, runtime_operation_abi_id,
};
use super::{response_type_ir, EntryOperationCallable, EntryOperationSpec};

/// A runtime-projected service operation entry. Structural fields are typed;
/// `return_type` and per-parameter `type` carry typed runtime descriptors,
/// while `response` and `schema` carry typed JSON schemas. Field order matches
/// the former `json!` construction so serialized bytes are unchanged.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationEntryIr {
    pub operation: String,
    pub operation_abi_id: String,
    pub entrypoint: Option<String>,
    pub mode: String,
    pub may_suspend: bool,
    pub interface_name: Option<String>,
    pub interface_module_path: Option<String>,
    pub interface_source_role: Option<CompilerSourceRole>,
    pub interface_exported: Option<bool>,
    pub implementation: OperationImplementationIr,
    pub parameters: Vec<OperationParamIr>,
    pub return_type: RuntimeTypeDescriptorIr,
    pub response: JsonSchema,
    pub summary: EffectSummary,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationImplementationIr {
    pub module_path: String,
    pub file_ir_identity: String,
    pub symbol: String,
    pub executable_index: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_const: Option<OperationConstReceiverIr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver: Option<OperationReceiverIr>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationReceiverIr {
    #[serde(rename = "type")]
    pub ty: String,
    pub binding: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationConstReceiverIr {
    pub module_path: String,
    pub const_name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationParamIr {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: RuntimeTypeDescriptorIr,
    pub schema: JsonSchema,
}

pub fn service_operation_entries(
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    operations: &[ArtifactOperation],
    entry_operations: &[EntryOperationSpec],
    interface_modules: &BTreeMap<String, InterfaceModule>,
    file_ir_units: &[FileIrUnit],
    public_instances: &[PublicInstanceExport],
) -> Result<Vec<OperationEntryIr>, ProjectionError> {
    let mut entries = operations
        .iter()
        .map(|artifact_operation| {
            let resolved = resolve_projected_operation(
                contract_projection,
                public_instances,
                &artifact_operation.operation,
            )?;
            let file_ir_unit = file_ir_units
                .iter()
                .find(|unit| unit.module_path == resolved.implementation_module())
                .ok_or_else(|| ProjectionError::ImplementationConformance {
                    message: format!(
                        "operation {}: File IR unit for implementation module {} not found",
                        artifact_operation.operation,
                        resolved.implementation_module()
                    ),
                })?;

            let interface_module = resolved
                .interface_name()
                .and_then(|interface_name| interface_modules.get(interface_name));
            let (mode, return_type, parameters, response) =
                resolved.runtime_surface(contract_projection, projection_index);
            let executable_symbol = resolved.executable_symbol();
            let executable_index = resolved
                .executable_index(file_ir_unit, &executable_symbol)
                .ok_or_else(|| ProjectionError::ImplementationConformance {
                    message: format!(
                        "operation {}: implementation method {}.{} has no executable body",
                        artifact_operation.operation,
                        resolved.implementation_module(),
                        executable_symbol
                    ),
                })?;
            let may_suspend =
                resolved.executable_may_suspend(file_ir_unit, executable_index, &executable_symbol);
            let uses_adapter = resolved.uses_non_receiver_adapter();
            Ok::<OperationEntryIr, ProjectionError>(OperationEntryIr {
                operation: artifact_operation.operation.clone(),
                operation_abi_id: resolved.operation_abi_id(&artifact_operation.operation),
                entrypoint: artifact_operation.target.clone(),
                mode: mode.to_string(),
                may_suspend,
                interface_name: resolved.interface_name().map(str::to_string),
                interface_module_path: interface_module.map(|module| module.module_name.clone()),
                interface_source_role: interface_module.map(|module| module.source_role),
                interface_exported: interface_module.map(|module| module.exported),
                implementation: OperationImplementationIr {
                    module_path: resolved.implementation_module().to_string(),
                    file_ir_identity: file_ir_unit.file_ir_identity.clone(),
                    symbol: executable_symbol.clone(),
                    executable_index: Some(executable_index),
                    receiver_const: resolved.receiver_const().clone(),
                    receiver: (!uses_adapter).then(|| OperationReceiverIr {
                        ty: resolved.receiver_type().to_string(),
                        binding: "self".to_string(),
                    }),
                    method: (!uses_adapter).then(|| resolved.method_name().to_string()),
                    function: uses_adapter.then_some(executable_symbol),
                },
                parameters,
                return_type,
                response,
                summary: effect_summary(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    for entry_operation in entry_operations {
        entries.push(entry_operation_entry(
            entry_operation,
            contract_projection,
            projection_index,
            file_ir_units,
        )?);
    }
    Ok(entries)
}

enum ResolvedProjectedOperation<'a> {
    Contract {
        interface_name: String,
        method_name: String,
        operation: &'a crate::contract::ContractInterfaceOperationProjection,
        implementation: &'a crate::contract::ContractOperationBindingProjection,
        receiver_const: Option<OperationConstReceiverIr>,
    },
    PublicInstance {
        instance: &'a PublicInstanceExport,
        operation: &'a PublicInstanceOperation,
        interface_name: Option<String>,
        contract_operation: Option<&'a crate::contract::ContractInterfaceOperationProjection>,
        executable_symbol: String,
        receiver_type: String,
        receiver_const: Option<OperationConstReceiverIr>,
    },
}

impl<'a> ResolvedProjectedOperation<'a> {
    fn uses_non_receiver_adapter(&self) -> bool {
        matches!(
            self,
            Self::Contract {
                receiver_const: None,
                ..
            }
        )
    }

    fn interface_name(&self) -> Option<&str> {
        match self {
            Self::Contract { interface_name, .. } => Some(interface_name.as_str()),
            Self::PublicInstance { interface_name, .. } => interface_name.as_deref(),
        }
    }

    fn method_name(&self) -> &str {
        match self {
            Self::Contract { method_name, .. } => method_name,
            Self::PublicInstance { operation, .. } => {
                public_instance_operation_method_name(operation)
            }
        }
    }

    fn implementation_module(&self) -> &str {
        match self {
            Self::Contract { implementation, .. } => implementation.module_path.as_str(),
            Self::PublicInstance { operation, .. } => operation
                .receiver_executable
                .executable_target
                .file_ref
                .module_path
                .as_str(),
        }
    }

    fn executable_symbol(&self) -> String {
        match self {
            Self::Contract {
                implementation,
                receiver_const,
                ..
            } => {
                if receiver_const.is_none() {
                    service_operation_adapter_symbol(
                        &implementation.type_name,
                        &implementation.method_name,
                    )
                    .to_string()
                } else {
                    implementation.executable_symbol.clone()
                }
            }
            Self::PublicInstance {
                executable_symbol, ..
            } => executable_symbol.clone(),
        }
    }

    fn receiver_type(&self) -> &str {
        match self {
            Self::Contract { implementation, .. } => implementation.type_name.as_str(),
            Self::PublicInstance { receiver_type, .. } => receiver_type.as_str(),
        }
    }

    fn receiver_const(&self) -> &Option<OperationConstReceiverIr> {
        match self {
            Self::Contract { receiver_const, .. } => receiver_const,
            Self::PublicInstance { receiver_const, .. } => receiver_const,
        }
    }

    fn executable_index(&self, file_ir_unit: &FileIrUnit, executable_symbol: &str) -> Option<u64> {
        match self {
            Self::Contract { .. } => executable_index(file_ir_unit, executable_symbol),
            Self::PublicInstance { operation, .. } => {
                let target = &operation.receiver_executable.executable_target;
                (file_ir_unit.module_path == target.file_ref.module_path)
                    .then_some(u64::from(target.executable_index))
            }
        }
    }

    fn executable_may_suspend(
        &self,
        file_ir_unit: &FileIrUnit,
        executable_index: u64,
        executable_symbol: &str,
    ) -> bool {
        match self {
            Self::Contract { .. } => executable_may_suspend(file_ir_unit, executable_symbol),
            Self::PublicInstance { .. } => usize::try_from(executable_index)
                .ok()
                .and_then(|index| file_ir_unit.executables.get(index))
                .map(|executable| executable.may_suspend)
                .unwrap_or(true),
        }
    }

    fn runtime_surface(
        &self,
        contract_projection: &ContractProjection,
        projection_index: &ContractProjectionIndex<'_>,
    ) -> (
        &'static str,
        RuntimeTypeDescriptorIr,
        Vec<OperationParamIr>,
        JsonSchema,
    ) {
        match self {
            Self::Contract { operation, .. } => {
                let (mode, return_type) =
                    projection_operation_mode_and_return_type(&operation.return_type);
                (
                    mode,
                    contract_projection.runtime_descriptor_for_type_key(&return_type),
                    operation_params(&operation.params, contract_projection),
                    contract_projection.schema_for_type_key(&return_type),
                )
            }
            Self::PublicInstance {
                contract_operation: Some(operation),
                ..
            } => {
                let (mode, return_type) =
                    projection_operation_mode_and_return_type(&operation.return_type);
                (
                    mode,
                    contract_projection.runtime_descriptor_for_type_key(&return_type),
                    operation_params(&operation.params, contract_projection),
                    contract_projection.schema_for_type_key(&return_type),
                )
            }
            Self::PublicInstance { operation, .. } => {
                if let Some((module_path, params, return_type)) =
                    public_instance_source_interface_signature(operation, projection_index)
                {
                    let (mode, response_type) = operation_mode_and_return_type(&return_type);
                    return (
                        mode,
                        contract_projection.runtime_descriptor_for_source_type_ref(
                            projection_index,
                            &module_path,
                            &response_type,
                        ),
                        source_function_params(
                            &params,
                            contract_projection,
                            projection_index,
                            &module_path,
                        ),
                        contract_projection.schema_for_source_type_ref(
                            projection_index,
                            &module_path,
                            &response_type,
                        ),
                    );
                }
                // The interface is not a (source-or-public) service interface —
                // e.g. a public instance implementing a *package* interface such
                // as `llmApi.LlmClient`. The contract / projection index does not
                // carry package interface decls, but the bound receiver impl
                // method does live in a service module, so we project the public
                // surface directly from that executable (the same source the
                // serviceUnit route uses). This keeps mode / params / response in
                // lockstep with the operation ABI id (which was computed from the
                // same self-stripped signature) instead of degrading to a
                // unary/empty fallback that the router rejects.
                if let Some((module_path, params, return_type)) =
                    public_instance_receiver_executable_signature(operation, projection_index)
                {
                    let (mode, response_type) = operation_mode_and_return_type(&return_type);
                    return (
                        mode,
                        contract_projection.runtime_descriptor_for_source_type_ref(
                            projection_index,
                            &module_path,
                            &response_type,
                        ),
                        source_function_params(
                            &params,
                            contract_projection,
                            projection_index,
                            &module_path,
                        ),
                        contract_projection.schema_for_source_type_ref(
                            projection_index,
                            &module_path,
                            &response_type,
                        ),
                    );
                }
                let module_path = operation
                    .receiver_executable
                    .executable_target
                    .file_ref
                    .module_path
                    .as_str();
                (
                    "unary",
                    contract_projection.runtime_descriptor_for_source_type_ref(
                        projection_index,
                        module_path,
                        &TypeRefIr::native("unit"),
                    ),
                    Vec::new(),
                    contract_projection.schema_for_source_type_ref(
                        projection_index,
                        module_path,
                        &TypeRefIr::native("unit"),
                    ),
                )
            }
        }
    }

    fn operation_abi_id(&self, operation_name: &str) -> String {
        match self {
            Self::Contract { operation, .. } => {
                contract_public_function_operation_abi_id(operation_name, operation)
            }
            Self::PublicInstance {
                instance,
                operation,
                ..
            } => runtime_operation_abi_id(instance, operation),
        }
    }
}

fn resolve_projected_operation<'a>(
    contract: &'a ContractProjection,
    public_instances: &'a [PublicInstanceExport],
    operation_name: &str,
) -> Result<ResolvedProjectedOperation<'a>, ProjectionError> {
    if let Some((interface_name, method_name)) = contract.split_operation_name(operation_name) {
        if let Some(interface) = contract.interfaces.get(interface_name) {
            if let Some(operation) = interface
                .operations
                .iter()
                .find(|operation| operation.name == method_name)
            {
                let _binding = contract.api_bindings.get(interface_name).ok_or_else(|| {
                    ProjectionError::ImplementationConformance {
                        message: format!(
                            "operation {operation_name}: API binding for {interface_name} is missing"
                        ),
                    }
                })?;
                let implementation = contract
                    .operation_binding(interface_name, method_name)
                    .ok_or_else(|| ProjectionError::ImplementationConformance {
                        message: format!(
                            "operation {operation_name}: implementation binding for {interface_name}.{method_name} is missing"
                        ),
                    })?;
                return Ok(ResolvedProjectedOperation::Contract {
                    interface_name: interface_name.to_string(),
                    method_name: method_name.to_string(),
                    operation,
                    implementation,
                    receiver_const: None,
                });
            }
        }
    }

    let (instance, public_operation) = public_instances
        .iter()
        .find_map(|instance| {
            instance
                .operations
                .iter()
                .find(|operation| operation.operation.public_path == operation_name)
                .map(|operation| (instance, operation))
        })
        .ok_or_else(|| ProjectionError::ImplementationConformance {
            message: format!("operation {operation_name}: expected Interface.method or public instance operation name"),
        })?;
    resolve_public_instance_operation(contract, instance, public_operation)
}

fn resolve_public_instance_operation<'a>(
    contract: &'a ContractProjection,
    instance: &'a PublicInstanceExport,
    public_operation: &'a PublicInstanceOperation,
) -> Result<ResolvedProjectedOperation<'a>, ProjectionError> {
    let (interface_name, contract_operation) =
        if let Some(interface_name) = public_instance_interface_name(public_operation) {
            let interface = contract.interfaces.get(&interface_name).ok_or_else(|| {
                ProjectionError::ImplementationConformance {
                    message: format!(
                        "operation {}: exported contract interface {interface_name} not found",
                        public_operation.operation.public_path
                    ),
                }
            })?;
            let method_name = public_instance_operation_method_name(public_operation);
            let operation = interface
                .operations
                .iter()
                .find(|operation| operation.name == method_name)
                .ok_or_else(|| ProjectionError::ImplementationConformance {
                    message: format!(
                        "operation {}: contract method {}.{} not found",
                        public_operation.operation.public_path, interface_name, method_name
                    ),
                })?;
            (Some(interface_name), Some(operation))
        } else {
            (None, None)
        };

    Ok(ResolvedProjectedOperation::PublicInstance {
        instance,
        operation: public_operation,
        interface_name,
        contract_operation,
        executable_symbol: public_instance_operation_executable_symbol(public_operation),
        receiver_type: public_instance_receiver_type_name(instance),
        receiver_const: Some(public_instance_receiver_const(public_operation)),
    })
}

fn public_instance_interface_name(operation: &PublicInstanceOperation) -> Option<String> {
    let interface = operation.operation.interface.as_ref()?;
    let ty: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
    match &ty {
        TypeRefIr::ServiceSymbol { symbol } if symbol.module_path.is_empty() => {
            Some(symbol.symbol.clone())
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => None,
    }
}

fn public_instance_operation_method_name(operation: &PublicInstanceOperation) -> &str {
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

fn public_instance_receiver_const(operation: &PublicInstanceOperation) -> OperationConstReceiverIr {
    let receiver = &operation.receiver_executable.receiver;
    OperationConstReceiverIr {
        module_path: receiver.file_ref.module_path.clone(),
        const_name: local_symbol_from_abi_id(
            "const:",
            &receiver.file_ref.module_path,
            &receiver.const_abi_id,
        )
        .unwrap_or_else(|| receiver.const_abi_id.clone()),
    }
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

fn public_instance_receiver_type_name(instance: &PublicInstanceExport) -> String {
    type_ref_display_name(&instance.declared_receiver_type)
}

fn type_ref_display_name(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::ServiceSymbol { symbol } => symbol.symbol.clone(),
        TypeRefIr::LocalType { type_index } => format!("#{type_index}"),
        TypeRefIr::PublicationType { module_path, .. } => format!("root.{module_path}"),
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::DbObjectSymbol { symbol } => symbol.symbol_path(),
        TypeRefIr::Native { name, .. } => name.clone(),
        TypeRefIr::Record { .. } => "{}".to_string(),
        TypeRefIr::Union { .. } => "union".to_string(),
        TypeRefIr::Nullable { inner } => type_ref_display_name(inner),
        TypeRefIr::Literal { .. } => "literal".to_string(),
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::AnyInterface { interface } => format!("any {}", interface.interface_abi_id),
        TypeRefIr::Function { .. } => "fn".to_string(),
    }
}

fn source_operation_params_from_service_params(
    params: &[OperationParam],
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    module_path: &str,
) -> Vec<OperationParamIr> {
    params
        .iter()
        .map(|param| OperationParamIr {
            name: param.name.clone(),
            ty: contract_projection.runtime_descriptor_for_source_type_ref(
                projection_index,
                module_path,
                &param.ty,
            ),
            schema: contract_projection.schema_for_source_type_ref(
                projection_index,
                module_path,
                &param.ty,
            ),
        })
        .collect()
}

fn source_function_params(
    params: &[FunctionTypeParamIr],
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    module_path: &str,
) -> Vec<OperationParamIr> {
    params
        .iter()
        .map(|param| OperationParamIr {
            name: param.name.clone(),
            ty: contract_projection.runtime_descriptor_for_source_type_ref(
                projection_index,
                module_path,
                &param.ty,
            ),
            schema: contract_projection.schema_for_source_type_ref(
                projection_index,
                module_path,
                &param.ty,
            ),
        })
        .collect()
}

fn entry_operation_entry(
    operation: &EntryOperationSpec,
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    file_ir_units: &[FileIrUnit],
) -> Result<OperationEntryIr, ProjectionError> {
    let file_ir_unit = file_ir_units
        .iter()
        .find(|unit| unit.module_path == operation.implementation_module)
        .ok_or_else(|| ProjectionError::ImplementationConformance {
            message: format!(
                "operation {}: File IR unit for entry implementation module {} not found",
                operation.operation, operation.implementation_module
            ),
        })?;

    let (mode, _) = operation_mode_and_return_type(&operation.return_type.ir);
    let return_type = response_type_ir(&operation.return_type);
    let executable_symbol = match &operation.callable {
        EntryOperationCallable::ImplMethod { type_name, method } => {
            service_operation_adapter_symbol(type_name, method).to_string()
        }
        EntryOperationCallable::Function { name } => name.clone(),
    };
    let executable_index = executable_index(file_ir_unit, &executable_symbol).ok_or_else(|| {
        ProjectionError::ImplementationConformance {
            message: format!(
                "operation {}: entry implementation {}.{} is missing from File IR",
                operation.operation, operation.implementation_module, executable_symbol
            ),
        }
    })?;
    let executable = usize::try_from(executable_index)
        .ok()
        .and_then(|index| file_ir_unit.executables.get(index))
        .ok_or_else(|| ProjectionError::ImplementationConformance {
            message: format!(
                "operation {}: File IR executable index {} for {}.{} is out of bounds",
                operation.operation,
                executable_index,
                operation.implementation_module,
                executable_symbol
            ),
        })?;
    let expected_kind = ExecutableKind::Function;
    if executable.kind != expected_kind {
        return Err(ProjectionError::ImplementationConformance {
            message: format!(
                "operation {}: entry implementation {}.{} has File IR kind {:?}, expected {:?}",
                operation.operation,
                operation.implementation_module,
                executable_symbol,
                executable.kind,
                expected_kind
            ),
        });
    }
    if executable.body.blocks.is_empty() {
        return Err(ProjectionError::ImplementationConformance {
            message: format!(
                "operation {}: entry implementation {}.{} has no executable body",
                operation.operation, operation.implementation_module, executable_symbol
            ),
        });
    }
    let may_suspend = executable_may_suspend(file_ir_unit, &executable_symbol);
    let implementation = match &operation.callable {
        EntryOperationCallable::ImplMethod { .. } => OperationImplementationIr {
            module_path: operation.implementation_module.clone(),
            file_ir_identity: file_ir_unit.file_ir_identity.clone(),
            symbol: executable_symbol.clone(),
            executable_index: Some(executable_index),
            receiver_const: None,
            receiver: None,
            method: None,
            function: Some(executable_symbol.clone()),
        },
        EntryOperationCallable::Function { name } => OperationImplementationIr {
            module_path: operation.implementation_module.clone(),
            file_ir_identity: file_ir_unit.file_ir_identity.clone(),
            symbol: executable_symbol.clone(),
            executable_index: Some(executable_index),
            receiver_const: None,
            receiver: None,
            method: None,
            function: Some(name.clone()),
        },
    };
    Ok(OperationEntryIr {
        operation: operation.operation.clone(),
        operation_abi_id: entry_operation_abi_id(
            &operation.operation,
            &operation.params,
            &operation.return_type,
        ),
        entrypoint: Some(operation.target.clone()),
        mode: mode.to_string(),
        may_suspend,
        interface_name: None,
        interface_module_path: None,
        interface_source_role: None,
        interface_exported: Some(false),
        implementation,
        parameters: source_operation_params(
            &operation.params,
            contract_projection,
            projection_index,
            &operation.implementation_module,
        ),
        return_type: contract_projection.runtime_descriptor_for_source_type_ref(
            projection_index,
            &operation.implementation_module,
            &return_type,
        ),
        response: contract_projection.schema_for_source_type_ref(
            projection_index,
            &operation.implementation_module,
            &return_type,
        ),
        summary: effect_summary_for_signature(&operation.return_type.ir),
    })
}

fn source_operation_params(
    params: &[EntryParamSpec],
    contract_projection: &ContractProjection,
    projection_index: &ContractProjectionIndex<'_>,
    current_module: &str,
) -> Vec<OperationParamIr> {
    params
        .iter()
        .map(|param| OperationParamIr {
            name: param.name.clone(),
            ty: contract_projection.runtime_descriptor_for_source_type_ref(
                projection_index,
                current_module,
                &param.ty.ir,
            ),
            schema: contract_projection.schema_for_source_type_ref(
                projection_index,
                current_module,
                &param.ty.ir,
            ),
        })
        .collect()
}

fn operation_params(
    params: &[ContractFunctionParamProjection],
    contract: &ContractProjection,
) -> Vec<OperationParamIr> {
    params
        .iter()
        .map(|param| OperationParamIr {
            name: param.name.clone(),
            ty: contract.runtime_descriptor_for_type_key(&param.ty),
            schema: contract.schema_for_type_key(&param.ty),
        })
        .collect()
}

pub fn service_operation_adapter_symbol(type_name: &str, method_name: &str) -> String {
    format!(
        "__skiff_service_operation_adapter_{}_{}",
        sanitize_adapter_component(type_name),
        sanitize_adapter_component(method_name)
    )
}

fn sanitize_adapter_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn projection_operation_mode_and_return_type(
    return_type: &ContractTypeKey,
) -> (&'static str, ContractTypeKey) {
    match return_type {
        ContractTypeKey::Builtin { name, args }
            if bare_type_name(name) == "Stream" && args.len() == 1 =>
        {
            ("serverStream", args[0].clone())
        }
        _ => ("unary", return_type.clone()),
    }
}

pub fn operation_mode_and_return_type(return_type: &TypeRefIr) -> (&'static str, TypeRefIr) {
    match return_type {
        TypeRefIr::Native { name, args } if bare_type_name(name) == "Stream" && args.len() == 1 => {
            ("serverStream", args[0].clone())
        }
        _ => ("unary", return_type.clone()),
    }
}

fn bare_type_name(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

#[derive(Debug)]
pub struct InterfaceModule {
    pub module_name: String,
    pub source_role: CompilerSourceRole,
    pub exported: bool,
}

pub fn interface_modules(
    source_metadata: &[ProjectionSourceMetadata],
    contract: &ContractProjection,
) -> BTreeMap<String, InterfaceModule> {
    let mut modules = BTreeMap::new();
    let sources_by_module = source_metadata
        .iter()
        .map(|source| (source.module_path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    for (interface_name, interface) in &contract.interfaces {
        let source_symbol = contract
            .interface_source_symbols
            .get(interface_name)
            .cloned()
            .unwrap_or_else(|| format!("{}.{}", interface.source_module, interface.source_name));
        let Some((module_path, _local_name)) = source_symbol.rsplit_once('.') else {
            continue;
        };
        let Some(source) = sources_by_module.get(module_path).copied() else {
            continue;
        };
        modules.insert(
            interface_name.clone(),
            InterfaceModule {
                module_name: source.module_path.clone(),
                source_role: source.role,
                exported: true,
            },
        );
    }
    modules
}

fn executable_may_suspend(file_ir_unit: &FileIrUnit, symbol: &str) -> bool {
    executable_index(file_ir_unit, symbol)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| file_ir_unit.executables.get(index))
        .map(|executable| executable.may_suspend)
        .unwrap_or(true)
}

fn executable_index(file_ir_unit: &FileIrUnit, symbol: &str) -> Option<u64> {
    file_ir_unit
        .declarations
        .executables
        .get(symbol)
        .map(|declaration| u64::from(declaration.executable_index))
}

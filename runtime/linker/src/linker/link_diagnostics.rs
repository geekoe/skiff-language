use std::collections::BTreeMap;

use skiff_artifact_model::{
    type_ref_abi_key, CanonicalPublicCallableSignature,
    FunctionTypeParamIr as ArtifactFunctionTypeParamIr, OperationAbiRef, PackageOperationTarget,
    ReceiverCallAbi, ServiceOperation, TypeRefIr,
};

use crate::{
    program::{
        addr::{ExecutableAddr, FileAddr, TypeAddr, UnitAddr},
        linked::{
            ConstIr, ExecutableKind, FunctionTypeParamIr, LinkedExecutable, LinkedFileUnit,
            LinkedFunctionTypeParamIr, LinkedTypeRef, PackageRefIr, PackageSymbolRef,
            ServiceSymbolRef,
        },
    },
    resolver::{ProgramError, ProgramResult},
};

#[derive(Debug, Clone, Copy)]
pub(super) enum PublicSignatureProjection {
    Full,
    StripExplicitSelf,
}

pub(super) fn executable_context(addr: &ExecutableAddr, symbol: &str) -> String {
    format!("{} ({symbol})", addr)
}

pub(super) fn service_operation_context(operation: &OperationAbiRef) -> String {
    format!(
        "service operation {} operationAbiId {}",
        operation.public_path, operation.operation_abi_id
    )
}

pub(super) fn package_operation_context(package_id: &str, operation: &OperationAbiRef) -> String {
    format!(
        "package {package_id} operation {} operationAbiId {}",
        operation.public_path, operation.operation_abi_id
    )
}

pub(super) fn service_operation_ref(operation: &ServiceOperation) -> &OperationAbiRef {
    match operation {
        ServiceOperation::LocalExecutable(target) => &target.operation,
        ServiceOperation::LocalReceiverExecutable(target) => &target.operation,
    }
}

pub(super) fn package_operation_target_operation(
    target: &PackageOperationTarget,
) -> &OperationAbiRef {
    match target {
        PackageOperationTarget::LocalExecutable { operation, .. } => operation,
        PackageOperationTarget::LocalConstReceiverExecutable { operation, .. } => operation,
    }
}

pub(super) fn type_context(addr: &TypeAddr) -> String {
    addr.to_string()
}

pub(super) fn const_context(unit: &UnitAddr, file: &FileAddr, name: &str) -> String {
    format!("{unit}:{file} const {name}")
}

pub(super) fn interface_context(unit: &UnitAddr, file: &FileAddr, name: &str) -> String {
    format!("{unit}:{file} interface {name}")
}

pub(super) fn db_context(unit: &UnitAddr, file: &FileAddr, name: &str) -> String {
    format!("{unit}:{file} db {name}")
}

pub(super) fn interface_declaration_abi_id(
    context: &str,
    file: &LinkedFileUnit,
    declaration_name: &str,
) -> ProgramResult<String> {
    let symbol_name = file
        .declarations
        .types
        .get(declaration_name)
        .and_then(|declaration| {
            declaration
                .symbol
                .strip_prefix(&format!("{}.", file.module_path))
                .map(str::to_string)
        })
        .unwrap_or_else(|| declaration_name.to_string());
    linked_type_ref_abi_key(
        context,
        &LinkedTypeRef::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: file.module_path.clone(),
                symbol: symbol_name,
            },
        },
    )
}

pub(super) fn package_interface_declaration_abi_id(
    context: &str,
    package: PackageRefIr,
    symbol_path: &str,
) -> ProgramResult<String> {
    linked_type_ref_abi_key(
        context,
        &LinkedTypeRef::PackageSymbol {
            symbol: PackageSymbolRef {
                package,
                symbol_path: symbol_path.to_string(),
                abi_expectation: None,
            },
        },
    )
}

pub(super) fn package_symbol_label(symbol: &PackageSymbolRef) -> String {
    format!(
        "{}::{}",
        package_ref_label(&symbol.package),
        symbol.symbol_path
    )
}

pub(super) fn package_ref_identity(package: &PackageRefIr) -> &str {
    match package {
        PackageRefIr::PackageId { package_id } => package_id,
        PackageRefIr::Dependency { dependency_ref } => dependency_ref,
    }
}

pub(super) fn package_ref_label(package: &PackageRefIr) -> String {
    match package {
        PackageRefIr::PackageId { package_id } => format!("packageId {package_id}"),
        PackageRefIr::Dependency { dependency_ref } => format!("dependencyRef {dependency_ref}"),
    }
}

pub(super) fn executable_public_signature(
    context: &str,
    executable: &LinkedExecutable,
    projection: PublicSignatureProjection,
) -> ProgramResult<CanonicalPublicCallableSignature> {
    let skip_params = match projection {
        PublicSignatureProjection::Full => 0,
        PublicSignatureProjection::StripExplicitSelf => {
            if executable.self_type.is_some() {
                0
            } else if executable.params.is_empty() {
                return Err(ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: executable.symbol.clone(),
                    expected_kind: "explicit-self executable public signature",
                });
            } else {
                1
            }
        }
    };
    let params = executable
        .params
        .iter()
        .skip(skip_params)
        .map(|param| {
            Ok(ArtifactFunctionTypeParamIr {
                name: param.name.clone(),
                ty: linked_type_ref_to_artifact(context, &param.ty)?,
            })
        })
        .collect::<ProgramResult<Vec<_>>>()?;
    let Some(return_type) = executable.return_type.as_ref() else {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: executable.symbol.clone(),
            expected_kind: "executable returnType for public ABI signature",
        });
    };
    Ok(CanonicalPublicCallableSignature {
        params,
        return_type: linked_type_ref_to_artifact(context, return_type)?,
        may_suspend: executable.may_suspend,
    })
}

pub(super) fn remote_slot_public_signature(
    context: &str,
    slot: &crate::program::LinkedRemoteOperationSlotPlanIr,
) -> ProgramResult<CanonicalPublicCallableSignature> {
    let params = slot
        .signature
        .params
        .iter()
        .map(|param| {
            Ok(ArtifactFunctionTypeParamIr {
                name: param.name.clone(),
                ty: linked_type_ref_to_artifact(context, &param.ty)?,
            })
        })
        .collect::<ProgramResult<Vec<_>>>()?;
    Ok(CanonicalPublicCallableSignature {
        params,
        return_type: linked_type_ref_to_artifact(context, &slot.signature.return_type)?,
        may_suspend: false,
    })
}

pub(super) fn receiver_executable_self_type(
    executable: &LinkedExecutable,
) -> Option<&LinkedTypeRef> {
    executable
        .self_type
        .as_ref()
        .or_else(|| executable.params.first().map(|param| &param.ty))
}

pub(super) fn linked_type_ref_to_artifact(
    context: &str,
    type_ref: &LinkedTypeRef,
) -> ProgramResult<TypeRefIr> {
    let value =
        serde_json::to_value(type_ref).map_err(|error| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: error.to_string(),
            expected_kind: "serializable linked type ref for public ABI signature",
        })?;
    serde_json::from_value::<TypeRefIr>(value).map_err(|error| ProgramError::LinkSymbolUnresolved {
        context: context.to_string(),
        symbol: error.to_string(),
        expected_kind: "artifact TypeRefIr for public ABI signature",
    })
}

pub(super) fn linked_interface_instantiation_to_artifact(
    context: &str,
    interface: &crate::program::LinkedInterfaceInstantiationRef,
) -> ProgramResult<skiff_artifact_model::InterfaceInstantiationRef> {
    let value =
        serde_json::to_value(interface).map_err(|error| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: error.to_string(),
            expected_kind: "serializable remote interface instantiation",
        })?;
    serde_json::from_value::<skiff_artifact_model::InterfaceInstantiationRef>(value).map_err(
        |error| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: error.to_string(),
            expected_kind: "artifact remote interface instantiation",
        },
    )
}

pub(super) fn public_signature_diagnostic(signature: &CanonicalPublicCallableSignature) -> String {
    serde_json::to_string(signature).unwrap_or_else(|_| format!("{signature:?}"))
}

pub(super) fn executable_callable_abi_ids(
    file: &LinkedFileUnit,
    executable_index: usize,
    executable: &LinkedExecutable,
) -> Vec<String> {
    let mut candidates = Vec::new();
    push_unique_candidate(&mut candidates, format!("callable:{}", executable.symbol));
    for (symbol, declaration) in &file.declarations.executables {
        if declaration.executable_index != executable_index {
            continue;
        }
        push_unique_candidate(&mut candidates, format!("callable:{}", declaration.symbol));
        push_unique_candidate(
            &mut candidates,
            format!(
                "callable:{}",
                qualified_item_symbol(&file.module_path, symbol)
            ),
        );
    }
    for (symbol, index) in &file.link_targets.executables {
        if *index != executable_index {
            continue;
        }
        push_unique_candidate(
            &mut candidates,
            format!(
                "callable:{}",
                qualified_item_symbol(&file.module_path, symbol)
            ),
        );
    }
    candidates
}

pub(super) fn const_callable_abi_id(file: &LinkedFileUnit, constant: &ConstIr) -> String {
    format!(
        "const:{}",
        qualified_item_symbol(&file.module_path, &constant.name)
    )
}

pub(super) fn declaration_name_for_type_index(
    file: &LinkedFileUnit,
    type_index: usize,
) -> Option<String> {
    file.declarations
        .types
        .iter()
        .find_map(|(name, declaration)| {
            (declaration.type_index == type_index).then(|| {
                declaration
                    .symbol
                    .strip_prefix(&format!("{}.", file.module_path))
                    .map(str::to_string)
                    .unwrap_or_else(|| name.clone())
            })
        })
        .or_else(|| file.types.get(type_index).map(|ty| ty.name.clone()))
}

pub(super) fn qualified_item_symbol(module_path: &str, symbol: &str) -> String {
    let prefix = format!("{module_path}.");
    if symbol.starts_with(&prefix) {
        symbol.to_string()
    } else {
        format!("{module_path}.{symbol}")
    }
}

pub(super) fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

pub(super) fn executable_kind_name(kind: &ExecutableKind) -> &'static str {
    match kind {
        ExecutableKind::Function => "function",
        ExecutableKind::ImplMethod => "impl method",
        ExecutableKind::Operation => "operation",
    }
}

pub(super) fn expected_executable_params_for_receiver_abi<'a>(
    context: &str,
    executable: &'a LinkedExecutable,
    slot: &'a crate::program::LinkedInterfaceMethodSlotPlanIr,
) -> ProgramResult<&'a [LinkedFunctionTypeParamIr]> {
    match slot.target.receiver_call_abi {
        ReceiverCallAbi::ExplicitSelfFirst => {
            if executable.self_type.is_none() {
                return Ok(&slot.signature.params);
            }
            slot.signature
                .params
                .get(1..)
                .ok_or_else(|| ProgramError::LinkSymbolUnresolved {
                    context: context.to_string(),
                    symbol: format!("{} slot {}", slot.method_name, slot.slot),
                    expected_kind: "interface method slot explicit self parameter",
                })
        }
    }
}

pub(super) fn receiver_executable_matches_concrete_type(
    executable: &LinkedExecutable,
    concrete_type: &LinkedTypeRef,
) -> bool {
    if executable.self_type.as_ref() == Some(concrete_type) {
        return true;
    }
    executable.self_type.is_none()
        && executable
            .params
            .first()
            .is_some_and(|param| param.name == "self" && &param.ty == concrete_type)
}

pub(super) fn validate_interface_operation_explicit_self(
    context: &str,
    interface: &crate::program::LinkedInterfaceInstantiationRef,
    operation: &crate::program::linked::InterfaceOperationIr,
) -> ProgramResult<()> {
    let Some(first) = operation.params.first() else {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{}.{}",
                interface_instantiation_symbol(interface),
                operation.name
            ),
            expected_kind: "interface method explicit self receiver",
        });
    };
    if first.name != "self" || !is_linked_self_type(&first.ty) {
        return Err(ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: format!(
                "{}.{} first parameter {}",
                interface_instantiation_symbol(interface),
                operation.name,
                first.name
            ),
            expected_kind: "interface method explicit self receiver",
        });
    }
    Ok(())
}

pub(super) fn substitute_interface_method_type(
    type_ref: &LinkedTypeRef,
    substitutions: &BTreeMap<String, LinkedTypeRef>,
    self_type: Option<&LinkedTypeRef>,
) -> ProgramResult<LinkedTypeRef> {
    if is_linked_self_type(type_ref) {
        return Ok(self_type.cloned().unwrap_or_else(|| type_ref.clone()));
    }
    if let LinkedTypeRef::TypeParam { name } = type_ref {
        if let Some(replacement) = substitutions.get(name) {
            return Ok(replacement.clone());
        }
    }
    Ok(match type_ref {
        LinkedTypeRef::Native { name, args } => LinkedTypeRef::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_interface_method_type(arg, substitutions, self_type))
                .collect::<ProgramResult<Vec<_>>>()?,
        },
        LinkedTypeRef::Record { fields } => LinkedTypeRef::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        substitute_interface_method_type(ty, substitutions, self_type)?,
                    ))
                })
                .collect::<ProgramResult<BTreeMap<_, _>>>()?,
        },
        LinkedTypeRef::Union { items } => LinkedTypeRef::Union {
            items: items
                .iter()
                .map(|item| substitute_interface_method_type(item, substitutions, self_type))
                .collect::<ProgramResult<Vec<_>>>()?,
        },
        LinkedTypeRef::Nullable { inner } => LinkedTypeRef::Nullable {
            inner: Box::new(substitute_interface_method_type(
                inner,
                substitutions,
                self_type,
            )?),
        },
        LinkedTypeRef::AnyInterface { interface } => LinkedTypeRef::AnyInterface {
            interface: crate::program::LinkedInterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| substitute_interface_method_type(arg, substitutions, self_type))
                    .collect::<ProgramResult<Vec<_>>>()?,
            },
        },
        LinkedTypeRef::Function {
            params,
            return_type,
        } => LinkedTypeRef::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: substitute_interface_method_type(&param.ty, substitutions, self_type)?,
                    })
                })
                .collect::<ProgramResult<Vec<_>>>()?,
            return_type: Box::new(substitute_interface_method_type(
                return_type,
                substitutions,
                self_type,
            )?),
        },
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::TypeParam { .. } => type_ref.clone(),
    })
}

pub(super) fn is_linked_self_type(type_ref: &LinkedTypeRef) -> bool {
    match type_ref {
        LinkedTypeRef::TypeParam { name } => name == "Self",
        LinkedTypeRef::Native { name, args } => name == "Self" && args.is_empty(),
        LinkedTypeRef::ServiceSymbol { symbol } | LinkedTypeRef::DbObjectSymbol { symbol } => {
            symbol.symbol == "Self"
        }
        _ => false,
    }
}

pub(super) fn canonical_linked_interface_method_abi_id(
    interface: &crate::program::LinkedInterfaceInstantiationRef,
    method_name: &str,
) -> String {
    if interface.canonical_type_args.is_empty() {
        format!("method:{}:{method_name}", interface.interface_abi_id)
    } else {
        let type_args = serde_json::to_string(&interface.canonical_type_args)
            .expect("canonical linked interface type args must serialize for method ABI key");
        format!(
            "method:{}:{type_args}:{method_name}",
            interface.interface_abi_id
        )
    }
}

pub(super) fn linked_type_ref_abi_key(
    context: &str,
    type_ref: &LinkedTypeRef,
) -> ProgramResult<String> {
    let value =
        serde_json::to_value(type_ref).map_err(|error| ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: error.to_string(),
            expected_kind: "serializable linked type ref for ABI id",
        })?;
    let artifact_type = serde_json::from_value::<TypeRefIr>(value).map_err(|error| {
        ProgramError::LinkSymbolUnresolved {
            context: context.to_string(),
            symbol: error.to_string(),
            expected_kind: "artifact TypeRefIr for ABI id",
        }
    })?;
    Ok(type_ref_abi_key(&artifact_type))
}

pub(super) fn type_ref_diagnostic(type_ref: &LinkedTypeRef) -> String {
    serde_json::to_string(type_ref).unwrap_or_else(|_| format!("{type_ref:?}"))
}

pub(super) fn executable_params_match_slot_signature(
    actual_params: &[crate::program::ParamIr],
    expected_params: &[crate::program::LinkedFunctionTypeParamIr],
) -> bool {
    actual_params.len() == expected_params.len()
        && actual_params
            .iter()
            .zip(expected_params.iter())
            .all(|(actual, expected)| actual.name == expected.name && actual.ty == expected.ty)
}

pub(super) fn interface_method_table_symbol(
    plan: &crate::program::LinkedInterfaceMethodTablePlanIr,
) -> String {
    format!(
        "interface method table {} for {}",
        plan.interface.interface_abi_id,
        type_ref_diagnostic(&plan.concrete_type)
    )
}

pub(super) fn remote_operation_table_symbol(
    dependency_ref: &str,
    public_instance_key: &str,
    plan: &crate::program::LinkedRemoteOperationTablePlanIr,
) -> String {
    format!(
        "remote operation table {dependency_ref}/{public_instance_key} for {}",
        interface_instantiation_symbol(&plan.interface)
    )
}

pub(super) fn interface_method_call_symbol(
    interface: &crate::program::LinkedInterfaceInstantiationRef,
    method_abi_id: &str,
    slot: u32,
) -> String {
    format!(
        "{} slot {} methodAbiId {}",
        interface_instantiation_symbol(interface),
        slot,
        method_abi_id
    )
}

pub(super) fn interface_instantiation_symbol(
    interface: &crate::program::LinkedInterfaceInstantiationRef,
) -> String {
    if interface.canonical_type_args.is_empty() {
        return interface.interface_abi_id.clone();
    }
    let args = interface
        .canonical_type_args
        .iter()
        .map(type_ref_diagnostic)
        .collect::<Vec<_>>()
        .join(", ");
    format!("{}<{}>", interface.interface_abi_id, args)
}

pub(super) fn unresolved_type_param_name<'a>(
    type_ref: &'a LinkedTypeRef,
    allowed_unresolved: Option<&[String]>,
) -> Option<&'a str> {
    match type_ref {
        LinkedTypeRef::TypeParam { name }
            if allowed_unresolved
                .is_some_and(|allowed| allowed.iter().any(|item| item == name)) =>
        {
            None
        }
        LinkedTypeRef::TypeParam { name } => Some(name.as_str()),
        LinkedTypeRef::Native { args, .. } => args
            .iter()
            .find_map(|arg| unresolved_type_param_name(arg, allowed_unresolved)),
        LinkedTypeRef::Record { fields } => fields
            .values()
            .find_map(|field| unresolved_type_param_name(field, allowed_unresolved)),
        LinkedTypeRef::Union { items } => items
            .iter()
            .find_map(|item| unresolved_type_param_name(item, allowed_unresolved)),
        LinkedTypeRef::Nullable { inner } => unresolved_type_param_name(inner, allowed_unresolved),
        LinkedTypeRef::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .find_map(|arg| unresolved_type_param_name(arg, allowed_unresolved)),
        LinkedTypeRef::Function {
            params,
            return_type,
        } => params
            .iter()
            .find_map(|param| unresolved_type_param_name(&param.ty, allowed_unresolved))
            .or_else(|| unresolved_type_param_name(return_type, allowed_unresolved)),
        LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => None,
    }
}

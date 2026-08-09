use skiff_artifact_identity::canonical_interface_method_abi_id_from_parts;

use crate::{
    program::LinkedTypeRef,
    resolver::{ProgramError, ProgramResult},
};

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

pub(crate) fn canonical_linked_interface_method_abi_id(
    interface: &crate::program::LinkedInterfaceInstantiationRef,
    method_name: &str,
) -> String {
    canonical_interface_method_abi_id_from_parts(
        &interface.interface_abi_id,
        &interface.canonical_type_args,
        method_name,
    )
}

pub(super) fn type_ref_diagnostic(type_ref: &LinkedTypeRef) -> String {
    serde_json::to_string(type_ref).unwrap_or_else(|_| format!("{type_ref:?}"))
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
        LinkedTypeRef::AppliedNominal { arguments, .. } => arguments
            .iter()
            .find_map(|argument| unresolved_type_param_name(argument, allowed_unresolved)),
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
        | LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::Address { .. }
        | LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::DbObjectSymbol { .. } => None,
    }
}

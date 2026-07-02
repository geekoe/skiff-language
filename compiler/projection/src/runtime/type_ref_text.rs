use skiff_artifact_model::{InterfaceInstantiationRef, LiteralIr, TypeRefIr};
use skiff_compiler_projection_input::{EntryFunctionSignature, EntryTypeSpec};

pub fn response_type_ir(ty: &EntryTypeSpec) -> TypeRefIr {
    match &ty.ir {
        TypeRefIr::Native { name, args } if name == "Stream" && args.len() == 1 => args[0].clone(),
        TypeRefIr::AnyInterface { .. } => ty.ir.clone(),
        _ => ty.ir.clone(),
    }
}

pub fn entry_type_source_text_with_named_types(
    ty: &EntryTypeSpec,
    named_type: &impl Fn(&str) -> String,
) -> String {
    type_ref_ir_source_text_with_named_types(
        &ty.ir,
        &|type_index| ty.local_type_names.get(&type_index).cloned(),
        named_type,
    )
}

pub fn entry_function_type_ref_source_text(
    signature: &EntryFunctionSignature,
    ty: &TypeRefIr,
) -> String {
    type_ref_ir_source_text_with_local_types(ty, &|type_index| {
        signature.local_type_names.get(&type_index).cloned()
    })
}

pub fn type_ref_ir_source_text_with_local_types(
    ty: &TypeRefIr,
    local_type_name: &impl Fn(u32) -> Option<String>,
) -> String {
    type_ref_ir_source_text_with_named_types(ty, local_type_name, &|name| name.to_string())
}

fn type_ref_ir_source_text_with_named_types(
    ty: &TypeRefIr,
    local_type_name: &impl Fn(u32) -> Option<String>,
    named_type: &impl Fn(&str) -> String,
) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => named_type(name),
        TypeRefIr::Native { name, args } => format!(
            "{}<{}>",
            named_type(name),
            args.iter()
                .map(|arg| {
                    type_ref_ir_source_text_with_named_types(arg, local_type_name, named_type)
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => named_type(
            &local_type_name(*type_index)
                .unwrap_or_else(|| format!("__invalid_local_type_{type_index}")),
        ),
        TypeRefIr::PublicationType { module_path, .. } => {
            named_type(&format!("root.{module_path}"))
        }
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            let name = if symbol.module_path.is_empty() {
                symbol.symbol.clone()
            } else if symbol.module_path.starts_with("std.") {
                symbol.symbol_path()
            } else {
                format!("root.{}", symbol.symbol_path())
            };
            named_type(&name)
        }
        TypeRefIr::PackageSymbol { symbol } => named_type(&symbol.symbol_path),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        type_ref_ir_source_text_with_named_types(ty, local_type_name, named_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Union { items } => items
            .iter()
            .map(|item| type_ref_ir_source_text_with_named_types(item, local_type_name, named_type))
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => format!(
            "{}?",
            type_ref_ir_source_text_with_named_types(inner, local_type_name, named_type)
        ),
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => {
                serde_json::to_string(value).expect("string literal should serialize")
            }
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::AnyInterface { interface } => {
            any_interface_source_text(interface, local_type_name, named_type)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| {
                    format!(
                        "{}: {}",
                        param.name,
                        type_ref_ir_source_text_with_named_types(
                            &param.ty,
                            local_type_name,
                            named_type
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            type_ref_ir_source_text_with_named_types(return_type, local_type_name, named_type)
        ),
    }
}

fn any_interface_source_text(
    interface: &InterfaceInstantiationRef,
    local_type_name: &impl Fn(u32) -> Option<String>,
    named_type: &impl Fn(&str) -> String,
) -> String {
    let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map_or_else(
            |_| interface.interface_abi_id.clone(),
            |ty| type_ref_ir_source_text_with_named_types(&ty, local_type_name, named_type),
        );
    if interface.canonical_type_args.is_empty() {
        format!("any {interface_name}")
    } else {
        format!(
            "any {interface_name}<{}>",
            interface
                .canonical_type_args
                .iter()
                .map(|arg| type_ref_ir_source_text_with_named_types(
                    arg,
                    local_type_name,
                    named_type
                ))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

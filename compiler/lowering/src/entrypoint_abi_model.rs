use std::collections::BTreeMap;

use crate::file_ir::{LiteralIr, TypeRefIr};

#[derive(Debug, Clone)]
pub struct EntryFunctionSignature {
    pub name: String,
    pub params: Vec<EntryParamSpec>,
    pub return_type: EntryTypeSpec,
    pub local_type_names: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct EntryParamSpec {
    pub name: String,
    pub ty: EntryTypeSpec,
}

#[derive(Debug, Clone)]
pub struct EntryTypeSpec {
    pub name: String,
    pub ir: TypeRefIr,
    pub local_type_names: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub struct PackageAbiType {
    pub name: String,
    pub descriptor: PackageAbiTypeDescriptor,
    pub discriminator: Option<String>,
    pub local_type_names: BTreeMap<u32, String>,
}

#[derive(Debug, Clone)]
pub enum PackageAbiTypeDescriptor {
    Alias { target: TypeRefIr },
    Union { variants: Vec<TypeRefIr> },
    Record { fields: BTreeMap<String, TypeRefIr> },
    External,
}

impl EntryTypeSpec {
    pub fn response_type_ir(&self) -> TypeRefIr {
        match &self.ir {
            TypeRefIr::Native { name, args } if name == "Stream" && args.len() == 1 => {
                args[0].clone()
            }
            _ => self.ir.clone(),
        }
    }

    pub fn source_text_with_named_types(&self, named_type: &impl Fn(&str) -> String) -> String {
        type_ref_ir_source_text_with_named_types(
            &self.ir,
            &|type_index| self.local_type_names.get(&type_index).cloned(),
            named_type,
        )
    }
}

impl EntryFunctionSignature {
    pub fn type_ref_source_text(&self, ty: &TypeRefIr) -> String {
        type_ref_ir_source_text_with_local_types(ty, &|type_index| {
            self.local_type_names.get(&type_index).cloned()
        })
    }
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
                .map(|arg| type_ref_ir_source_text_with_named_types(
                    arg,
                    local_type_name,
                    named_type
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => named_type(
            &local_type_name(*type_index)
                .unwrap_or_else(|| format!("__invalid_local_type_{type_index}")),
        ),
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
                .map(|(name, ty)| format!(
                    "{name}: {}",
                    type_ref_ir_source_text_with_named_types(ty, local_type_name, named_type)
                ))
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
        TypeRefIr::AnyInterface { interface } => {
            if interface.canonical_type_args.is_empty() {
                format!("any {}", interface.interface_abi_id)
            } else {
                format!(
                    "any {}<{}>",
                    interface.interface_abi_id,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| type_ref_ir_source_text_with_named_types(
                            arg,
                            local_type_name,
                            named_type,
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => {
                serde_json::to_string(value).expect("string literal should serialize")
            }
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| format!(
                    "{}: {}",
                    param.name,
                    type_ref_ir_source_text_with_named_types(
                        &param.ty,
                        local_type_name,
                        named_type
                    )
                ))
                .collect::<Vec<_>>()
                .join(", "),
            type_ref_ir_source_text_with_named_types(return_type, local_type_name, named_type)
        ),
    }
}

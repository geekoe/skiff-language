use std::collections::BTreeMap;

use crate::file_ir::{LiteralIr, TypeRefIr};
use skiff_artifact_model::{InterfaceInstantiationRef, NominalTypeRefBaseIr};

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

/// Package/public declaration handoff uses the canonical File IR declaration
/// model directly. Keeping aliases here avoids a second descriptor DTO while
/// callers migrate to the canonical owner.
pub type PackageAbiType = crate::file_ir::TypeDeclIr;
pub type PackageAbiTypeDescriptor = crate::file_ir::TypeDescriptorIr;

impl EntryTypeSpec {
    pub fn response_type_ir(&self) -> TypeRefIr {
        match &self.ir {
            TypeRefIr::Builtin { name, args } if name == "Stream" && args.len() == 1 => {
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
        TypeRefIr::Builtin { name, args } if args.is_empty() => named_type(name),
        TypeRefIr::Builtin { name, args } => format!(
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
        TypeRefIr::AppliedNominal { base, arguments } => format!(
            "{}<{}>",
            nominal_base_source_text(base, local_type_name, named_type),
            arguments
                .iter()
                .map(|argument| type_ref_ir_source_text_with_named_types(
                    argument,
                    local_type_name,
                    named_type,
                ))
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
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => named_type(&format!("{package_id}::{stable_schema_key}")),
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
            any_interface_source_text(interface, local_type_name, named_type)
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

fn nominal_base_source_text(
    base: &NominalTypeRefBaseIr,
    local_type_name: &impl Fn(u32) -> Option<String>,
    named_type: &impl Fn(&str) -> String,
) -> String {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => named_type(
            &local_type_name(*type_index)
                .unwrap_or_else(|| format!("__invalid_local_type_{type_index}")),
        ),
        NominalTypeRefBaseIr::PublicationType { module_path, .. } => {
            named_type(&format!("root.{module_path}"))
        }
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => {
            let name = if symbol.module_path.is_empty() {
                symbol.symbol.clone()
            } else if symbol.module_path.starts_with("std.") {
                symbol.symbol_path()
            } else {
                format!("root.{}", symbol.symbol_path())
            };
            named_type(&name)
        }
        NominalTypeRefBaseIr::PackageSymbol { symbol } => named_type(&symbol.symbol_path),
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => named_type(&format!("{package_id}::{stable_schema_key}")),
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
                .map(|arg| {
                    type_ref_ir_source_text_with_named_types(arg, local_type_name, named_type)
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_ir::{ServiceSymbolRef, TypeRefIr};
    use skiff_artifact_identity::interface_instantiation_ref;

    #[test]
    fn any_interface_source_text_renders_structured_interface_identity_as_type_syntax() {
        let ty = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(
                TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "tools".to_string(),
                        symbol: "ToolProvider".to_string(),
                    },
                },
                Vec::new(),
            ),
        };

        assert_eq!(
            type_ref_ir_source_text_with_local_types(&ty, &|_| None),
            "any root.tools.ToolProvider"
        );
    }

    #[test]
    fn any_interface_source_text_preserves_canonical_type_arguments() {
        let ty = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(
                TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "tools".to_string(),
                        symbol: "Provider".to_string(),
                    },
                },
                vec![TypeRefIr::builtin("string")],
            ),
        };

        assert_eq!(
            type_ref_ir_source_text_with_local_types(&ty, &|_| None),
            "any root.tools.Provider<string>"
        );
        assert_eq!(
            crate::type_lowering::type_ref_ir_type_text(&ty),
            "any tools.Provider<string>"
        );
    }
}

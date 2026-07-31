use std::collections::BTreeMap;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::{
    compile_identity::PackageSchemaTypeId,
    publication_abi::InterfaceInstantiationRef,
    refs::SourceSpanRef,
    symbols::{PackageSymbolRef, ServiceSymbolRef},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum LiteralIr {
    Null,
    Bool { value: bool },
    Number { value: serde_json::Number },
    String { value: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FunctionTypeParamIr {
    pub name: String,
    pub ty: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NominalTypeRefBaseIr {
    LocalType {
        type_index: u32,
    },
    PublicationType {
        module_path: String,
        type_index: u32,
    },
    ServiceSymbol {
        symbol: ServiceSymbolRef,
    },
    PackageSymbol {
        symbol: PackageSymbolRef,
    },
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum TypeRefIr {
    Builtin {
        name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<TypeRefIr>,
    },
    LocalType {
        type_index: u32,
    },
    PublicationType {
        module_path: String,
        type_index: u32,
    },
    ServiceSymbol {
        symbol: ServiceSymbolRef,
    },
    PackageSymbol {
        symbol: PackageSymbolRef,
    },
    /// Exact Package-owned nominal schema identity. This is used at explicit
    /// serialization boundaries; ordinary cross-package source references
    /// remain `PackageSymbol`.
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
    AppliedNominal {
        base: NominalTypeRefBaseIr,
        #[serde(deserialize_with = "deserialize_non_empty_type_arguments")]
        arguments: Vec<TypeRefIr>,
    },
    DbObjectSymbol {
        symbol: ServiceSymbolRef,
    },
    Record {
        fields: BTreeMap<String, TypeRefIr>,
    },
    Union {
        items: Vec<TypeRefIr>,
    },
    Nullable {
        inner: Box<TypeRefIr>,
    },
    Literal {
        value: LiteralIr,
    },
    TypeParam {
        name: String,
    },
    AnyInterface {
        interface: InterfaceInstantiationRef,
    },
    Function {
        params: Vec<FunctionTypeParamIr>,
        return_type: Box<TypeRefIr>,
    },
}

impl TypeRefIr {
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin {
            name: name.into(),
            args: Vec::new(),
        }
    }
}

fn deserialize_non_empty_type_arguments<'de, D>(deserializer: D) -> Result<Vec<TypeRefIr>, D::Error>
where
    D: Deserializer<'de>,
{
    let arguments = Vec::<TypeRefIr>::deserialize(deserializer)?;
    if arguments.is_empty() {
        return Err(D::Error::custom(
            "appliedNominal arguments must be non-empty",
        ));
    }
    Ok(arguments)
}

pub(crate) fn visit_type_ref<E>(
    ty: &TypeRefIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    visitor(ty)?;
    match ty {
        TypeRefIr::Builtin { args, .. } => {
            for argument in args {
                visit_type_ref(argument, visitor)?;
            }
        }
        TypeRefIr::AppliedNominal { arguments, .. } => {
            for argument in arguments {
                visit_type_ref(argument, visitor)?;
            }
        }
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                visit_type_ref(field, visitor)?;
            }
        }
        TypeRefIr::Union { items } => {
            for item in items {
                visit_type_ref(item, visitor)?;
            }
        }
        TypeRefIr::Nullable { inner } => visit_type_ref(inner, visitor)?,
        TypeRefIr::AnyInterface { interface } => {
            for argument in &interface.canonical_type_args {
                visit_type_ref(argument, visitor)?;
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            for parameter in params {
                visit_type_ref(&parameter.ty, visitor)?;
            }
            visit_type_ref(return_type, visitor)?;
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => {}
    }
    Ok(())
}

pub(crate) fn visit_type_descriptor_type_refs<E>(
    descriptor: &TypeDescriptorIr,
    visitor: &mut impl FnMut(&TypeRefIr) -> Result<(), E>,
) -> Result<(), E> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values() {
                visit_type_ref(field, visitor)?;
            }
        }
        TypeDescriptorIr::Representation { representation } => {
            visit_type_ref(representation, visitor)?;
        }
        TypeDescriptorIr::Union { branches } => {
            for branch in branches {
                match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        visit_type_ref(nominal_type, visitor)?;
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                        visit_type_ref(payload_type, visitor)?;
                    }
                    NamedUnionBranchIr::Literal { .. } => {}
                }
            }
        }
        TypeDescriptorIr::Alias { target } => visit_type_ref(target, visitor)?,
        TypeDescriptorIr::Interface => {}
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceDeclIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<InterfaceOperationIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InterfaceOperationIr {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub is_native: bool,
    pub is_provider: bool,
    pub is_static: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implicit_self: Option<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeDeclIr {
    pub name: String,
    pub descriptor: TypeDescriptorIr,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub type_params: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub implements: Vec<TypeRefIr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpanRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum TypeDescriptorIr {
    /// A nominal record declaration.
    Record { fields: BTreeMap<String, TypeRefIr> },
    /// A nominal representation declaration. This is never a transparent
    /// alias, even when the representation is primitive-backed.
    Representation { representation: TypeRefIr },
    /// A named union declaration. Anonymous unions remain `TypeRefIr::Union`.
    Union { branches: Vec<NamedUnionBranchIr> },
    /// A transparent alias declaration. Catch identity expands through it.
    Alias { target: TypeRefIr },
    /// An interface declaration. Operations remain in `FileDeclarations`.
    Interface,
}

/// Identity input for one branch of a named union.
///
/// The enclosing `TypeDeclIr` is the nominal union owner. Runtime identity
/// combines that owner (including fully-instantiated type arguments) with
/// exactly one of these branch inputs, so equal branch shapes in two named
/// unions cannot alias.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum NamedUnionBranchIr {
    ConcreteNominal {
        nominal_type: TypeRefIr,
    },
    SyntheticDiscriminator {
        payload_type: TypeRefIr,
        discriminator_field: String,
        discriminator_value: String,
    },
    Literal {
        value: LiteralIr,
    },
}

#[cfg(test)]
mod tests;

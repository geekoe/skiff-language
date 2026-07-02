use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
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
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum TypeRefIr {
    #[serde(rename = "builtin")]
    Native {
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
    pub fn native(name: impl Into<String>) -> Self {
        Self::Native {
            name: name.into(),
            args: Vec::new(),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discriminator: Option<String>,
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
    Record {
        fields: BTreeMap<String, TypeRefIr>,
    },
    Alias {
        target: TypeRefIr,
    },
    Union {
        variants: Vec<TypeRefIr>,
    },
    #[serde(rename = "external")]
    Native {
        symbol: String,
    },
}

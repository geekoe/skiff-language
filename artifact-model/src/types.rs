use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
    Record { fields: BTreeMap<String, TypeRefIr> },
    Alias { target: TypeRefIr },
    Union { variants: Vec<TypeRefIr> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_schema_type_ref_round_trips_exact_identity() {
        let expected = TypeRefIr::PackageSchema {
            package_id: "skiff.run/llm-api".to_string(),
            stable_schema_key: "LlmRequest".to_string(),
            package_schema_type_id: PackageSchemaTypeId::new("schema:request"),
        };
        let wire = serde_json::to_value(&expected).unwrap();
        assert_eq!(wire["kind"], "packageSchema");
        assert_eq!(wire["packageId"], "skiff.run/llm-api");
        assert_eq!(wire["stableSchemaKey"], "LlmRequest");
        assert_eq!(wire["packageSchemaTypeId"], "schema:request");
        assert_eq!(serde_json::from_value::<TypeRefIr>(wire).unwrap(), expected);
    }

    #[test]
    fn package_schema_type_ref_rejects_missing_or_unknown_identity_fields() {
        for wire in [
            serde_json::json!({
                "kind": "packageSchema",
                "packageId": "skiff.run/llm-api",
                "stableSchemaKey": "LlmRequest"
            }),
            serde_json::json!({
                "kind": "packageSchema",
                "packageId": "skiff.run/llm-api",
                "stableSchemaKey": "LlmRequest",
                "packageSchemaTypeId": "schema:request",
                "abiTypeId": "wrong-domain"
            }),
        ] {
            assert!(serde_json::from_value::<TypeRefIr>(wire).is_err());
        }
    }
}

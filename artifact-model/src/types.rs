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
        type_arguments: BTreeMap<String, TypeRefIr>,
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

    #[test]
    fn declaration_descriptors_distinguish_all_canonical_kinds() {
        let descriptors = [
            TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("string"),
            },
            TypeDescriptorIr::Union {
                branches: vec![NamedUnionBranchIr::Literal {
                    value: LiteralIr::String {
                        value: "ready".to_string(),
                    },
                }],
            },
            TypeDescriptorIr::Alias {
                target: TypeRefIr::builtin("string"),
            },
            TypeDescriptorIr::Interface,
        ];
        let expected_kinds = ["record", "representation", "union", "alias", "interface"];

        for (descriptor, expected_kind) in descriptors.into_iter().zip(expected_kinds) {
            let wire = serde_json::to_value(&descriptor).unwrap();
            assert_eq!(wire["kind"], expected_kind);
            assert_eq!(
                serde_json::from_value::<TypeDescriptorIr>(wire).unwrap(),
                descriptor
            );
        }
    }

    #[test]
    fn named_union_preserves_all_branch_identity_inputs() {
        let descriptor = TypeDescriptorIr::Union {
            branches: vec![
                NamedUnionBranchIr::ConcreteNominal {
                    nominal_type: TypeRefIr::LocalType { type_index: 1 },
                    type_arguments: BTreeMap::from([(
                        "T".to_string(),
                        TypeRefIr::builtin("string"),
                    )]),
                },
                NamedUnionBranchIr::SyntheticDiscriminator {
                    payload_type: TypeRefIr::Record {
                        fields: BTreeMap::new(),
                    },
                    discriminator_field: "kind".to_string(),
                    discriminator_value: "retryable".to_string(),
                },
                NamedUnionBranchIr::Literal {
                    value: LiteralIr::Bool { value: true },
                },
            ],
        };
        let wire = serde_json::to_value(&descriptor).unwrap();

        assert_eq!(wire["branches"][0]["typeArguments"]["T"]["name"], "string");
        assert_eq!(wire["branches"][1]["discriminatorField"], "kind");
        assert_eq!(wire["branches"][2]["value"]["kind"], "bool");
        assert_eq!(
            serde_json::from_value::<TypeDescriptorIr>(wire).unwrap(),
            descriptor
        );
    }
}

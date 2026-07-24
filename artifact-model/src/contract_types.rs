use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{compile_identity::ContractTypeId, types::TypeRefIr};

/// A literal value whose exact payload participates in a ServiceContract type.
///
/// V2 intentionally exposes only string literals. Other literal domains must
/// not become boundary wire protocols until their canonical representation is
/// defined explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractLiteral {
    String { value: String },
}

/// A type reference inside a ServiceContract boundary schema.
///
/// Contract references are nominal and carry ContractTypeId directly. Inline
/// containers remain structural and recursively participate in closure checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractTypeRef {
    Builtin {
        name: String,
        arguments: Vec<ContractTypeRef>,
    },
    Contract {
        contract_type_id: ContractTypeId,
    },
    /// Exact nominal reference owned by a PackageArtifact public API.
    ///
    /// This form is valid only in a package boundary projection. Service API
    /// projection must replace it with the service-owned ContractTypeId before
    /// compiling a ServiceContract.
    PackagePublic {
        local_type_id: String,
    },
    TypeParam {
        name: String,
    },
    Record {
        fields: BTreeMap<String, ContractTypeRef>,
    },
    StructuralUnion {
        variants: Vec<ContractTypeRef>,
    },
    Nullable {
        inner: Box<ContractTypeRef>,
    },
    Literal {
        value: ContractLiteral,
    },
}

impl ContractTypeRef {
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::Builtin {
            name: name.into(),
            arguments: Vec::new(),
        }
    }

    pub fn contract(contract_type_id: ContractTypeId) -> Self {
        Self::Contract { contract_type_id }
    }

    pub fn structural_union(variants: Vec<Self>) -> Self {
        Self::StructuralUnion { variants }
    }

    pub fn string_literal(value: impl Into<String>) -> Self {
        Self::Literal {
            value: ContractLiteral::String {
                value: value.into(),
            },
        }
    }
}

/// Package-local callable signatures have their own type domain. A contract
/// nominal reference is therefore explicit and cannot be confused with an
/// AbiTypeId or hidden inside a legacy TypeRefIr display string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PackageTypeRef {
    Local {
        local_type: TypeRefIr,
    },
    Contract {
        contract_type_id: ContractTypeId,
    },
    Container {
        name: String,
        arguments: Vec<PackageTypeRef>,
    },
    Nullable {
        inner: Box<PackageTypeRef>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContractTypeNameability {
    PublicNameable,
    ClosureOnly,
}

/// One stable tag-to-type entry in a named discriminated union.
///
/// The definition compiler canonicalizes the surrounding list by `tag`; the
/// materialized ServiceContract validator rejects duplicates and non-canonical
/// order instead of silently rewriting loaded artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractDiscriminatedUnionBranch {
    pub tag: String,
    pub branch_type: ContractTypeRef,
}

impl ContractDiscriminatedUnionBranch {
    pub fn new(tag: impl Into<String>, branch_type: ContractTypeRef) -> Self {
        Self {
            tag: tag.into(),
            branch_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ContractTypeDescriptor {
    Record {
        fields: BTreeMap<String, ContractTypeRef>,
    },
    StructuralUnion {
        variants: Vec<ContractTypeRef>,
    },
    DiscriminatedUnion {
        discriminator_field: String,
        branches: Vec<ContractDiscriminatedUnionBranch>,
    },
    Representation {
        target: ContractTypeRef,
    },
    Alias {
        target: ContractTypeRef,
    },
    Enumeration {
        variants: Vec<String>,
    },
    CallbackInterface {
        operations: BTreeMap<String, BoundaryCallbackOperation>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BoundaryCallbackOperation {
    pub parameters: Vec<ContractTypeRef>,
    pub return_type: ContractTypeRef,
    pub may_suspend: bool,
}

/// Reusable semantic body for a schema entry. The definition compiler adds the
/// stable key and derived ContractTypeId around this body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractTypeShape {
    pub nameability: ContractTypeNameability,
    pub type_params: Vec<String>,
    pub descriptor: ContractTypeDescriptor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractSchemaType {
    pub contract_type_id: ContractTypeId,
    pub stable_key: String,
    pub shape: ContractTypeShape,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn contract_type_ref_is_strict_and_nominal_id_is_explicit() {
        let reference = ContractTypeRef::contract(ContractTypeId::new("contract-type"));
        assert_eq!(
            serde_json::to_value(reference).unwrap(),
            json!({ "kind": "contract", "contractTypeId": "contract-type" })
        );
        for invalid in [
            json!({ "kind": "contract" }),
            json!({ "kind": "contract", "abiTypeId": "legacy" }),
            json!({
                "kind": "contract",
                "contractTypeId": "contract-type",
                "displayName": "not semantic"
            }),
        ] {
            assert!(serde_json::from_value::<ContractTypeRef>(invalid).is_err());
        }
    }

    #[test]
    fn v2_literal_and_structural_union_refs_have_strict_typed_wire() {
        let reference = ContractTypeRef::structural_union(vec![
            ContractTypeRef::string_literal("created"),
            ContractTypeRef::builtin("null"),
        ]);
        let wire = json!({
            "kind": "structuralUnion",
            "variants": [
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "created" }
                },
                { "kind": "builtin", "name": "null", "arguments": [] }
            ]
        });
        assert_eq!(serde_json::to_value(&reference).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ContractTypeRef>(wire.clone()).unwrap(),
            reference
        );

        for invalid in [
            json!({ "kind": "union", "variants": [] }),
            json!({ "kind": "structuralUnion" }),
            json!({
                "kind": "structuralUnion",
                "variants": [],
                "legacyDiscriminator": "kind"
            }),
            json!({ "kind": "literal" }),
            json!({ "kind": "literal", "value": { "kind": "string" } }),
            json!({
                "kind": "literal",
                "value": { "kind": "string", "value": "created", "extra": true }
            }),
        ] {
            assert!(
                serde_json::from_value::<ContractTypeRef>(invalid.clone()).is_err(),
                "non-v2 or incomplete ref must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn v2_discriminated_union_descriptor_round_trips_strict_branch_entries() {
        let descriptor = ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field: "kind".to_string(),
            branches: vec![ContractDiscriminatedUnionBranch::new(
                "created",
                ContractTypeRef::Record {
                    fields: BTreeMap::from([(
                        "kind".to_string(),
                        ContractTypeRef::string_literal("created"),
                    )]),
                },
            )],
        };
        let wire = json!({
            "kind": "discriminatedUnion",
            "discriminatorField": "kind",
            "branches": [{
                "tag": "created",
                "branchType": {
                    "kind": "record",
                    "fields": {
                        "kind": {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "created" }
                        }
                    }
                }
            }]
        });
        assert_eq!(serde_json::to_value(&descriptor).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ContractTypeDescriptor>(wire).unwrap(),
            descriptor
        );

        for invalid in [
            json!({ "kind": "discriminatedUnion", "branches": [] }),
            json!({ "kind": "discriminatedUnion", "discriminatorField": "kind" }),
            json!({
                "kind": "discriminatedUnion",
                "discriminatorField": "kind",
                "branches": [{
                    "branchType": { "kind": "builtin", "name": "string", "arguments": [] }
                }]
            }),
            json!({
                "kind": "discriminatedUnion",
                "discriminatorField": "kind",
                "branches": [{ "tag": "created" }]
            }),
            json!({
                "kind": "discriminatedUnion",
                "discriminatorField": "kind",
                "branches": [{
                    "tag": "created",
                    "branchType": { "kind": "builtin", "name": "string", "arguments": [] },
                    "legacyBranchId": "branch"
                }]
            }),
        ] {
            assert!(
                serde_json::from_value::<ContractTypeDescriptor>(invalid.clone()).is_err(),
                "incomplete discriminated union must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn v2_structural_union_and_representation_descriptors_reject_legacy_shapes() {
        let structural = ContractTypeDescriptor::StructuralUnion {
            variants: vec![ContractTypeRef::builtin("string")],
        };
        let representation = ContractTypeDescriptor::Representation {
            target: ContractTypeRef::builtin("string"),
        };
        for descriptor in [&structural, &representation] {
            let wire = serde_json::to_value(descriptor).unwrap();
            assert_eq!(
                serde_json::from_value::<ContractTypeDescriptor>(wire).unwrap(),
                *descriptor
            );
        }
        assert_eq!(
            serde_json::to_value(&structural).unwrap(),
            json!({
                "kind": "structuralUnion",
                "variants": [{ "kind": "builtin", "name": "string", "arguments": [] }]
            })
        );
        assert_eq!(
            serde_json::to_value(&representation).unwrap(),
            json!({
                "kind": "representation",
                "target": { "kind": "builtin", "name": "string", "arguments": [] }
            })
        );

        for invalid in [
            json!({ "kind": "union", "variants": [] }),
            json!({ "kind": "structuralUnion" }),
            json!({ "kind": "representation" }),
            json!({
                "kind": "representation",
                "target": { "kind": "builtin", "name": "string", "arguments": [] },
                "transparent": true
            }),
        ] {
            assert!(
                serde_json::from_value::<ContractTypeDescriptor>(invalid.clone()).is_err(),
                "legacy or incomplete descriptor must fail closed: {invalid}"
            );
        }
    }

    #[test]
    fn generic_shape_and_type_parameter_have_strict_wire() {
        let shape = ContractTypeShape {
            nameability: ContractTypeNameability::PublicNameable,
            type_params: vec!["T".to_string()],
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    ContractTypeRef::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
        };
        let wire = json!({
            "nameability": "publicNameable",
            "typeParams": ["T"],
            "descriptor": {
                "kind": "record",
                "fields": {
                    "value": { "kind": "typeParam", "name": "T" }
                }
            }
        });
        assert_eq!(serde_json::to_value(&shape).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ContractTypeShape>(wire.clone()).unwrap(),
            shape
        );

        for invalid in [
            json!({
                "nameability": "publicNameable",
                "descriptor": wire["descriptor"].clone()
            }),
            json!({
                "nameability": "publicNameable",
                "typeParams": ["T"],
                "descriptor": {
                    "kind": "record",
                    "fields": { "value": { "kind": "typeParam" } }
                }
            }),
            json!({
                "nameability": "publicNameable",
                "typeParams": ["T"],
                "descriptor": wire["descriptor"].clone(),
                "displayType": "Box<T>"
            }),
        ] {
            assert!(serde_json::from_value::<ContractTypeShape>(invalid).is_err());
        }
    }
}

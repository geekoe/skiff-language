use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{compile_identity::ContractTypeId, types::TypeRefIr};

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
    Record {
        fields: BTreeMap<String, ContractTypeRef>,
    },
    Union {
        variants: Vec<ContractTypeRef>,
    },
    Nullable {
        inner: Box<ContractTypeRef>,
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
    Union {
        variants: Vec<ContractTypeRef>,
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
}

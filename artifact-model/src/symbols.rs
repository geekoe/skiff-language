use serde::{Deserialize, Serialize};

use crate::publication_abi::OperationAbiRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceSymbolRef {
    pub module_path: String,
    pub symbol: String,
}

impl ServiceSymbolRef {
    pub fn symbol_path(&self) -> String {
        format!("{}.{}", self.module_path, self.symbol)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageSymbolRef {
    pub package: PackageRefIr,
    pub symbol_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abi_expectation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageOperationSymbolRef {
    pub package_ref: PackageRefIr,
    pub operation: OperationAbiRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDependencySymbolRef {
    pub dependency_ref: String,
    pub operation: OperationAbiRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields,
    tag = "kind"
)]
pub enum PackageRefIr {
    PackageId { package_id: String },
    Dependency { dependency_ref: String },
}

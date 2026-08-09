use std::collections::BTreeMap;

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::{
    boundary::BoundaryOperationDescriptor,
    compile_identity::{ContractOperationId, PackageSchemaTypeId, ServiceProtocolIdentity},
    contract_types::PackageTypeRequirement,
    publication_abi::InterfaceInstantiationRef,
};

/// One public-instance method slot. Vector position in the enclosing
/// interface table is the slot; no provider implementation fact is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractPublicInstanceMethod {
    pub method_abi_id: String,
    pub contract_operation_id: ContractOperationId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractPublicInstanceInterface {
    pub interface: InterfaceInstantiationRef,
    pub methods: Vec<ContractPublicInstanceMethod>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractPublicInstance {
    #[serde(deserialize_with = "deserialize_canonical_public_instance_interfaces")]
    pub interfaces: Vec<ContractPublicInstanceInterface>,
}

fn deserialize_canonical_public_instance_interfaces<'de, D>(
    deserializer: D,
) -> Result<Vec<ContractPublicInstanceInterface>, D::Error>
where
    D: Deserializer<'de>,
{
    let rows = Vec::<ContractPublicInstanceInterface>::deserialize(deserializer)?;
    let mut previous: Option<Vec<u8>> = None;
    for row in &rows {
        let key =
            skiff_canonical_json::canonical_json_bytes(&row.interface).map_err(D::Error::custom)?;
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err(D::Error::custom(
                "public instance interfaces must be strictly ordered and unique by exact interface instantiation",
            ));
        }
        previous = Some(key);
    }
    Ok(rows)
}

/// Human-facing text is carried with the artifact but is deliberately outside
/// ServiceProtocolIdentity. It must never be used to resolve an operation or a
/// schema type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractDiagnosticText {
    pub service: String,
    pub operations: BTreeMap<ContractOperationId, String>,
    pub types: BTreeMap<PackageSchemaTypeId, String>,
}

/// Independent, code-free service protocol artifact.
///
/// Provider package/build, deployment, route, config and runtime fields do not
/// exist in this type. Serde's deny_unknown_fields makes that boundary strict on
/// the wire as well as in Rust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContract {
    pub schema_version: String,
    pub service_id: String,
    pub contract_version: String,
    pub service_protocol_identity: ServiceProtocolIdentity,
    pub operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    pub public_instances: BTreeMap<String, ContractPublicInstance>,
    pub package_type_requirements: Vec<PackageTypeRequirement>,
    pub diagnostic_text: ContractDiagnosticText,
}

#[cfg(test)]
mod tests;

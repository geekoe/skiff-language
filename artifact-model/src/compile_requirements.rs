use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::compile_identity::{
    ContractOperationId, PackageLocalAbiIdentity, ServiceProtocolIdentity,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRequirement {
    pub alias: String,
    pub package_id: String,
    pub exact_version: String,
    pub expected_local_abi: PackageLocalAbiIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractRequirement {
    pub alias: String,
    pub service_id: String,
    pub contract_version: String,
    pub expected_protocol_identity: ServiceProtocolIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRequirement {
    pub contract_requirement: ContractRequirement,
    pub service_binding_slot: u32,
    pub used_operations: BTreeSet<ContractOperationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceCallRef {
    pub service_requirement_slot: u32,
    pub contract_operation_id: ContractOperationId,
    pub expected_protocol_identity: ServiceProtocolIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageConfigRequirement {
    pub path: String,
    pub value_type: String,
    pub required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageResourceRequirement {
    pub key: String,
    pub capability: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRuntimeCapabilityRequirement {
    pub capability: String,
    pub required_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRuntimeRequirements {
    pub config: Vec<PackageConfigRequirement>,
    pub resources: Vec<PackageResourceRequirement>,
    pub runtime_capabilities: Vec<PackageRuntimeCapabilityRequirement>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn service_call_ref_rejects_provider_and_missing_contract_identity() {
        let complete = json!({
            "serviceRequirementSlot": 0,
            "contractOperationId": "operation",
            "expectedProtocolIdentity": "protocol"
        });
        serde_json::from_value::<ServiceCallRef>(complete.clone()).unwrap();

        let mut missing = complete.clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove("expectedProtocolIdentity");
        assert!(serde_json::from_value::<ServiceCallRef>(missing).is_err());

        let mut provider = complete;
        provider
            .as_object_mut()
            .unwrap()
            .insert("providerBuildId".to_string(), json!("forbidden"));
        assert!(serde_json::from_value::<ServiceCallRef>(provider).is_err());
    }
}

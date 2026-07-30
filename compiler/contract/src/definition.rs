use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skiff_artifact_model::{
    BoundaryOperationContract, PackageSchemaTypeId, PackageTypeRequirement,
};

/// Strict typed input for the code-free ServiceContract producer.
///
/// Map keys are stable authoring keys, not display names guessed from provider
/// source. ContractTypeRef values inside operation/schema bodies must use the
/// identity derived from the same service coordinate and stable type key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinition {
    pub service_id: String,
    pub contract_version: String,
    pub operations: BTreeMap<String, BoundaryOperationContract>,
    pub package_type_requirements: Vec<PackageTypeRequirement>,
    pub diagnostic_text: ServiceContractDefinitionDiagnosticText,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinitionDiagnosticText {
    pub service: String,
    pub operations: BTreeMap<String, String>,
    pub types: BTreeMap<PackageSchemaTypeId, String>,
}

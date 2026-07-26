use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    boundary::BoundaryOperationDescriptor,
    compile_identity::{ContractOperationId, PackageSchemaTypeId, ServiceProtocolIdentity},
    contract_types::PackageTypeRequirement,
};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContract {
    pub schema_version: String,
    pub service_id: String,
    pub contract_version: String,
    pub service_protocol_identity: ServiceProtocolIdentity,
    pub operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    pub package_type_requirements: Vec<PackageTypeRequirement>,
    pub diagnostic_text: ContractDiagnosticText,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn service_contract_wire_rejects_missing_and_provider_fields() {
        let minimal = json!({
            "schemaVersion": "skiff-service-contract-v5",
            "serviceId": "example.echo",
            "contractVersion": "1.0.0",
            "serviceProtocolIdentity": "protocol",
            "operations": {},
            "packageTypeRequirements": [],
            "diagnosticText": { "service": "", "operations": {}, "types": {} }
        });
        serde_json::from_value::<ServiceContract>(minimal.clone())
            .expect("complete strict contract wire");

        for field in [
            "serviceProtocolIdentity",
            "operations",
            "packageTypeRequirements",
            "diagnosticText",
        ] {
            let mut missing = minimal.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<ServiceContract>(missing).is_err(),
                "missing {field} must fail closed"
            );
        }

        for forbidden in [
            "providerPackageId",
            "providerBuildId",
            "deploymentRevision",
            "route",
            "runtimeReplica",
            "implementationRequirements",
        ] {
            let mut value = minimal.clone();
            value
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_string(), json!("forbidden"));
            assert!(
                serde_json::from_value::<ServiceContract>(value).is_err(),
                "{forbidden} must not enter ServiceContract"
            );
        }
    }
}

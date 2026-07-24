use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractDiagnosticText, ContractOperationId, ContractSchemaType,
    ContractTypeId, ServiceContract, ServiceContractDefinition, ServiceContractRef,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, CONTRACT_OPERATION_IDENTITY_PREFIX,
    CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER, CONTRACT_TYPE_IDENTITY_PREFIX,
    CONTRACT_TYPE_IDENTITY_SCHEMA_MARKER, SERVICE_PROTOCOL_IDENTITY_PREFIX,
    SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
};

mod alias_expansion;
mod normalization;
mod schema_graph;
mod schema_validation;
mod validation;

pub use alias_expansion::normalize_contract_definition_surface;
pub use normalization::{normalize_contract_operation_contract, normalize_contract_type_shape};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractTypeIdentityInput<'a> {
    schema: &'static str,
    service_id: &'a str,
    stable_type_key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractOperationIdentityInput<'a> {
    schema: &'static str,
    service_id: &'a str,
    stable_operation_key: &'a str,
}

/// The complete canonical protocol preimage. Diagnostic text and provider-side
/// implementation requirements are absent by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProtocolIdentityProjection {
    schema: &'static str,
    service_id: String,
    operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
}

pub fn contract_type_id(
    service_id: &str,
    _package_version_label: &str,
    stable_type_key: &str,
) -> Result<ContractTypeId> {
    validation::validate_coordinate_part("serviceId", service_id)?;
    validation::validate_stable_key("contract type", stable_type_key)?;
    let bytes = canonical_ir_bytes(
        &ContractTypeIdentityInput {
            schema: CONTRACT_TYPE_IDENTITY_SCHEMA_MARKER,
            service_id,
            stable_type_key,
        },
        ArtifactIdentityError::SerializeContractTypeIdentity,
    )?;
    Ok(ContractTypeId::new(framed_identity(
        CONTRACT_TYPE_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn contract_operation_id(
    service_id: &str,
    _package_version_label: &str,
    stable_operation_key: &str,
) -> Result<ContractOperationId> {
    validation::validate_coordinate_part("serviceId", service_id)?;
    validation::validate_stable_key("contract operation", stable_operation_key)?;
    let bytes = canonical_ir_bytes(
        &ContractOperationIdentityInput {
            schema: CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER,
            service_id,
            stable_operation_key,
        },
        ArtifactIdentityError::SerializeContractOperationIdentity,
    )?;
    Ok(ContractOperationId::new(framed_identity(
        CONTRACT_OPERATION_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn service_protocol_identity_projection(
    contract: &ServiceContract,
) -> Result<ServiceProtocolIdentityProjection> {
    validation::validate_service_contract_surface(contract)?;
    Ok(ServiceProtocolIdentityProjection {
        schema: SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
        service_id: contract.service_id.clone(),
        operations: contract.operations.clone(),
        boundary_schema: contract.boundary_schema.clone(),
    })
}

pub fn service_protocol_identity(contract: &ServiceContract) -> Result<ServiceProtocolIdentity> {
    let projection = service_protocol_identity_projection(contract)?;
    let bytes = canonical_ir_bytes(
        &projection,
        ArtifactIdentityError::SerializeServiceProtocolIdentity,
    )?;
    Ok(ServiceProtocolIdentity::new(framed_identity(
        SERVICE_PROTOCOL_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn service_protocol_identity_hash(identity: &str) -> Result<&str> {
    let hash = identity
        .strip_prefix(SERVICE_PROTOCOL_IDENTITY_PREFIX)
        .and_then(|suffix| suffix.strip_prefix(':'))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| ArtifactIdentityError::InvalidServiceProtocolIdentity {
            identity: identity.to_string(),
        })?;
    Ok(hash)
}

/// Assigns the protocol identity after validating the independently assigned
/// contract type and operation identities.
pub fn assign_service_contract_identities(
    contract: &mut ServiceContract,
) -> Result<ServiceProtocolIdentity> {
    let identity = service_protocol_identity(contract)?;
    contract.service_protocol_identity = identity.clone();
    validate_service_contract_identities(contract)?;
    Ok(identity)
}

/// Materializes a canonical, code-free contract definition. Stable-key maps
/// are converted to independently derived type/operation identities before the
/// protocol identity is assigned.
pub fn service_contract_from_definition(
    mut definition: ServiceContractDefinition,
) -> Result<ServiceContract> {
    definition
        .validate()
        .map_err(|message| ArtifactIdentityError::InvalidServiceContract { message })?;

    normalize_contract_definition_surface(
        &definition.service_id,
        &definition.contract_version,
        &mut definition.operations,
        &mut definition.boundary_schema,
    )?;

    let operations = definition
        .operations
        .into_iter()
        .map(|(stable_key, contract)| {
            let operation_id = contract_operation_id(
                &definition.service_id,
                &definition.contract_version,
                &stable_key,
            )?;
            Ok((
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id,
                    stable_key,
                    contract,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let boundary_schema = definition
        .boundary_schema
        .into_iter()
        .map(|(stable_key, shape)| {
            let contract_type_id = contract_type_id(
                &definition.service_id,
                &definition.contract_version,
                &stable_key,
            )?;
            Ok((
                contract_type_id.clone(),
                ContractSchemaType {
                    contract_type_id,
                    stable_key,
                    shape,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let diagnostic_text = ContractDiagnosticText {
        service: definition.diagnostic_text.service,
        operations: definition
            .diagnostic_text
            .operations
            .into_iter()
            .map(|(stable_key, text)| {
                Ok((
                    contract_operation_id(
                        &definition.service_id,
                        &definition.contract_version,
                        &stable_key,
                    )?,
                    text,
                ))
            })
            .collect::<Result<_>>()?,
        types: definition
            .diagnostic_text
            .types
            .into_iter()
            .filter(|(stable_key, _)| {
                boundary_schema
                    .values()
                    .any(|schema| &schema.stable_key == stable_key)
            })
            .map(|(stable_key, text)| {
                Ok((
                    contract_type_id(
                        &definition.service_id,
                        &definition.contract_version,
                        &stable_key,
                    )?,
                    text,
                ))
            })
            .collect::<Result<_>>()?,
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: definition.service_id,
        contract_version: definition.contract_version,
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations,
        boundary_schema,
        diagnostic_text,
    };
    assign_service_contract_identities(&mut contract)?;
    Ok(contract)
}

pub fn service_contract_ref(contract: &ServiceContract) -> Result<ServiceContractRef> {
    validate_service_contract_identities(contract)?;
    Ok(ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    })
}

pub fn validate_service_contract_identities(contract: &ServiceContract) -> Result<()> {
    let computed = service_protocol_identity(contract)?;
    if contract.service_protocol_identity != computed {
        return Err(ArtifactIdentityError::ServiceProtocolIdentityMismatch {
            declared: contract.service_protocol_identity.to_string(),
            computed: computed.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod definition_tests {
    use skiff_artifact_model::{
        BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
        BoundaryErrorContract, BoundaryOperationContract, BoundaryReturn, BoundaryStreamContract,
        BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
        BoundaryValuePlan, ContractTypeRef, ServiceContractDefinitionDiagnosticText,
        SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn code_free_definition_materializes_without_provider_or_deployment_facts() {
        let definition = ServiceContractDefinition {
            schema_version: SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION.to_string(),
            service_id: "example.com/checkpoint".to_string(),
            contract_version: "1.0.0".to_string(),
            operations: BTreeMap::from([("health".to_string(), operation_contract())]),
            boundary_schema: BTreeMap::new(),
            diagnostic_text: ServiceContractDefinitionDiagnosticText {
                service: "Checkpoint".to_string(),
                operations: BTreeMap::from([("health".to_string(), "Health".to_string())]),
                types: BTreeMap::new(),
            },
        };
        let contract = service_contract_from_definition(definition.clone()).unwrap();
        validate_service_contract_identities(&contract).unwrap();
        assert_eq!(contract.operations.len(), 1);
        let wire = serde_json::to_string(&contract).unwrap();
        for forbidden in [
            "providerPackageId",
            "providerBuildId",
            "deploymentRevision",
            "route",
        ] {
            assert!(!wire.contains(forbidden));
            let mut value = serde_json::to_value(&definition).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .insert(forbidden.to_string(), serde_json::json!("forbidden"));
            assert!(serde_json::from_value::<ServiceContractDefinition>(value).is_err());
        }
    }

    fn operation_contract() -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: ContractTypeRef::builtin("bool"),
                value_plan: BoundaryValuePlan::Linkable {
                    carrier: BoundaryValueCarrier::DetachedValueGraph,
                    encoding: BoundaryValueEncoding::CanonicalValue,
                    owner: BoundaryValueOwner::Provider,
                    lifetime: BoundaryValueLifetime::Call,
                },
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        }
    }
}

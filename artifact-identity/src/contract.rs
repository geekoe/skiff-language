use std::collections::BTreeMap;

use serde::Serialize;
use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, ContractSchemaType, ContractTypeId,
    ServiceContract, ServiceProtocolIdentity,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, CONTRACT_OPERATION_IDENTITY_PREFIX,
    CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER, CONTRACT_TYPE_IDENTITY_PREFIX,
    CONTRACT_TYPE_IDENTITY_SCHEMA_MARKER, SERVICE_PROTOCOL_IDENTITY_PREFIX,
    SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
};

mod validation;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractTypeIdentityInput<'a> {
    schema: &'static str,
    service_id: &'a str,
    contract_version: &'a str,
    stable_type_key: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractOperationIdentityInput<'a> {
    schema: &'static str,
    service_id: &'a str,
    contract_version: &'a str,
    stable_operation_key: &'a str,
}

/// The complete canonical protocol preimage. Diagnostic text and provider-side
/// implementation requirements are absent by construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProtocolIdentityProjection {
    schema: &'static str,
    service_id: String,
    contract_version: String,
    operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
}

pub fn contract_type_id(
    service_id: &str,
    contract_version: &str,
    stable_type_key: &str,
) -> Result<ContractTypeId> {
    validation::validate_coordinate_part("serviceId", service_id)?;
    validation::validate_coordinate_part("contractVersion", contract_version)?;
    validation::validate_stable_key("contract type", stable_type_key)?;
    let bytes = canonical_ir_bytes(
        &ContractTypeIdentityInput {
            schema: CONTRACT_TYPE_IDENTITY_SCHEMA_MARKER,
            service_id,
            contract_version,
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
    contract_version: &str,
    stable_operation_key: &str,
) -> Result<ContractOperationId> {
    validation::validate_coordinate_part("serviceId", service_id)?;
    validation::validate_coordinate_part("contractVersion", contract_version)?;
    validation::validate_stable_key("contract operation", stable_operation_key)?;
    let bytes = canonical_ir_bytes(
        &ContractOperationIdentityInput {
            schema: CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER,
            service_id,
            contract_version,
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
        contract_version: contract.contract_version.clone(),
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

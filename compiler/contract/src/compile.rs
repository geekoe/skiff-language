use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, contract_type_id,
};
use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractDiagnosticText, ContractOperationId, ContractSchemaType,
    ContractTypeId, ContractTypeRef, ServiceContract, ServiceProtocolIdentity,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};

use crate::{
    ContractDefinitionError, Result, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

pub fn compile_service_contract_definition(
    definition: ServiceContractDefinition,
) -> Result<ServiceContract> {
    validate_definition_keys(&definition)?;
    let type_ids = definition
        .boundary_schema
        .keys()
        .map(|stable_key| {
            Ok((
                stable_key.clone(),
                contract_type_id(
                    &definition.service_id,
                    &definition.contract_version,
                    stable_key,
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let operation_ids = definition
        .operations
        .keys()
        .map(|stable_key| {
            Ok((
                stable_key.clone(),
                contract_operation_id(
                    &definition.service_id,
                    &definition.contract_version,
                    stable_key,
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let diagnostic_text =
        project_diagnostic_text(definition.diagnostic_text, &operation_ids, &type_ids)?;

    let operations = definition
        .operations
        .into_iter()
        .map(|(stable_key, contract)| {
            let operation_id = operation_ids[&stable_key].clone();
            (
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id,
                    stable_key,
                    contract,
                },
            )
        })
        .collect();
    let boundary_schema = definition
        .boundary_schema
        .into_iter()
        .map(|(stable_key, shape)| {
            let contract_type_id = type_ids[&stable_key].clone();
            (
                contract_type_id.clone(),
                ContractSchemaType {
                    contract_type_id,
                    stable_key,
                    shape,
                },
            )
        })
        .collect();
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

/// Identity helper for typed definition producers that need to reference a
/// schema entry before the final ServiceContract map is materialized.
pub fn definition_contract_type_id(
    service_id: &str,
    contract_version: &str,
    stable_type_key: &str,
) -> Result<ContractTypeId> {
    Ok(contract_type_id(
        service_id,
        contract_version,
        stable_type_key,
    )?)
}

pub fn definition_contract_type_ref(
    service_id: &str,
    contract_version: &str,
    stable_type_key: &str,
) -> Result<ContractTypeRef> {
    Ok(ContractTypeRef::contract(definition_contract_type_id(
        service_id,
        contract_version,
        stable_type_key,
    )?))
}

pub fn definition_contract_operation_id(
    service_id: &str,
    contract_version: &str,
    stable_operation_key: &str,
) -> Result<ContractOperationId> {
    Ok(contract_operation_id(
        service_id,
        contract_version,
        stable_operation_key,
    )?)
}

fn validate_definition_keys(definition: &ServiceContractDefinition) -> Result<()> {
    if definition.operations.is_empty() {
        return Err(ContractDefinitionError::EmptyOperations);
    }
    for key in definition.operations.keys() {
        if key.trim().is_empty() {
            return Err(ContractDefinitionError::EmptyStableKey { kind: "operation" });
        }
    }
    for key in definition.boundary_schema.keys() {
        if key.trim().is_empty() {
            return Err(ContractDefinitionError::EmptyStableKey { kind: "type" });
        }
    }
    Ok(())
}

fn project_diagnostic_text(
    diagnostic: ServiceContractDefinitionDiagnosticText,
    operation_ids: &BTreeMap<String, ContractOperationId>,
    type_ids: &BTreeMap<String, ContractTypeId>,
) -> Result<ContractDiagnosticText> {
    let operations = diagnostic
        .operations
        .into_iter()
        .map(|(stable_key, text)| {
            let Some(operation_id) = operation_ids.get(&stable_key) else {
                return Err(ContractDefinitionError::UnknownDiagnosticOperation {
                    key: stable_key,
                });
            };
            Ok((operation_id.clone(), text))
        })
        .collect::<Result<_>>()?;
    let types = diagnostic
        .types
        .into_iter()
        .map(|(stable_key, text)| {
            let Some(type_id) = type_ids.get(&stable_key) else {
                return Err(ContractDefinitionError::UnknownDiagnosticType { key: stable_key });
            };
            Ok((type_id.clone(), text))
        })
        .collect::<Result<_>>()?;
    Ok(ContractDiagnosticText {
        service: diagnostic.service,
        operations,
        types,
    })
}

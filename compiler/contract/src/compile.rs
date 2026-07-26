use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id,
    normalize_contract_operation_contract,
};
use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractDiagnosticText, ContractOperationId, ServiceContract,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use crate::{
    ContractDefinitionError, Result, ServiceContractDefinition,
    ServiceContractDefinitionDiagnosticText,
};

pub fn compile_service_contract_definition(
    mut definition: ServiceContractDefinition,
) -> Result<ServiceContract> {
    validate_definition(&definition)?;
    let operation_ids = definition
        .operations
        .keys()
        .map(|key| {
            Ok((
                key.clone(),
                contract_operation_id(&definition.service_id, &definition.contract_version, key)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let operations = definition
        .operations
        .into_iter()
        .map(|(stable_key, contract)| {
            let operation_id = operation_ids[&stable_key].clone();
            let contract = normalize_contract_operation_contract(contract, &stable_key)?;
            Ok((
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id,
                    stable_key,
                    contract,
                },
            ))
        })
        .collect::<Result<_>>()?;
    definition
        .package_type_requirements
        .sort_by(|a, b| a.package_id.cmp(&b.package_id));
    for requirement in &mut definition.package_type_requirements {
        requirement.required_type_ids.sort();
        requirement.required_type_ids.dedup();
    }
    let diagnostic_text = project_diagnostic_text(definition.diagnostic_text, &operation_ids)?;
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: definition.service_id,
        contract_version: definition.contract_version,
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations,
        package_type_requirements: definition.package_type_requirements,
        diagnostic_text,
    };
    assign_service_contract_identities(&mut contract)?;
    Ok(contract)
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

fn validate_definition(definition: &ServiceContractDefinition) -> Result<()> {
    if definition
        .operations
        .keys()
        .any(|key| key.trim().is_empty())
    {
        return Err(ContractDefinitionError::EmptyStableKey { kind: "operation" });
    }
    let required = definition
        .package_type_requirements
        .iter()
        .flat_map(|requirement| requirement.required_type_ids.iter())
        .collect::<BTreeSet<_>>();
    if let Some(type_id) = definition
        .diagnostic_text
        .types
        .keys()
        .find(|type_id| !required.contains(type_id))
    {
        return Err(ContractDefinitionError::UnknownDiagnosticType {
            key: type_id.to_string(),
        });
    }
    Ok(())
}

fn project_diagnostic_text(
    diagnostic: ServiceContractDefinitionDiagnosticText,
    operation_ids: &BTreeMap<String, ContractOperationId>,
) -> Result<ContractDiagnosticText> {
    let operations = diagnostic
        .operations
        .into_iter()
        .map(|(key, text)| {
            operation_ids
                .get(&key)
                .cloned()
                .map(|id| (id, text))
                .ok_or(ContractDefinitionError::UnknownDiagnosticOperation { key })
        })
        .collect::<Result<_>>()?;
    Ok(ContractDiagnosticText {
        service: diagnostic.service,
        operations,
        types: diagnostic.types,
    })
}

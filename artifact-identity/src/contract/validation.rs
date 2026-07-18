use std::collections::BTreeSet;

use skiff_artifact_model::{ServiceContract, SERVICE_CONTRACT_SCHEMA_VERSION};

use crate::{ArtifactIdentityError, Result};

use super::schema_validation::validate_contract_schema;
use super::{contract_operation_id, contract_type_id};

pub(super) fn validate_service_contract_surface(contract: &ServiceContract) -> Result<()> {
    if contract.schema_version != SERVICE_CONTRACT_SCHEMA_VERSION {
        return invalid_contract(format!(
            "schemaVersion must be {SERVICE_CONTRACT_SCHEMA_VERSION}, got {}",
            contract.schema_version
        ));
    }
    validate_coordinate_part("serviceId", &contract.service_id)?;
    validate_coordinate_part("contractVersion", &contract.contract_version)?;
    if contract.operations.is_empty() {
        return invalid_contract("operations must contain at least one operation");
    }

    let mut stable_operation_keys = BTreeSet::new();
    for (operation_id, descriptor) in &contract.operations {
        if operation_id != &descriptor.operation_id {
            return invalid_contract(format!(
                "operation map key {operation_id} does not match nested operationId {}",
                descriptor.operation_id
            ));
        }
        if !stable_operation_keys.insert(descriptor.stable_key.as_str()) {
            return invalid_contract(format!(
                "duplicate operation stable key {}",
                descriptor.stable_key
            ));
        }
        let expected = contract_operation_id(
            &contract.service_id,
            &contract.contract_version,
            &descriptor.stable_key,
        )?;
        if operation_id != &expected {
            return invalid_contract(format!(
                "operation {} has identity {operation_id}, expected {expected}",
                descriptor.stable_key
            ));
        }
    }

    let mut stable_type_keys = BTreeSet::new();
    for (type_id, schema_type) in &contract.boundary_schema {
        if type_id != &schema_type.contract_type_id {
            return invalid_contract(format!(
                "boundary schema key {type_id} does not match nested contractTypeId {}",
                schema_type.contract_type_id
            ));
        }
        if !stable_type_keys.insert(schema_type.stable_key.as_str()) {
            return invalid_contract(format!(
                "duplicate contract type stable key {}",
                schema_type.stable_key
            ));
        }
        let expected = contract_type_id(
            &contract.service_id,
            &contract.contract_version,
            &schema_type.stable_key,
        )?;
        if type_id != &expected {
            return invalid_contract(format!(
                "contract type {} has identity {type_id}, expected {expected}",
                schema_type.stable_key
            ));
        }
    }

    validate_contract_schema(contract)?;
    validate_diagnostic_keys(contract)
}

fn validate_diagnostic_keys(contract: &ServiceContract) -> Result<()> {
    for operation_id in contract.diagnostic_text.operations.keys() {
        if !contract.operations.contains_key(operation_id) {
            return invalid_contract(format!(
                "diagnostic text references unknown operation {operation_id}"
            ));
        }
    }
    for type_id in contract.diagnostic_text.types.keys() {
        if !contract.boundary_schema.contains_key(type_id) {
            return invalid_contract(format!(
                "diagnostic text references unknown contract type {type_id}"
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_coordinate_part(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid_contract(format!("{label} must be a non-empty string"));
    }
    Ok(())
}

pub(super) fn validate_stable_key(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid_contract(format!("{label} stable key must be a non-empty string"));
    }
    Ok(())
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}

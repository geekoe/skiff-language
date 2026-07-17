use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryErrorContract, BoundaryOperationContract,
    BoundaryStreamContract, ContractTypeDescriptor, ContractTypeId, ContractTypeRef,
    ServiceContract, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use crate::{ArtifactIdentityError, Result};

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

    validate_closed_schema(contract)?;
    validate_diagnostic_keys(contract)
}

fn validate_closed_schema(contract: &ServiceContract) -> Result<()> {
    let mut referenced = BTreeSet::new();
    for descriptor in contract.operations.values() {
        collect_operation_contract_refs(&descriptor.contract, &mut referenced);
    }
    for schema_type in contract.boundary_schema.values() {
        collect_descriptor_refs(&schema_type.shape.descriptor, &mut referenced);
    }
    for type_id in referenced {
        if !contract.boundary_schema.contains_key(&type_id) {
            return invalid_contract(format!(
                "boundary schema is not closed: referenced ContractTypeId {type_id} is absent"
            ));
        }
    }
    Ok(())
}

fn collect_operation_contract_refs(
    operation: &BoundaryOperationContract,
    referenced: &mut BTreeSet<ContractTypeId>,
) {
    for parameter in &operation.parameters {
        collect_type_ref(&parameter.ty, referenced);
    }
    collect_type_ref(&operation.return_value.ty, referenced);
    if let BoundaryErrorContract::Typed { payload_type, .. } = &operation.errors {
        collect_type_ref(payload_type, referenced);
    }
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        collect_type_ref(item_type, referenced);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_type_ids, ..
    } = &operation.callbacks
    {
        referenced.extend(interface_type_ids.iter().cloned());
    }
}

fn collect_descriptor_refs(
    descriptor: &ContractTypeDescriptor,
    referenced: &mut BTreeSet<ContractTypeId>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for ty in fields.values() {
                collect_type_ref(ty, referenced);
            }
        }
        ContractTypeDescriptor::Union { variants } => {
            for ty in variants {
                collect_type_ref(ty, referenced);
            }
        }
        ContractTypeDescriptor::Alias { target } => collect_type_ref(target, referenced),
        ContractTypeDescriptor::Enumeration { .. } => {}
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                for parameter in &operation.parameters {
                    collect_type_ref(parameter, referenced);
                }
                collect_type_ref(&operation.return_type, referenced);
            }
        }
    }
}

fn collect_type_ref(ty: &ContractTypeRef, referenced: &mut BTreeSet<ContractTypeId>) {
    match ty {
        ContractTypeRef::Builtin { arguments, .. } => {
            for argument in arguments {
                collect_type_ref(argument, referenced);
            }
        }
        ContractTypeRef::Contract { contract_type_id } => {
            referenced.insert(contract_type_id.clone());
        }
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                collect_type_ref(field, referenced);
            }
        }
        ContractTypeRef::Union { variants } => {
            for variant in variants {
                collect_type_ref(variant, referenced);
            }
        }
        ContractTypeRef::Nullable { inner } => collect_type_ref(inner, referenced),
    }
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

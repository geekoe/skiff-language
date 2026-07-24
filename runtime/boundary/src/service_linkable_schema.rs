use std::collections::{BTreeMap, HashSet};

use skiff_artifact_model::{
    ContractSchemaType, ContractTypeDescriptor, ContractTypeId, ContractTypeRef,
};

use crate::service_linkable::ServiceLinkableMaterializationError;

pub(crate) fn validate_schema_closure(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
) -> Result<(), ServiceLinkableMaterializationError> {
    validate_schema_closure_inner(ty, schema, &mut HashSet::new())
}

fn validate_schema_closure_inner(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<(), ServiceLinkableMaterializationError> {
    match ty {
        ContractTypeRef::Builtin { arguments, .. } => {
            for argument in arguments {
                validate_schema_closure_inner(argument, schema, active)?;
            }
        }
        ContractTypeRef::Contract { contract_type_id } => {
            let descriptor = schema.get(contract_type_id).ok_or_else(|| {
                ServiceLinkableMaterializationError::MissingSchema {
                    contract_type_id: contract_type_id.clone(),
                }
            })?;
            if descriptor.contract_type_id != *contract_type_id {
                return Err(
                    ServiceLinkableMaterializationError::SchemaIdentityMismatch {
                        requested: contract_type_id.clone(),
                        actual: descriptor.contract_type_id.clone(),
                    },
                );
            }
            if !active.insert(contract_type_id.clone()) {
                return Err(ServiceLinkableMaterializationError::CyclicSchema {
                    contract_type_id: contract_type_id.clone(),
                });
            }
            validate_descriptor_closure(&descriptor.shape.descriptor, schema, active)?;
            active.remove(contract_type_id);
        }
        ContractTypeRef::PackagePublic { local_type_id } => {
            return Err(ServiceLinkableMaterializationError::InvalidContractPlan {
                message: format!(
                    "unresolved package public type {local_type_id} reached runtime boundary"
                ),
            });
        }
        ContractTypeRef::TypeParam { .. } => {}
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                validate_schema_closure_inner(field, schema, active)?;
            }
        }
        ContractTypeRef::StructuralUnion { variants } => {
            for variant in variants {
                validate_schema_closure_inner(variant, schema, active)?;
            }
        }
        ContractTypeRef::Nullable { inner } => {
            validate_schema_closure_inner(inner, schema, active)?;
        }
        ContractTypeRef::Literal { .. } => {}
    }
    Ok(())
}

fn validate_descriptor_closure(
    descriptor: &ContractTypeDescriptor,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<(), ServiceLinkableMaterializationError> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for field in fields.values() {
                validate_schema_closure_inner(field, schema, active)?;
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for variant in variants {
                validate_schema_closure_inner(variant, schema, active)?;
            }
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => {
            for branch in branches {
                validate_schema_closure_inner(&branch.branch_type, schema, active)?;
            }
        }
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => {
            validate_schema_closure_inner(target, schema, active)?;
        }
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                for parameter in &operation.parameters {
                    validate_schema_closure_inner(parameter, schema, active)?;
                }
                validate_schema_closure_inner(&operation.return_type, schema, active)?;
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
    Ok(())
}

pub(crate) fn contract_type_is_callback_interface(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    contract_type_is_callback_interface_inner(ty, schema, &mut HashSet::new())
}

fn contract_type_is_callback_interface_inner(
    ty: &ContractTypeRef,
    schema: &BTreeMap<ContractTypeId, ContractSchemaType>,
    active: &mut HashSet<ContractTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let ContractTypeRef::Contract { contract_type_id } = ty else {
        return Ok(false);
    };
    if !active.insert(contract_type_id.clone()) {
        return Err(ServiceLinkableMaterializationError::CyclicSchema {
            contract_type_id: contract_type_id.clone(),
        });
    }
    let schema_type = schema.get(contract_type_id).ok_or_else(|| {
        ServiceLinkableMaterializationError::MissingSchema {
            contract_type_id: contract_type_id.clone(),
        }
    })?;
    let result = match &schema_type.shape.descriptor {
        ContractTypeDescriptor::CallbackInterface { .. } => true,
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            contract_type_is_callback_interface_inner(target, schema, active)?
        }
        _ => false,
    };
    active.remove(contract_type_id);
    Ok(result)
}

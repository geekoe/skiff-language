use std::collections::HashSet;

use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeId, PackageSchemaTypeRecord,
};

use crate::{
    package_schema_records::PackageSchemaRecords,
    service_linkable::ServiceLinkableMaterializationError,
};

type PackageSchema = PackageSchemaRecords;

pub(crate) fn validate_schema_closure(
    ty: &ContractTypeRef,
    schema: &PackageSchema,
) -> Result<(), ServiceLinkableMaterializationError> {
    validate_schema_closure_inner(ty, schema, &mut HashSet::new())
}

fn validate_schema_closure_inner(
    ty: &ContractTypeRef,
    schema: &PackageSchema,
    active: &mut HashSet<PackageSchemaTypeId>,
) -> Result<(), ServiceLinkableMaterializationError> {
    match ty {
        ContractTypeRef::Builtin { arguments, .. } => {
            for argument in arguments {
                validate_schema_closure_inner(argument, schema, active)?;
            }
        }
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let record = resolve_record(
                package_id,
                stable_schema_key,
                package_schema_type_id,
                schema,
            )?;
            if !active.insert(package_schema_type_id.clone()) {
                return Err(ServiceLinkableMaterializationError::CyclicSchema {
                    package_schema_type_id: package_schema_type_id.clone(),
                });
            }
            validate_descriptor_closure(&record.canonical_descriptor.descriptor, schema, active)?;
            active.remove(package_schema_type_id);
        }
        ContractTypeRef::TypeParam { name } => {
            return Err(ServiceLinkableMaterializationError::InvalidContractPlan {
                message: format!("unresolved contract type parameter {name}"),
            });
        }
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
    schema: &PackageSchema,
    active: &mut HashSet<PackageSchemaTypeId>,
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
    schema: &PackageSchema,
) -> Result<bool, ServiceLinkableMaterializationError> {
    contract_type_is_callback_interface_inner(ty, schema, &mut HashSet::new())
}

fn contract_type_is_callback_interface_inner(
    ty: &ContractTypeRef,
    schema: &PackageSchema,
    active: &mut HashSet<PackageSchemaTypeId>,
) -> Result<bool, ServiceLinkableMaterializationError> {
    let ContractTypeRef::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = ty
    else {
        return Ok(false);
    };
    if !active.insert(package_schema_type_id.clone()) {
        return Err(ServiceLinkableMaterializationError::CyclicSchema {
            package_schema_type_id: package_schema_type_id.clone(),
        });
    }
    let record = resolve_record(
        package_id,
        stable_schema_key,
        package_schema_type_id,
        schema,
    )?;
    let result = match &record.canonical_descriptor.descriptor {
        ContractTypeDescriptor::CallbackInterface { .. } => true,
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            contract_type_is_callback_interface_inner(target, schema, active)?
        }
        _ => false,
    };
    active.remove(package_schema_type_id);
    Ok(result)
}

pub(crate) fn resolve_record<'a>(
    package_id: &str,
    stable_schema_key: &str,
    package_schema_type_id: &PackageSchemaTypeId,
    schema: &'a PackageSchema,
) -> Result<&'a PackageSchemaTypeRecord, ServiceLinkableMaterializationError> {
    let record = schema.get(package_schema_type_id).ok_or_else(|| {
        ServiceLinkableMaterializationError::MissingSchema {
            package_schema_type_id: package_schema_type_id.clone(),
        }
    })?;
    if record.package_schema_type_id != *package_schema_type_id {
        return Err(
            ServiceLinkableMaterializationError::SchemaIdentityMismatch {
                requested: package_schema_type_id.clone(),
                actual: record.package_schema_type_id.clone(),
            },
        );
    }
    if record.package_id != package_id || record.stable_schema_key != stable_schema_key {
        return Err(
            ServiceLinkableMaterializationError::SchemaOwnerOrKeyMismatch {
                package_schema_type_id: package_schema_type_id.clone(),
                expected_package_id: package_id.to_string(),
                expected_stable_schema_key: stable_schema_key.to_string(),
                actual_package_id: record.package_id.clone(),
                actual_stable_schema_key: record.stable_schema_key.clone(),
            },
        );
    }
    Ok(record.as_ref())
}

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    BoundaryErrorContract, BoundaryOperationContract, BoundaryStreamContract,
    ContractTypeDescriptor, ContractTypeId, ContractTypeRef, ServiceContract,
};

use crate::{ArtifactIdentityError, Result};

use super::{
    normalization::{
        normalize_contract_operation_contract, normalize_contract_type_shape, string_literal_value,
    },
    schema_graph::{reject_recursive_schema, SchemaEdge},
};

pub(super) fn validate_contract_schema(contract: &ServiceContract) -> Result<()> {
    validate_canonical_shapes(contract)?;
    validate_operation_types(contract)?;

    let mut edges = BTreeMap::new();
    for (type_id, schema_type) in &contract.boundary_schema {
        let path = format!("boundarySchema[{}]", schema_type.stable_key);
        let type_params = validate_type_param_declarations(&schema_type.shape.type_params, &path)?;
        let mut type_edges = Vec::new();
        validate_descriptor(
            contract,
            &schema_type.shape.descriptor,
            &path,
            &type_params,
            &mut type_edges,
        )?;
        edges.insert(type_id.clone(), type_edges);
    }
    reject_recursive_schema(contract, &edges)
}

fn validate_canonical_shapes(contract: &ServiceContract) -> Result<()> {
    for descriptor in contract.operations.values() {
        let path = format!("operations[{}].contract", descriptor.stable_key);
        let normalized = normalize_contract_operation_contract(descriptor.contract.clone(), &path)?;
        if normalized != descriptor.contract {
            return invalid_contract(format!(
                "{path}: operation type surface is not in canonical contract form"
            ));
        }
    }
    for schema_type in contract.boundary_schema.values() {
        let path = format!("boundarySchema[{}].shape", schema_type.stable_key);
        let normalized = normalize_contract_type_shape(schema_type.shape.clone(), &path)?;
        if normalized != schema_type.shape {
            return invalid_contract(format!(
                "{path}: schema type is not in canonical contract form"
            ));
        }
    }
    Ok(())
}

fn validate_operation_types(contract: &ServiceContract) -> Result<()> {
    for descriptor in contract.operations.values() {
        let path = format!("operations[{}].contract", descriptor.stable_key);
        validate_operation_contract(contract, &descriptor.contract, &path)?;
    }
    Ok(())
}

fn validate_operation_contract(
    contract: &ServiceContract,
    operation: &BoundaryOperationContract,
    path: &str,
) -> Result<()> {
    let mut ignored_edges = Vec::new();
    let type_params = BTreeSet::new();
    for (index, parameter) in operation.parameters.iter().enumerate() {
        validate_type_ref(
            contract,
            &parameter.ty,
            &format!("{path}.parameters[{index}].ty"),
            false,
            &type_params,
            &mut ignored_edges,
        )?;
    }
    validate_type_ref(
        contract,
        &operation.return_value.ty,
        &format!("{path}.returnValue.ty"),
        false,
        &type_params,
        &mut ignored_edges,
    )?;
    if let BoundaryErrorContract::Typed { payload_type, .. } = &operation.errors {
        validate_type_ref(
            contract,
            payload_type,
            &format!("{path}.errors.payloadType"),
            false,
            &type_params,
            &mut ignored_edges,
        )?;
    }
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        validate_type_ref(
            contract,
            item_type,
            &format!("{path}.stream.itemType"),
            false,
            &type_params,
            &mut ignored_edges,
        )?;
    }
    if let skiff_artifact_model::BoundaryCallbackContract::RequestScoped {
        interface_type_ids,
        ..
    } = &operation.callbacks
    {
        for (index, type_id) in interface_type_ids.iter().enumerate() {
            let callback_path = format!("{path}.callbacks.interfaceTypeIds[{index}]");
            let Some(schema_type) = contract.boundary_schema.get(type_id) else {
                return dangling_ref(&callback_path, type_id);
            };
            if !matches!(
                schema_type.shape.descriptor,
                ContractTypeDescriptor::CallbackInterface { .. }
            ) {
                return invalid_contract(format!(
                    "{callback_path}: ContractTypeId {type_id} does not identify a callback interface"
                ));
            }
        }
    }
    Ok(())
}

fn validate_descriptor(
    contract: &ServiceContract,
    descriptor: &ContractTypeDescriptor,
    path: &str,
    type_params: &BTreeSet<String>,
    edges: &mut Vec<SchemaEdge>,
) -> Result<()> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            for (name, field) in fields {
                validate_type_ref(
                    contract,
                    field,
                    &format!("{path}.descriptor.fields[{name}]"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            for (index, variant) in variants.iter().enumerate() {
                validate_type_ref(
                    contract,
                    variant,
                    &format!("{path}.descriptor.variants[{index}]"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
        ContractTypeDescriptor::DiscriminatedUnion {
            discriminator_field,
            branches,
        } => {
            if discriminator_field.is_empty() {
                return invalid_contract(format!(
                    "{path}.descriptor.discriminatorField: discriminator field must not be empty"
                ));
            }
            for branch in branches {
                let branch_path = format!("{path}.descriptor.branches[{}]", branch.tag);
                validate_type_ref(
                    contract,
                    &branch.branch_type,
                    &branch_path,
                    true,
                    type_params,
                    edges,
                )?;
                let fields = branch_record_fields(contract, &branch.branch_type).ok_or_else(|| {
                    ArtifactIdentityError::InvalidServiceContract {
                        message: format!(
                            "{branch_path}: discriminated union branch must be an inline or nominal record"
                        ),
                    }
                })?;
                let discriminator_path = format!("{branch_path}.fields[{discriminator_field}]");
                let Some(discriminator_type) = fields.get(discriminator_field) else {
                    return invalid_contract(format!(
                        "{discriminator_path}: discriminator field is missing"
                    ));
                };
                if string_literal_value(discriminator_type) != Some(branch.tag.as_str()) {
                    return invalid_contract(format!(
                        "{discriminator_path}: expected string literal tag `{}`",
                        branch.tag
                    ));
                }
            }
        }
        ContractTypeDescriptor::Alias { .. } => {
            return invalid_contract(format!(
                "{path}.descriptor: transparent alias must be expanded before a ServiceContract is materialized"
            ));
        }
        ContractTypeDescriptor::Representation { target } => {
            validate_type_ref(
                contract,
                target,
                &format!("{path}.descriptor.target"),
                false,
                type_params,
                edges,
            )?;
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for (name, operation) in operations {
                for (index, parameter) in operation.parameters.iter().enumerate() {
                    validate_type_ref(
                        contract,
                        parameter,
                        &format!("{path}.descriptor.operations[{name}].parameters[{index}]"),
                        false,
                        type_params,
                        edges,
                    )?;
                }
                validate_type_ref(
                    contract,
                    &operation.return_type,
                    &format!("{path}.descriptor.operations[{name}].returnType"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
    }
    Ok(())
}

fn validate_type_ref(
    contract: &ServiceContract,
    ty: &ContractTypeRef,
    path: &str,
    allow_anonymous_record: bool,
    type_params: &BTreeSet<String>,
    edges: &mut Vec<SchemaEdge>,
) -> Result<()> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => {
            if name == "Map" {
                validate_map_key(contract, &arguments[0], &format!("{path}.arguments[0]"))?;
            }
            for (index, argument) in arguments.iter().enumerate() {
                validate_type_ref(
                    contract,
                    argument,
                    &format!("{path}.arguments[{index}]"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
        ContractTypeRef::Contract { contract_type_id } => {
            let Some(schema_type) = contract.boundary_schema.get(contract_type_id) else {
                return dangling_ref(path, contract_type_id);
            };
            if matches!(
                schema_type.shape.descriptor,
                ContractTypeDescriptor::Alias { .. }
            ) {
                return invalid_contract(format!(
                    "{path}: transparent alias {} must be expanded before a ServiceContract is materialized",
                    schema_type.stable_key
                ));
            }
            edges.push(SchemaEdge {
                target: contract_type_id.clone(),
                path: path.to_string(),
            });
        }
        ContractTypeRef::PackagePublic { local_type_id } => {
            return invalid_contract(format!(
                "{path}: unresolved package public type `{local_type_id}` is not valid in a ServiceContract"
            ));
        }
        ContractTypeRef::TypeParam { name } => {
            if !type_params.contains(name) {
                return invalid_contract(format!("{path}: unknown type parameter `{name}`"));
            }
        }
        ContractTypeRef::Record { fields } => {
            if !allow_anonymous_record {
                return invalid_contract(format!(
                    "{path}: anonymous record is only valid as a direct discriminated-union branch"
                ));
            }
            for (name, field) in fields {
                validate_type_ref(
                    contract,
                    field,
                    &format!("{path}.fields[{name}]"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
        ContractTypeRef::StructuralUnion { variants } => {
            for (index, variant) in variants.iter().enumerate() {
                validate_type_ref(
                    contract,
                    variant,
                    &format!("{path}.variants[{index}]"),
                    false,
                    type_params,
                    edges,
                )?;
            }
        }
        ContractTypeRef::Nullable { inner } => {
            validate_type_ref(
                contract,
                inner,
                &format!("{path}.inner"),
                false,
                type_params,
                edges,
            )?;
        }
        ContractTypeRef::Literal { .. } => {}
    }
    Ok(())
}

fn validate_type_param_declarations(
    declarations: &[String],
    path: &str,
) -> Result<BTreeSet<String>> {
    let mut declared = BTreeSet::new();
    for (index, name) in declarations.iter().enumerate() {
        let valid = name
            .chars()
            .next()
            .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
            && name
                .chars()
                .skip(1)
                .all(|character| character == '_' || character.is_ascii_alphanumeric());
        if !valid {
            return invalid_contract(format!(
                "{path}.typeParams[{index}]: invalid type parameter `{name}`"
            ));
        }
        if !declared.insert(name.clone()) {
            return invalid_contract(format!(
                "{path}.typeParams[{index}]: duplicate type parameter `{name}`"
            ));
        }
    }
    Ok(declared)
}

fn validate_map_key(contract: &ServiceContract, key: &ContractTypeRef, path: &str) -> Result<()> {
    if matches!(
        key,
        ContractTypeRef::Builtin { name, arguments }
            if name == "string" && arguments.is_empty()
    ) {
        return Ok(());
    }
    let ContractTypeRef::Contract { contract_type_id } = key else {
        return invalid_contract(format!(
            "{path}: Map key must be exact string or one nominal representation over string"
        ));
    };
    let Some(schema_type) = contract.boundary_schema.get(contract_type_id) else {
        return dangling_ref(path, contract_type_id);
    };
    let target = match &schema_type.shape.descriptor {
        ContractTypeDescriptor::Representation { target } => target,
        ContractTypeDescriptor::Alias { .. } => {
            return invalid_contract(format!(
                "{path}: transparent alias {} must be expanded before Map key validation",
                schema_type.stable_key
            ));
        }
        _ => {
            return invalid_contract(format!(
                "{path}: Map key ContractTypeId {contract_type_id} must identify a representation"
            ));
        }
    };
    if !matches!(
        target,
        ContractTypeRef::Builtin { name, arguments }
            if name == "string" && arguments.is_empty()
    ) {
        return invalid_contract(format!(
            "{path}: Map key representation {} must target exact string",
            schema_type.stable_key
        ));
    }
    Ok(())
}

fn branch_record_fields<'a>(
    contract: &'a ServiceContract,
    branch_type: &'a ContractTypeRef,
) -> Option<&'a BTreeMap<String, ContractTypeRef>> {
    match branch_type {
        ContractTypeRef::Record { fields } => Some(fields),
        ContractTypeRef::Contract { contract_type_id } => {
            match &contract
                .boundary_schema
                .get(contract_type_id)?
                .shape
                .descriptor
            {
                ContractTypeDescriptor::Record { fields } => Some(fields),
                _ => None,
            }
        }
        _ => None,
    }
}

fn dangling_ref<T>(path: &str, type_id: &ContractTypeId) -> Result<T> {
    invalid_contract(format!(
        "{path}: boundary schema is not closed; referenced ContractTypeId {type_id} is absent"
    ))
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}

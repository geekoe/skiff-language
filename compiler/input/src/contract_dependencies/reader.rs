use std::{collections::BTreeMap, fs, path::Path};

use skiff_artifact_identity::{
    validate_package_schema_records, validate_service_contract_identities,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryOperationDescriptor, BoundaryStreamContract,
    ContractRequirement, ContractTypeDescriptor, ContractTypeRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord, ServiceContract, TypeRefIr,
};
use skiff_compiler_input_model::{is_reserved_source_import_alias, is_valid_source_import_alias};
use skiff_compiler_projection_input::ResolvedPackageSchema;

use super::{strict_json::StrictJsonValue, ContractDependencyError};

/// A ServiceContract that has crossed the compiler input trust boundary.
/// Construction is private to the canonical validation routines below.
#[derive(Debug, Clone)]
pub struct ResolvedContractDependency {
    requirement: ContractRequirement,
    contract: ServiceContract,
    schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

impl ResolvedContractDependency {
    pub fn validated(
        requirement: ContractRequirement,
        contract: ServiceContract,
        package_schemas: &[ResolvedPackageSchema],
    ) -> Result<Self, ContractDependencyError> {
        validate_alias(&requirement.alias)?;
        validate_service_contract_identities(&contract).map_err(|source| {
            ContractDependencyError::InvalidContract {
                alias: requirement.alias.clone(),
                source: Box::new(source),
            }
        })?;
        if contract.service_id != requirement.service_id
            || contract.contract_version != requirement.contract_version
        {
            return Err(ContractDependencyError::CoordinateMismatch {
                alias: requirement.alias,
                expected_service_id: requirement.service_id,
                expected_version: requirement.contract_version,
                actual_service_id: contract.service_id,
                actual_version: contract.contract_version,
            });
        }
        if contract.service_protocol_identity != requirement.expected_protocol_identity {
            return Err(ContractDependencyError::ProtocolIdentityMismatch {
                alias: requirement.alias,
                expected: requirement.expected_protocol_identity.to_string(),
                actual: contract.service_protocol_identity.to_string(),
            });
        }
        let schema_records =
            validated_schema_records(&requirement.alias, &contract, package_schemas)?;
        Ok(Self {
            requirement,
            contract,
            schema_records,
        })
    }

    pub fn requirement(&self) -> &ContractRequirement {
        &self.requirement
    }

    pub fn contract(&self) -> &ServiceContract {
        &self.contract
    }

    pub fn schema_records(&self) -> &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord> {
        &self.schema_records
    }
}

/// Reads and validates a published ServiceContract. The reader recomputes and
/// validates canonical identities; it never overwrites an untrusted declared
/// identity with an assigned value.
pub fn read_contract_dependency(
    path: &Path,
    requirement: ContractRequirement,
    package_schemas: &[ResolvedPackageSchema],
) -> Result<ResolvedContractDependency, ContractDependencyError> {
    let bytes = fs::read(path).map_err(|source| ContractDependencyError::Read {
        path: path.display().to_string(),
        source,
    })?;
    read_contract_dependency_json(
        path.display().to_string(),
        &bytes,
        requirement,
        package_schemas,
    )
}

pub fn read_contract_dependency_json(
    label: impl Into<String>,
    bytes: &[u8],
    requirement: ContractRequirement,
    package_schemas: &[ResolvedPackageSchema],
) -> Result<ResolvedContractDependency, ContractDependencyError> {
    let label = label.into();
    let value = serde_json::from_slice::<StrictJsonValue>(bytes)
        .map_err(|source| ContractDependencyError::Parse {
            label: label.clone(),
            source,
        })?
        .into_inner();
    let contract = serde_json::from_value::<ServiceContract>(value).map_err(|source| {
        ContractDependencyError::Parse {
            label: label.clone(),
            source,
        }
    })?;
    ResolvedContractDependency::validated(requirement, contract, package_schemas)
}

fn validated_schema_records(
    alias: &str,
    contract: &ServiceContract,
    package_schemas: &[ResolvedPackageSchema],
) -> Result<BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>, ContractDependencyError> {
    let schemas_by_owner = package_schemas
        .iter()
        .map(|schema| (schema.package_id(), schema))
        .collect::<BTreeMap<_, _>>();
    let mut records = BTreeMap::new();
    for requirement in &contract.package_type_requirements {
        let schema = schemas_by_owner
            .get(requirement.package_id.as_str())
            .ok_or_else(|| ContractDependencyError::MissingPackageSchema {
                alias: alias.to_string(),
                package_id: requirement.package_id.clone(),
            })?;
        for type_id in &requirement.required_type_ids {
            let record = schema.records().get(type_id).ok_or_else(|| {
                ContractDependencyError::MissingSchemaRecord {
                    alias: alias.to_string(),
                    package_id: requirement.package_id.clone(),
                    package_schema_type_id: type_id.clone(),
                }
            })?;
            let Some((indexed_id, indexed_record)) = schema.public_type(&record.stable_schema_key)
            else {
                return Err(ContractDependencyError::ContractTypeNotPublicNameable {
                    alias: alias.to_string(),
                    stable_key: record.stable_schema_key.clone(),
                    package_schema_type_id: type_id.clone(),
                });
            };
            if indexed_id != type_id || indexed_record != record {
                return Err(ContractDependencyError::ContractTypeNotPublicNameable {
                    alias: alias.to_string(),
                    stable_key: record.stable_schema_key.clone(),
                    package_schema_type_id: type_id.clone(),
                });
            }
            if records.insert(type_id.clone(), record.clone()).is_some() {
                return Err(ContractDependencyError::DuplicateSchemaTypeId {
                    alias: alias.to_string(),
                    package_schema_type_id: type_id.clone(),
                });
            }
        }
    }

    validate_package_schema_records(&records).map_err(|source| {
        ContractDependencyError::InvalidSchemaRecords {
            alias: alias.to_string(),
            source: Box::new(source),
        }
    })?;

    let expected = contract
        .package_type_requirements
        .iter()
        .flat_map(|requirement| {
            requirement
                .required_type_ids
                .iter()
                .map(move |type_id| (type_id.clone(), requirement.package_id.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    if records.len() != expected.len() {
        return Err(ContractDependencyError::SchemaRecordSetMismatch {
            alias: alias.to_string(),
            expected: expected.len(),
            actual: records.len(),
        });
    }
    for (type_id, package_id) in &expected {
        let record =
            records
                .get(type_id)
                .ok_or_else(|| ContractDependencyError::MissingSchemaRecord {
                    alias: alias.to_string(),
                    package_id: (*package_id).to_string(),
                    package_schema_type_id: type_id.clone(),
                })?;
        if record.package_id != *package_id {
            return Err(ContractDependencyError::SchemaRecordOwnerMismatch {
                alias: alias.to_string(),
                package_schema_type_id: type_id.clone(),
                expected_package_id: (*package_id).to_string(),
                actual_package_id: record.package_id.clone(),
            });
        }
    }

    let mut reachable = std::collections::BTreeSet::new();
    for operation in contract.operations.values() {
        validate_operation_refs(alias, operation, &records)?;
        let mut roots = std::collections::BTreeSet::new();
        collect_operation_type_ids(operation, &mut roots);
        for root in roots {
            visit_reachable_type(&root, &records, &mut reachable);
        }
    }
    for public_instance in contract.public_instances.values() {
        for interface in &public_instance.interfaces {
            if let Some(stable_key) = public_instance_interface_stable_key(interface) {
                if let Some(record) = records
                    .values()
                    .find(|record| record.stable_schema_key == stable_key)
                {
                    visit_reachable_type(&record.package_schema_type_id, &records, &mut reachable);
                }
            }
        }
    }
    if reachable != expected.keys().cloned().collect() {
        return Err(ContractDependencyError::SchemaReachabilityMismatch {
            alias: alias.to_string(),
        });
    }
    Ok(records)
}

fn public_instance_interface_stable_key(
    interface: &skiff_artifact_model::ContractPublicInstanceInterface,
) -> Option<String> {
    let declaration =
        serde_json::from_str::<TypeRefIr>(&interface.interface.interface_abi_id).ok()?;
    match declaration {
        TypeRefIr::ServiceSymbol { symbol } => Some(symbol.symbol),
        TypeRefIr::PackageSymbol { symbol } => symbol
            .symbol_path
            .rsplit_once('.')
            .map(|(_, symbol)| symbol.to_string()),
        _ => None,
    }
}

fn visit_reachable_type(
    type_id: &PackageSchemaTypeId,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    reachable: &mut std::collections::BTreeSet<PackageSchemaTypeId>,
) {
    if !reachable.insert(type_id.clone()) {
        return;
    }
    let Some(record) = records.get(type_id) else {
        return;
    };
    let mut children = std::collections::BTreeSet::new();
    collect_descriptor_type_ids(&record.canonical_descriptor.descriptor, &mut children);
    for child in children {
        visit_reachable_type(&child, records, reachable);
    }
}

fn collect_operation_type_ids(
    operation: &BoundaryOperationDescriptor,
    out: &mut std::collections::BTreeSet<PackageSchemaTypeId>,
) {
    for parameter in &operation.contract.parameters {
        collect_type_ids(&parameter.ty, out);
    }
    collect_type_ids(&operation.contract.return_value.ty, out);
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.contract.stream {
        collect_type_ids(item_type, out);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_types, ..
    } = &operation.contract.callbacks
    {
        out.extend(
            interface_types
                .iter()
                .map(|reference| reference.package_schema_type_id.clone()),
        );
    }
}

fn collect_descriptor_type_ids(
    descriptor: &ContractTypeDescriptor,
    out: &mut std::collections::BTreeSet<PackageSchemaTypeId>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields.values().for_each(|ty| collect_type_ids(ty, out));
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants.iter().for_each(|ty| collect_type_ids(ty, out));
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|branch| collect_type_ids(&branch.branch_type, out)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => collect_type_ids(target, out),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                operation
                    .parameters
                    .iter()
                    .for_each(|ty| collect_type_ids(ty, out));
                collect_type_ids(&operation.return_type, out);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_ids(
    ty: &ContractTypeRef,
    out: &mut std::collections::BTreeSet<PackageSchemaTypeId>,
) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_schema_type_id,
            ..
        } => {
            out.insert(package_schema_type_id.clone());
        }
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments.iter().for_each(|ty| collect_type_ids(ty, out)),
        ContractTypeRef::Record { fields } => {
            fields.values().for_each(|ty| collect_type_ids(ty, out));
        }
        ContractTypeRef::Nullable { inner } => collect_type_ids(inner, out),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_ids(interface, out);
            arguments.iter().for_each(|ty| collect_type_ids(ty, out));
        }
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}

fn validate_operation_refs(
    alias: &str,
    operation: &BoundaryOperationDescriptor,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<(), ContractDependencyError> {
    for parameter in &operation.contract.parameters {
        validate_type_refs(alias, &parameter.ty, records)?;
    }
    validate_type_refs(alias, &operation.contract.return_value.ty, records)?;
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.contract.stream {
        validate_type_refs(alias, item_type, records)?;
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_types, ..
    } = &operation.contract.callbacks
    {
        for interface_type in interface_types {
            let record = records
                .get(&interface_type.package_schema_type_id)
                .ok_or_else(|| ContractDependencyError::MissingSchemaRecord {
                    alias: alias.to_string(),
                    package_id: interface_type.package_id.clone(),
                    package_schema_type_id: interface_type.package_schema_type_id.clone(),
                })?;
            if record.package_id != interface_type.package_id
                || record.stable_schema_key != interface_type.stable_schema_key
            {
                return Err(ContractDependencyError::SchemaReferenceMismatch {
                    alias: alias.to_string(),
                    package_schema_type_id: interface_type.package_schema_type_id.clone(),
                });
            }
        }
    }
    Ok(())
}

fn validate_type_refs(
    alias: &str,
    ty: &ContractTypeRef,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<(), ContractDependencyError> {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => {
            let record = records.get(package_schema_type_id).ok_or_else(|| {
                ContractDependencyError::MissingSchemaRecord {
                    alias: alias.to_string(),
                    package_id: package_id.clone(),
                    package_schema_type_id: package_schema_type_id.clone(),
                }
            })?;
            if record.package_id != *package_id || record.stable_schema_key != *stable_schema_key {
                return Err(ContractDependencyError::SchemaReferenceMismatch {
                    alias: alias.to_string(),
                    package_schema_type_id: package_schema_type_id.clone(),
                });
            }
        }
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => {
            for argument in arguments {
                validate_type_refs(alias, argument, records)?;
            }
        }
        ContractTypeRef::Record { fields } => {
            for field in fields.values() {
                validate_type_refs(alias, field, records)?;
            }
        }
        ContractTypeRef::Nullable { inner } => validate_type_refs(alias, inner, records)?,
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            validate_type_refs(alias, interface, records)?;
            for argument in arguments {
                validate_type_refs(alias, argument, records)?;
            }
        }
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
    Ok(())
}

fn validate_alias(alias: &str) -> Result<(), ContractDependencyError> {
    if !is_valid_source_import_alias(alias) || is_reserved_source_import_alias(alias) {
        return Err(ContractDependencyError::InvalidAlias {
            alias: alias.to_string(),
        });
    }
    Ok(())
}

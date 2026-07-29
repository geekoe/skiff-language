use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryStreamContract, ContractOperationId, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape, PackageSchemaCanonicalDescriptor, PackageSchemaIndex,
    PackageSchemaIndexEntry, PackageSchemaIndexIdentity, PackageSchemaTypeId,
    PackageSchemaTypeRecord, PackageTypeRequirement, ServiceContract, ServiceContractRef,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, CONTRACT_OPERATION_IDENTITY_PREFIX,
    CONTRACT_OPERATION_IDENTITY_SCHEMA_MARKER, PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER, PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
    PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER, SERVICE_PROTOCOL_IDENTITY_PREFIX,
    SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
};

mod normalization;
#[cfg(test)]
mod suspension_tests;

pub use normalization::{normalize_contract_operation_contract, normalize_contract_type_shape};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageSchemaTypeIdentityInput<'a> {
    schema: &'static str,
    package_id: &'a str,
    stable_schema_key: &'a str,
    canonical_descriptor: &'a PackageSchemaCanonicalDescriptor,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageSchemaIndexIdentityInput<'a> {
    schema: &'static str,
    package_id: &'a str,
    types: &'a BTreeMap<String, PackageSchemaIndexEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractOperationIdentityInput<'a> {
    schema: &'static str,
    service_id: &'a str,
    stable_operation_key: &'a str,
}

/// The protocol preimage contains only operations and their exact reachable
/// package type requirements. Package indexes and descriptors are deliberately
/// absent, so unrelated package schema entries cannot perturb this identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceProtocolIdentityProjection {
    schema: &'static str,
    service_id: String,
    operations: BTreeMap<ContractOperationId, BoundaryOperationDescriptor>,
    package_type_requirements: Vec<PackageTypeRequirement>,
}

pub fn package_schema_type_id(
    package_id: &str,
    stable_schema_key: &str,
    canonical_descriptor: &PackageSchemaCanonicalDescriptor,
) -> Result<PackageSchemaTypeId> {
    validate_non_empty("packageId", package_id)?;
    validate_non_empty("stableSchemaKey", stable_schema_key)?;
    let normalized = normalize_schema_descriptor(canonical_descriptor.clone())?;
    if &normalized != canonical_descriptor {
        return invalid_contract("package schema canonicalDescriptor is not canonical");
    }
    let bytes = canonical_ir_bytes(
        &PackageSchemaTypeIdentityInput {
            schema: PACKAGE_SCHEMA_TYPE_IDENTITY_SCHEMA_MARKER,
            package_id,
            stable_schema_key,
            canonical_descriptor,
        },
        ArtifactIdentityError::SerializePackageSchemaTypeIdentity,
    )?;
    Ok(PackageSchemaTypeId::new(framed_identity(
        PACKAGE_SCHEMA_TYPE_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn package_schema_index_identity(
    package_id: &str,
    types: &BTreeMap<String, PackageSchemaIndexEntry>,
) -> Result<PackageSchemaIndexIdentity> {
    validate_non_empty("packageId", package_id)?;
    for stable_key in types.keys() {
        validate_non_empty("stableSchemaKey", stable_key)?;
    }
    let bytes = canonical_ir_bytes(
        &PackageSchemaIndexIdentityInput {
            schema: PACKAGE_SCHEMA_INDEX_IDENTITY_SCHEMA_MARKER,
            package_id,
            types,
        },
        ArtifactIdentityError::SerializePackageSchemaIndexIdentity,
    )?;
    Ok(PackageSchemaIndexIdentity::new(framed_identity(
        PACKAGE_SCHEMA_INDEX_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn validate_package_schema_index(index: &PackageSchemaIndex) -> Result<()> {
    for (stable_schema_key, entry) in &index.types {
        if entry.nameability != ContractTypeNameability::PublicNameable
            || entry.public_path.as_deref() != Some(stable_schema_key.as_str())
        {
            return invalid_contract(format!(
                "package schema index entry {stable_schema_key} must be an api.yml public named type"
            ));
        }
    }
    let expected = package_schema_index_identity(&index.package_id, &index.types)?;
    if index.package_schema_index_identity != expected {
        return invalid_contract(format!(
            "package schema index declared identity {}, expected {expected}",
            index.package_schema_index_identity
        ));
    }
    Ok(())
}

/// Validates identity, closure and the v1 acyclic graph rule for a resolved
/// collection of independently addressed package schema records.
pub fn validate_package_schema_records(
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<()> {
    let mut visiting = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for type_id in records.keys() {
        visit_schema_record(type_id, records, &mut visiting, &mut complete)?;
    }

    for (type_id, record) in records {
        if type_id != &record.package_schema_type_id {
            return invalid_contract(format!(
                "package schema record map key {type_id} does not match nested identity {}",
                record.package_schema_type_id
            ));
        }
        let expected = package_schema_type_id(
            &record.package_id,
            &record.stable_schema_key,
            &record.canonical_descriptor,
        )?;
        if type_id != &expected {
            return invalid_contract(format!(
                "package schema type {} has identity {type_id}, expected {expected}",
                record.stable_schema_key
            ));
        }
    }

    Ok(())
}

pub fn contract_operation_id(
    service_id: &str,
    _package_version_label: &str,
    stable_operation_key: &str,
) -> Result<ContractOperationId> {
    validate_non_empty("serviceId", service_id)?;
    validate_non_empty("stableOperationKey", stable_operation_key)?;
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
    validate_service_contract_surface(contract)?;
    Ok(ServiceProtocolIdentityProjection {
        schema: SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER,
        service_id: contract.service_id.clone(),
        operations: contract.operations.clone(),
        package_type_requirements: contract.package_type_requirements.clone(),
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
    identity
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
        })
}

pub fn assign_service_contract_identities(
    contract: &mut ServiceContract,
) -> Result<ServiceProtocolIdentity> {
    let identity = service_protocol_identity(contract)?;
    contract.service_protocol_identity = identity.clone();
    validate_service_contract_identities(contract)?;
    Ok(identity)
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

fn validate_service_contract_surface(contract: &ServiceContract) -> Result<()> {
    if contract.schema_version != SERVICE_CONTRACT_SCHEMA_VERSION {
        return invalid_contract(format!(
            "schemaVersion must be {SERVICE_CONTRACT_SCHEMA_VERSION}, got {}",
            contract.schema_version
        ));
    }
    validate_non_empty("serviceId", &contract.service_id)?;
    validate_non_empty("contractVersion", &contract.contract_version)?;
    if contract.operations.is_empty() && !contract.package_type_requirements.is_empty() {
        return invalid_contract(
            "zero-operation service contracts cannot contain packageTypeRequirements",
        );
    }
    for (operation_id, descriptor) in &contract.operations {
        if operation_id != &descriptor.operation_id {
            return invalid_contract("operation map key does not match nested operationId");
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
        validate_operation_existentials(&descriptor.contract)?;
        skiff_artifact_model::validate_boundary_operation_contract(&descriptor.contract).map_err(
            |error| ArtifactIdentityError::InvalidServiceContract {
                message: format!(
                    "operation {} has an invalid boundary contract: {error}",
                    descriptor.stable_key
                ),
            },
        )?;
    }
    let mut previous_package: Option<&str> = None;
    let mut required = BTreeSet::new();
    for requirement in &contract.package_type_requirements {
        validate_non_empty("packageId", &requirement.package_id)?;
        if previous_package.is_some_and(|previous| previous >= requirement.package_id.as_str()) {
            return invalid_contract("packageTypeRequirements must be sorted by unique packageId");
        }
        previous_package = Some(&requirement.package_id);
        if requirement.required_type_ids.is_empty()
            || !requirement
                .required_type_ids
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        {
            return invalid_contract(
                "requiredTypeIds must be non-empty, sorted, and contain no duplicates",
            );
        }
        for type_id in &requirement.required_type_ids {
            required.insert((requirement.package_id.as_str(), type_id));
        }
    }
    let mut referenced = Vec::new();
    for operation in contract.operations.values() {
        collect_operation_refs(operation, &mut referenced);
    }
    for reference in referenced {
        if !required.contains(&(reference.0, reference.1)) {
            return invalid_contract(format!(
                "operation references package schema type {} from {} outside packageTypeRequirements",
                reference.1, reference.0
            ));
        }
    }
    for type_id in contract.diagnostic_text.types.keys() {
        if !contract
            .package_type_requirements
            .iter()
            .any(|requirement| requirement.required_type_ids.contains(type_id))
        {
            return invalid_contract(format!(
                "diagnostic text references unknown package schema type {type_id}"
            ));
        }
    }
    Ok(())
}

fn validate_operation_existentials(operation: &BoundaryOperationContract) -> Result<()> {
    for parameter in &operation.parameters {
        validate_existential_ref(&parameter.ty)?;
    }
    validate_existential_ref(&operation.return_value.ty)?;
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.stream {
        validate_existential_ref(item_type)?;
    }
    Ok(())
}

fn validate_existential_ref(ty: &ContractTypeRef) -> Result<()> {
    match ty {
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            if !matches!(interface.as_ref(), ContractTypeRef::PackageSchema { .. }) {
                return invalid_contract(
                    "anyInterface target must be an exact PackageSchema interface nominal",
                );
            }
            validate_existential_ref(interface)?;
            arguments.iter().try_for_each(validate_existential_ref)
        }
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments.iter().try_for_each(validate_existential_ref),
        ContractTypeRef::Record { fields } => {
            fields.values().try_for_each(validate_existential_ref)
        }
        ContractTypeRef::Nullable { inner } => validate_existential_ref(inner),
        ContractTypeRef::PackageSchema { .. }
        | ContractTypeRef::TypeParam { .. }
        | ContractTypeRef::Literal { .. } => Ok(()),
    }
}

fn normalize_schema_descriptor(
    descriptor: PackageSchemaCanonicalDescriptor,
) -> Result<PackageSchemaCanonicalDescriptor> {
    let shape = normalize_contract_type_shape(
        ContractTypeShape {
            nameability: ContractTypeNameability::ClosureOnly,
            type_params: descriptor.type_params,
            descriptor: descriptor.descriptor,
        },
        "packageSchemaType",
    )?;
    Ok(PackageSchemaCanonicalDescriptor {
        type_params: shape.type_params,
        descriptor: shape.descriptor,
    })
}

fn visit_schema_record(
    type_id: &PackageSchemaTypeId,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    visiting: &mut BTreeSet<PackageSchemaTypeId>,
    complete: &mut BTreeSet<PackageSchemaTypeId>,
) -> Result<()> {
    if complete.contains(type_id) {
        return Ok(());
    }
    if !visiting.insert(type_id.clone()) {
        return invalid_contract(format!(
            "package schema v1 forbids recursive type cycle at {type_id}"
        ));
    }
    let record =
        records
            .get(type_id)
            .ok_or_else(|| ArtifactIdentityError::InvalidServiceContract {
                message: format!("missing package schema record {type_id}"),
            })?;
    let mut children = Vec::new();
    collect_descriptor_refs(&record.canonical_descriptor.descriptor, &mut children);
    for (package_id, stable_key, child_id) in children {
        let child =
            records
                .get(child_id)
                .ok_or_else(|| ArtifactIdentityError::InvalidServiceContract {
                    message: format!(
                        "package schema closure is missing {package_id}:{stable_key}:{child_id}"
                    ),
                })?;
        if child.package_id != package_id || child.stable_schema_key != stable_key {
            return invalid_contract(format!(
                "package schema child reference {child_id} owner or stable key mismatch"
            ));
        }
        visit_schema_record(child_id, records, visiting, complete)?;
    }
    visiting.remove(type_id);
    complete.insert(type_id.clone());
    Ok(())
}

fn collect_operation_refs<'a>(
    operation: &'a BoundaryOperationDescriptor,
    out: &mut Vec<(&'a str, &'a PackageSchemaTypeId)>,
) {
    for parameter in &operation.contract.parameters {
        collect_type_refs(&parameter.ty, out);
    }
    collect_type_refs(&operation.contract.return_value.ty, out);
    if let BoundaryStreamContract::ServerStream { item_type, .. } = &operation.contract.stream {
        collect_type_refs(item_type, out);
    }
    if let BoundaryCallbackContract::RequestScoped {
        interface_types, ..
    } = &operation.contract.callbacks
    {
        out.extend(interface_types.iter().map(|reference| {
            (
                reference.package_id.as_str(),
                &reference.package_schema_type_id,
            )
        }));
    }
}

fn collect_descriptor_refs<'a>(
    descriptor: &'a ContractTypeDescriptor,
    out: &mut Vec<(&'a str, &'a str, &'a PackageSchemaTypeId)>,
) {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => {
            fields
                .values()
                .for_each(|ty| collect_type_refs_with_keys(ty, out));
        }
        ContractTypeDescriptor::StructuralUnion { variants } => {
            variants
                .iter()
                .for_each(|ty| collect_type_refs_with_keys(ty, out));
        }
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => branches
            .iter()
            .for_each(|branch| collect_type_refs_with_keys(&branch.branch_type, out)),
        ContractTypeDescriptor::Representation { target }
        | ContractTypeDescriptor::Alias { target } => collect_type_refs_with_keys(target, out),
        ContractTypeDescriptor::CallbackInterface { operations } => {
            for operation in operations.values() {
                operation
                    .parameters
                    .iter()
                    .for_each(|ty| collect_type_refs_with_keys(ty, out));
                collect_type_refs_with_keys(&operation.return_type, out);
            }
        }
        ContractTypeDescriptor::Enumeration { .. } => {}
    }
}

fn collect_type_refs<'a>(
    ty: &'a ContractTypeRef,
    out: &mut Vec<(&'a str, &'a PackageSchemaTypeId)>,
) {
    let mut keyed = Vec::new();
    collect_type_refs_with_keys(ty, &mut keyed);
    out.extend(
        keyed
            .into_iter()
            .map(|(package_id, _, type_id)| (package_id, type_id)),
    );
}

fn collect_type_refs_with_keys<'a>(
    ty: &'a ContractTypeRef,
    out: &mut Vec<(&'a str, &'a str, &'a PackageSchemaTypeId)>,
) {
    match ty {
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => out.push((package_id, stable_schema_key, package_schema_type_id)),
        ContractTypeRef::Builtin { arguments, .. }
        | ContractTypeRef::StructuralUnion {
            variants: arguments,
        } => arguments
            .iter()
            .for_each(|child| collect_type_refs_with_keys(child, out)),
        ContractTypeRef::Record { fields } => fields
            .values()
            .for_each(|child| collect_type_refs_with_keys(child, out)),
        ContractTypeRef::AnyInterface {
            interface,
            arguments,
        } => {
            collect_type_refs_with_keys(interface, out);
            arguments
                .iter()
                .for_each(|child| collect_type_refs_with_keys(child, out));
        }
        ContractTypeRef::Nullable { inner } => collect_type_refs_with_keys(inner, out),
        ContractTypeRef::TypeParam { .. } | ContractTypeRef::Literal { .. } => {}
    }
}

fn validate_non_empty(label: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid_contract(format!("{label} must be a non-empty string"));
    }
    Ok(())
}

fn invalid_contract<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidServiceContract {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        BoundaryCallbackContract, BoundaryCallbackExpirationError, BoundaryCallbackLifetime,
        BoundaryEffectGuarantee, BoundaryFeatureUnavailableReason, BoundaryOperationContract,
        BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
        BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
        ContractDiagnosticText, ContractTypeDescriptor, PackageSchemaIndexEntry,
        PackageSchemaTypeRef, PackageTypeRequirement,
    };

    use super::*;

    #[test]
    fn package_type_identity_uses_owner_key_and_descriptor_not_release_coordinates() {
        let string_descriptor = descriptor("string");
        let first = package_schema_type_id("example.pkg", "User", &string_descriptor).unwrap();
        let across_version_and_build =
            package_schema_type_id("example.pkg", "User", &string_descriptor).unwrap();
        assert_eq!(first, across_version_and_build);
        assert_ne!(
            first,
            package_schema_type_id("other.pkg", "User", &string_descriptor).unwrap()
        );
        assert_ne!(
            first,
            package_schema_type_id("example.pkg", "Account", &string_descriptor).unwrap()
        );
        assert_ne!(
            first,
            package_schema_type_id("example.pkg", "User", &descriptor("integer")).unwrap()
        );
    }

    #[test]
    fn unrelated_index_entry_does_not_change_existing_service_protocol() {
        let user_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let mut contract = service_contract(user_id.clone());
        let first = assign_service_contract_identities(&mut contract).unwrap();

        let base_index = BTreeMap::from([("User".to_string(), index_entry(user_id.clone()))]);
        let mut expanded_index = base_index.clone();
        expanded_index.insert(
            "Unused".to_string(),
            index_entry(
                package_schema_type_id("example.pkg", "Unused", &descriptor("bool")).unwrap(),
            ),
        );
        assert_ne!(
            package_schema_index_identity("example.pkg", &base_index).unwrap(),
            package_schema_index_identity("example.pkg", &expanded_index).unwrap()
        );
        assert_eq!(first, service_protocol_identity(&contract).unwrap());
    }

    #[test]
    fn protocol_requires_sorted_exact_package_type_requirements() {
        let user_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let mut contract = service_contract(user_id.clone());
        contract.package_type_requirements[0]
            .required_type_ids
            .push(user_id);
        assert!(service_protocol_identity(&contract).is_err());
    }

    #[test]
    fn service_protocol_mutation_matrix_covers_open_operation_surface() {
        let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let base = service_contract(type_id.clone());
        let baseline = service_protocol_identity(&base).unwrap();
        assert_eq!(
            serde_json::to_value(service_protocol_identity_projection(&base).unwrap()).unwrap()
                ["schema"],
            SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER
        );
        let operation_id = base.operations.keys().next().unwrap().clone();

        let mut mutations = Vec::new();

        let mut parameter = base.clone();
        parameter
            .operations
            .get_mut(&operation_id)
            .unwrap()
            .contract
            .parameters[0]
            .ty = ContractTypeRef::builtin("string");
        mutations.push(parameter);

        let mut returned = base.clone();
        returned
            .operations
            .get_mut(&operation_id)
            .unwrap()
            .contract
            .return_value
            .ty = ContractTypeRef::builtin("string");
        mutations.push(returned);

        let mut streamed = base.clone();
        streamed
            .operations
            .get_mut(&operation_id)
            .unwrap()
            .contract
            .stream = BoundaryStreamContract::ServerStream {
            item_type: ContractTypeRef::builtin("string"),
            item_value_plan: value_plan(
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            ),
        };
        mutations.push(streamed);

        let mut callback = base.clone();
        callback
            .operations
            .get_mut(&operation_id)
            .unwrap()
            .contract
            .callbacks = BoundaryCallbackContract::RequestScoped {
            interface_types: vec![PackageSchemaTypeRef {
                package_id: "example.pkg".to_string(),
                stable_schema_key: "User".to_string(),
                package_schema_type_id: type_id,
            }],
            lifetime: BoundaryCallbackLifetime::TopLevelRequest,
            expiration_error: BoundaryCallbackExpirationError::CapabilityExpired,
        };
        mutations.push(callback);

        for changed in mutations {
            assert_ne!(service_protocol_identity(&changed).unwrap(), baseline);
            assert_eq!(
                changed.operations.keys().next().unwrap(),
                &operation_id,
                "ContractOperationId excludes mutable operation surface"
            );
        }
    }

    #[test]
    fn service_contract_identity_rejects_noncanonical_boundary_value_plans() {
        let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let canonical = service_contract(type_id);

        for mutation in 0..8 {
            let mut invalid = canonical.clone();
            let operation = &mut invalid.operations.values_mut().next().unwrap().contract;
            let plan = if mutation < 4 {
                &mut operation.parameters[0].value_plan
            } else {
                &mut operation.return_value.value_plan
            };
            match mutation % 4 {
                0 => set_plan_owner(
                    plan,
                    if mutation < 4 {
                        BoundaryValueOwner::Provider
                    } else {
                        BoundaryValueOwner::Caller
                    },
                ),
                1 => set_plan_lifetime(plan, BoundaryValueLifetime::Request),
                2 => set_plan_carrier(plan, BoundaryValueCarrier::CallbackCapability),
                3 => set_plan_encoding(plan, BoundaryValueEncoding::OpaqueCapability),
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    service_protocol_identity(&invalid),
                    Err(ArtifactIdentityError::InvalidServiceContract { .. })
                ),
                "boundary value-plan mutation {mutation} must be rejected before hashing"
            );
        }
    }

    #[test]
    fn service_contract_identity_rejects_noncanonical_server_stream_setup() {
        let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let mut canonical = service_contract(type_id);
        canonical
            .operations
            .values_mut()
            .next()
            .unwrap()
            .contract
            .stream = BoundaryStreamContract::ServerStream {
            item_type: ContractTypeRef::builtin("string"),
            item_value_plan: value_plan(
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            ),
        };

        for mutation in 0..6 {
            let mut invalid = canonical.clone();
            let operation = &mut invalid.operations.values_mut().next().unwrap().contract;
            match mutation {
                0 => operation.return_value.ty = ContractTypeRef::builtin("string"),
                1 => {
                    operation.stream = BoundaryStreamContract::Unsupported {
                        reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
                    }
                }
                2..=5 => {
                    let BoundaryStreamContract::ServerStream {
                        item_value_plan, ..
                    } = &mut operation.stream
                    else {
                        unreachable!()
                    };
                    match mutation {
                        2 => set_plan_owner(item_value_plan, BoundaryValueOwner::Caller),
                        3 => set_plan_lifetime(item_value_plan, BoundaryValueLifetime::Call),
                        4 => set_plan_carrier(
                            item_value_plan,
                            BoundaryValueCarrier::CallbackCapability,
                        ),
                        5 => set_plan_encoding(
                            item_value_plan,
                            BoundaryValueEncoding::OpaqueCapability,
                        ),
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    service_protocol_identity(&invalid),
                    Err(ArtifactIdentityError::InvalidServiceContract { .. })
                ),
                "server-stream mutation {mutation} must be rejected before hashing"
            );
        }
    }

    #[test]
    fn service_contract_wire_omits_closed_error_set_and_provider_execution_facts() {
        let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let contract = service_contract(type_id);
        let wire = serde_json::to_value(&contract).unwrap();
        let operation = wire["operations"]
            .as_object()
            .and_then(|operations| operations.values().next())
            .expect("operation wire");

        assert!(operation["contract"].get("errors").is_none());
        assert!(operation["contract"].get("maySuspend").is_none());
        assert!(operation["contract"].get("cancellation").is_none());

        for (field, value) in [
            ("errors", serde_json::json!({"kind": "none"})),
            ("maySuspend", serde_json::json!(false)),
            (
                "cancellation",
                serde_json::json!({"kind": "notCancellable"}),
            ),
        ] {
            let mut legacy = wire.clone();
            legacy["operations"]
                .as_object_mut()
                .and_then(|operations| operations.values_mut().next())
                .and_then(|operation| operation.get_mut("contract"))
                .expect("operation contract wire")[field] = value;
            assert!(
                serde_json::from_value::<ServiceContract>(legacy).is_err(),
                "legacy operation field {field} must fail strict decoding"
            );
        }
    }

    #[test]
    fn zero_operation_service_contract_has_stable_identity() {
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: "example.empty".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::new(),
            package_type_requirements: Vec::new(),
            diagnostic_text: ContractDiagnosticText {
                service: "example.empty".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        };
        let first = assign_service_contract_identities(&mut contract).unwrap();
        let second = service_protocol_identity(&contract).unwrap();

        assert_eq!(first, second);
        validate_service_contract_identities(&contract).unwrap();

        let mut unreachable_types = contract;
        unreachable_types.package_type_requirements = vec![PackageTypeRequirement {
            package_id: "example.types".to_string(),
            required_type_ids: vec![PackageSchemaTypeId::new("type:unreachable")],
        }];
        assert!(matches!(
            service_protocol_identity(&unreachable_types),
            Err(ArtifactIdentityError::InvalidServiceContract { .. })
        ));
    }

    #[test]
    fn stale_service_contract_generation_and_identity_prefix_fail_closed() {
        let type_id = package_schema_type_id("example.pkg", "User", &descriptor("string")).unwrap();
        let mut stale_schema = service_contract(type_id.clone());
        stale_schema.schema_version = "skiff-service-contract-v4".to_string();
        assert!(matches!(
            service_protocol_identity(&stale_schema),
            Err(ArtifactIdentityError::InvalidServiceContract { .. })
        ));

        let mut stale_identity = service_contract(type_id);
        assign_service_contract_identities(&mut stale_identity).unwrap();
        stale_identity.service_protocol_identity = ServiceProtocolIdentity::new(
            stale_identity.service_protocol_identity.as_str().replacen(
                SERVICE_PROTOCOL_IDENTITY_PREFIX,
                "skiff-service-protocol-v4:sha256",
                1,
            ),
        );
        assert!(matches!(
            validate_service_contract_identities(&stale_identity),
            Err(ArtifactIdentityError::ServiceProtocolIdentityMismatch { .. })
        ));
    }

    #[test]
    fn recursive_package_schema_records_fail_closed() {
        let type_id = PackageSchemaTypeId::new("forged-self-id");
        let record = PackageSchemaTypeRecord {
            package_id: "example.pkg".to_string(),
            stable_schema_key: "Node".to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "next".to_string(),
                        ContractTypeRef::package_schema("example.pkg", "Node", type_id.clone()),
                    )]),
                },
            },
        };
        let records = BTreeMap::from([(type_id, record)]);
        let error = validate_package_schema_records(&records).unwrap_err();
        assert!(error.to_string().contains("recursive type cycle"));
    }

    fn descriptor(target: &str) -> PackageSchemaCanonicalDescriptor {
        PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Representation {
                target: ContractTypeRef::builtin(target),
            },
        }
    }

    fn index_entry(type_id: PackageSchemaTypeId) -> PackageSchemaIndexEntry {
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id,
            public_path: Some("api.User".to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        }
    }

    #[test]
    fn package_schema_index_rejects_non_public_named_types() {
        let type_id = PackageSchemaTypeId::new("type:user");
        let mut types = BTreeMap::from([("api.User".to_string(), index_entry(type_id))]);
        types.get_mut("api.User").unwrap().nameability = ContractTypeNameability::ClosureOnly;
        let index = PackageSchemaIndex {
            package_id: "example.pkg".to_string(),
            package_schema_index_identity: package_schema_index_identity("example.pkg", &types)
                .unwrap(),
            types,
        };
        let error = validate_package_schema_index(&index).unwrap_err();
        assert!(error.to_string().contains("api.yml public named type"));
    }

    fn service_contract(type_id: PackageSchemaTypeId) -> ServiceContract {
        let operation_id = contract_operation_id("example.service", "1.0.0", "get").unwrap();
        let operation = BoundaryOperationDescriptor {
            operation_id: operation_id.clone(),
            stable_key: "get".to_string(),
            contract: BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "user".to_string(),
                    ty: ContractTypeRef::package_schema("example.pkg", "User", type_id.clone()),
                    value_plan: value_plan(BoundaryValueOwner::Caller, BoundaryValueLifetime::Call),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("void"),
                    value_plan: value_plan(
                        BoundaryValueOwner::Provider,
                        BoundaryValueLifetime::Call,
                    ),
                },
                stream: BoundaryStreamContract::Unary,
                callbacks: BoundaryCallbackContract::None,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        };
        ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: "example.service".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(operation_id, operation)]),
            package_type_requirements: vec![PackageTypeRequirement {
                package_id: "example.pkg".to_string(),
                required_type_ids: vec![type_id],
            }],
            diagnostic_text: ContractDiagnosticText {
                service: String::new(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        }
    }

    fn value_plan(owner: BoundaryValueOwner, lifetime: BoundaryValueLifetime) -> BoundaryValuePlan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime,
        }
    }

    fn set_plan_carrier(plan: &mut BoundaryValuePlan, value: BoundaryValueCarrier) {
        let BoundaryValuePlan::Linkable { carrier, .. } = plan else {
            unreachable!()
        };
        *carrier = value;
    }

    fn set_plan_encoding(plan: &mut BoundaryValuePlan, value: BoundaryValueEncoding) {
        let BoundaryValuePlan::Linkable { encoding, .. } = plan else {
            unreachable!()
        };
        *encoding = value;
    }

    fn set_plan_owner(plan: &mut BoundaryValuePlan, value: BoundaryValueOwner) {
        let BoundaryValuePlan::Linkable { owner, .. } = plan else {
            unreachable!()
        };
        *owner = value;
    }

    fn set_plan_lifetime(plan: &mut BoundaryValuePlan, value: BoundaryValueLifetime) {
        let BoundaryValuePlan::Linkable { lifetime, .. } = plan else {
            unreachable!()
        };
        *lifetime = value;
    }
}

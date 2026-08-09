use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, package_schema_index_identity,
    package_schema_type_id,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryOperationContract,
    BoundaryOperationDescriptor, BoundaryParameter, BoundaryReturn, BoundaryStreamContract,
    BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractDiagnosticText, ContractRequirement, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, PackageBuildId, PackageLocalAbiIdentity,
    PackageSchemaCanonicalDescriptor, PackageSchemaIndex, PackageSchemaIndexEntry,
    PackageSchemaTypeRecord, PackageTypeRequirement, ServiceContract, ServiceProtocolIdentity,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_compiler_input::ResolvedContractDependency;
use skiff_compiler_projection_input::ResolvedPackageSchema;

pub(crate) fn resolved_contract_fixture(
    alias: &str,
    service_id: &str,
    operation_key: &str,
    public_type_key: &str,
    second_type_key: &str,
) -> ResolvedContractDependency {
    let (contract, schema) = contract_and_schema(
        service_id,
        "1.0.0",
        operation_key,
        public_type_key,
        second_type_key,
    );
    ResolvedContractDependency::validated(requirement(alias, &contract), contract, &[schema])
        .unwrap()
}

pub(crate) fn resolved_nullable_field_contract_fixture(
    alias: &str,
    service_id: &str,
    operation_key: &str,
    outer_type_key: &str,
    nullable_field: &str,
    inner_type_key: &str,
) -> ResolvedContractDependency {
    let version = "1.0.0";
    let package_id = format!("{service_id}.package");
    let inner_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("code".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let inner_record = schema_record(&package_id, inner_type_key, inner_descriptor);
    let outer_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([(
                nullable_field.to_string(),
                ContractTypeRef::Nullable {
                    inner: Box::new(ContractTypeRef::package_schema(
                        &package_id,
                        inner_type_key,
                        inner_record.package_schema_type_id.clone(),
                    )),
                },
            )]),
        },
    };
    let outer_record = schema_record(&package_id, outer_type_key, outer_descriptor);
    let (contract, schema) = contract_and_schema_from_records(
        service_id,
        version,
        operation_key,
        outer_record,
        inner_record,
    );
    ResolvedContractDependency::validated(requirement(alias, &contract), contract, &[schema])
        .unwrap()
}

pub(crate) fn requirement(alias: &str, contract: &ServiceContract) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

pub(crate) fn contract_and_schema(
    service_id: &str,
    version: &str,
    operation_key: &str,
    public_type_key: &str,
    second_type_key: &str,
) -> (ServiceContract, ResolvedPackageSchema) {
    let package_id = format!("{service_id}.package");
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    contract_and_schema_from_records(
        service_id,
        version,
        operation_key,
        schema_record(&package_id, public_type_key, descriptor.clone()),
        schema_record(&package_id, second_type_key, descriptor),
    )
}

fn schema_record(
    package_id: &str,
    stable_schema_key: &str,
    canonical_descriptor: PackageSchemaCanonicalDescriptor,
) -> PackageSchemaTypeRecord {
    PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id: package_schema_type_id(
            package_id,
            stable_schema_key,
            &canonical_descriptor,
        )
        .unwrap(),
        canonical_descriptor,
    }
}

fn contract_and_schema_from_records(
    service_id: &str,
    version: &str,
    operation_key: &str,
    parameter_record: PackageSchemaTypeRecord,
    return_record: PackageSchemaTypeRecord,
) -> (ServiceContract, ResolvedPackageSchema) {
    let package_id = parameter_record.package_id.clone();
    assert_eq!(return_record.package_id, package_id);
    let parameter_type = ContractTypeRef::package_schema(
        &package_id,
        &parameter_record.stable_schema_key,
        parameter_record.package_schema_type_id.clone(),
    );
    let return_type = ContractTypeRef::package_schema(
        &package_id,
        &return_record.stable_schema_key,
        return_record.package_schema_type_id.clone(),
    );
    let records = [parameter_record, return_record]
        .into_iter()
        .map(|record| (record.package_schema_type_id.clone(), record))
        .collect::<BTreeMap<_, _>>();
    let index_types = records
        .values()
        .map(|record| {
            (
                record.stable_schema_key.clone(),
                PackageSchemaIndexEntry {
                    package_schema_type_id: record.package_schema_type_id.clone(),
                    public_path: Some(record.stable_schema_key.clone()),
                    nameability: ContractTypeNameability::PublicNameable,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    let index = PackageSchemaIndex {
        package_id: package_id.clone(),
        package_schema_index_identity: package_schema_index_identity(&package_id, &index_types)
            .unwrap(),
        types: index_types,
    };
    let required_type_ids = records.keys().cloned().collect();
    let schema = ResolvedPackageSchema::new(
        "schema".to_string(),
        package_id.clone(),
        version.to_string(),
        PackageBuildId::new("test-build"),
        PackageLocalAbiIdentity::new("test-abi"),
        index,
        records,
    )
    .unwrap();
    let operation_id = contract_operation_id(service_id, version, operation_key).unwrap();
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: operation_key.to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "input".to_string(),
                ty: parameter_type,
                value_plan: linkable(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: return_type,
                value_plan: linkable(BoundaryValueOwner::Provider),
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
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, operation)]),
        public_instances: BTreeMap::new(),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id,
            required_type_ids,
        }],
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    (contract, schema)
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

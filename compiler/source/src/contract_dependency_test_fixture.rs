use std::collections::BTreeMap;

use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, package_schema_index_identity,
    package_schema_type_id,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractRequirement, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, PackageBuildId, PackageLocalAbiIdentity, PackageSchemaCanonicalDescriptor,
    PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaTypeRecord, PackageTypeRequirement,
    ServiceContract, ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
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
    let public_type_id = package_schema_type_id(&package_id, public_type_key, &descriptor).unwrap();
    let second_type_id = package_schema_type_id(&package_id, second_type_key, &descriptor).unwrap();
    let records = [
        (public_type_key, public_type_id.clone()),
        (second_type_key, second_type_id.clone()),
    ]
    .into_iter()
    .map(|(key, id)| {
        (
            id.clone(),
            PackageSchemaTypeRecord {
                package_id: package_id.clone(),
                stable_schema_key: key.to_string(),
                package_schema_type_id: id,
                canonical_descriptor: descriptor.clone(),
            },
        )
    })
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
                ty: ContractTypeRef::package_schema(
                    &package_id,
                    public_type_key,
                    public_type_id.clone(),
                ),
                value_plan: linkable(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::package_schema(
                    &package_id,
                    second_type_key,
                    second_type_id.clone(),
                ),
                value_plan: linkable(BoundaryValueOwner::Provider),
            },
            errors: BoundaryErrorContract::None,
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
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
        package_type_requirements: vec![PackageTypeRequirement {
            package_id,
            required_type_ids: vec![public_type_id, second_type_id]
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
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

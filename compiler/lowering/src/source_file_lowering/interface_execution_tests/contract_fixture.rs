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

pub(super) fn contract_dependency() -> (
    ResolvedContractDependency,
    PackageSchemaTypeRecord,
    PackageLocalAbiIdentity,
) {
    let service_id = "example.payments";
    let service_version = "1.0.0";
    let package_id = "example.types";
    let package_version = "1.0.0";
    let stable_schema_key = "User";
    let package_local_abi = PackageLocalAbiIdentity::new("abi:example.types");
    let canonical_descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let package_schema_type_id =
        package_schema_type_id(package_id, stable_schema_key, &canonical_descriptor).unwrap();
    let record = PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_schema_key.to_string(),
        package_schema_type_id: package_schema_type_id.clone(),
        canonical_descriptor,
    };
    let index_types = BTreeMap::from([(
        stable_schema_key.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: package_schema_type_id.clone(),
            public_path: Some(stable_schema_key.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let schema = ResolvedPackageSchema::new(
        "types".to_string(),
        package_id.to_string(),
        package_version.to_string(),
        PackageBuildId::new("build:example.types"),
        package_local_abi.clone(),
        PackageSchemaIndex {
            package_id: package_id.to_string(),
            package_schema_index_identity: package_schema_index_identity(package_id, &index_types)
                .unwrap(),
            types: index_types,
        },
        BTreeMap::from([(package_schema_type_id.clone(), record.clone())]),
    )
    .unwrap();
    let package_type = || {
        ContractTypeRef::package_schema(
            package_id,
            stable_schema_key,
            package_schema_type_id.clone(),
        )
    };
    let ping_operation_id = contract_operation_id(service_id, service_version, "ping").unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: service_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            ping_operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: ping_operation_id,
                stable_key: "ping".to_string(),
                contract: unary_operation(package_type(), package_type()),
            },
        )]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: package_id.to_string(),
            required_type_ids: vec![package_schema_type_id],
        }],
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    let requirement = ContractRequirement {
        alias: "payments".to_string(),
        service_id: service_id.to_string(),
        contract_version: service_version.to_string(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    };
    (
        ResolvedContractDependency::validated(requirement, contract, &[schema]).unwrap(),
        record,
        package_local_abi,
    )
}

fn unary_operation(
    parameter_type: ContractTypeRef,
    return_type: ContractTypeRef,
) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "input".to_string(),
            ty: parameter_type,
            value_plan: value_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: return_type,
            value_plan: value_plan(BoundaryValueOwner::Provider),
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
    }
}

fn value_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

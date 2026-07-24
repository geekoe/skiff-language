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
    PackageSchemaIndex, PackageSchemaIndexEntry, PackageSchemaTypeId, PackageSchemaTypeRecord,
    PackageTypeRequirement, ServiceContract, ServiceProtocolIdentity,
    SERVICE_CONTRACT_SCHEMA_VERSION,
};
use skiff_compiler_projection_input::ResolvedPackageSchema;

use super::*;

#[test]
fn validated_dependency_indexes_package_owned_public_type() {
    let (contract, schema) = fixture();
    let dependency =
        ResolvedContractDependency::validated(requirement("echo", &contract), contract, &[schema])
            .unwrap();
    let index = ContractDependencyIndex::build([dependency]).unwrap();
    let record = index
        .public_package_type_by_stable_key("echo", "Payload")
        .unwrap();
    assert_eq!(record.package_id, "example.types");
    assert_eq!(
        index
            .operation_by_stable_key("echo", "echo")
            .unwrap()
            .stable_key,
        "echo"
    );
}

#[test]
fn missing_and_extra_schema_inputs_fail_closed() {
    let (contract, schema) = fixture();
    assert!(matches!(
        ResolvedContractDependency::validated(
            requirement("echo", &contract),
            contract.clone(),
            &[]
        ),
        Err(ContractDependencyError::MissingPackageSchema { .. })
    ));

    let mut contract_with_extra = contract;
    contract_with_extra.package_type_requirements[0]
        .required_type_ids
        .push(PackageSchemaTypeId::new("missing"));
    assert!(matches!(
        ResolvedContractDependency::validated(
            requirement("echo", &contract_with_extra),
            contract_with_extra,
            &[schema]
        ),
        Err(ContractDependencyError::InvalidContract { .. })
            | Err(ContractDependencyError::MissingSchemaRecord { .. })
    ));
}

#[test]
fn operation_owner_or_key_mismatch_fails_closed() {
    let (mut contract, schema) = fixture();
    let ContractTypeRef::PackageSchema {
        stable_schema_key, ..
    } = &mut contract
        .operations
        .values_mut()
        .next()
        .unwrap()
        .contract
        .parameters[0]
        .ty
    else {
        panic!("fixture nominal");
    };
    *stable_schema_key = "Other".to_string();
    assign_service_contract_identities(&mut contract).unwrap();
    assert!(matches!(
        ResolvedContractDependency::validated(requirement("echo", &contract), contract, &[schema]),
        Err(ContractDependencyError::SchemaReferenceMismatch { .. })
    ));
}

#[test]
fn strict_json_reader_rejects_provider_fields_before_schema_resolution() {
    let (contract, schema) = fixture();
    let mut wire = serde_json::to_value(&contract).unwrap();
    wire.as_object_mut().unwrap().insert(
        "providerBuildId".to_string(),
        serde_json::json!("forbidden"),
    );
    assert!(matches!(
        read_contract_dependency_json(
            "provider",
            &serde_json::to_vec(&wire).unwrap(),
            requirement("echo", &contract),
            &[schema],
        ),
        Err(ContractDependencyError::Parse { .. })
    ));
}

#[test]
fn store_backed_contract_and_schema_records_cross_the_real_input_boundary() {
    let (contract, schema) = fixture();
    let root = std::env::temp_dir().join(format!(
        "skiff-f164-store-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let store = skiff_deployment::storage::CanonicalArtifactStore::create(&root).unwrap();
    store.write_package_schema_index(schema.index()).unwrap();
    for record in schema.records().values() {
        store.write_package_schema_type_record(record).unwrap();
    }
    let index_ref = skiff_artifact_model::PackageSchemaIndexRef {
        package_id: schema.package_id().to_string(),
        package_schema_index_identity: schema.index().package_schema_index_identity.clone(),
    };
    let loaded_index = store.read_package_schema_index(&index_ref).unwrap();
    let loaded_records = schema
        .records()
        .keys()
        .map(|type_id| {
            let reference = skiff_artifact_model::PackageSchemaTypeRecordRef {
                package_id: schema.package_id().to_string(),
                package_schema_type_id: type_id.clone(),
            };
            let record = store.read_package_schema_type_record(&reference).unwrap();
            (type_id.clone(), record.as_ref().clone())
        })
        .collect();
    let loaded_schema = ResolvedPackageSchema::new(
        schema.alias().to_string(),
        schema.package_id().to_string(),
        schema.exact_version().to_string(),
        schema.package_build_id().clone(),
        schema.expected_local_abi().clone(),
        loaded_index.as_ref().clone(),
        loaded_records,
    )
    .unwrap();
    let dependency = read_contract_dependency_json(
        "store-backed-contract",
        &serde_json::to_vec(&contract).unwrap(),
        requirement("echo", &contract),
        &[loaded_schema],
    )
    .unwrap();
    assert_eq!(
        dependency
            .schema_records()
            .values()
            .next()
            .unwrap()
            .package_id,
        "example.types"
    );
    std::fs::remove_dir_all(root).unwrap();
}

fn fixture() -> (ServiceContract, ResolvedPackageSchema) {
    let package_id = "example.types";
    let stable_key = "Payload";
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([("value".to_string(), ContractTypeRef::builtin("string"))]),
        },
    };
    let type_id = package_schema_type_id(package_id, stable_key, &descriptor).unwrap();
    let record = PackageSchemaTypeRecord {
        package_id: package_id.to_string(),
        stable_schema_key: stable_key.to_string(),
        package_schema_type_id: type_id.clone(),
        canonical_descriptor: descriptor,
    };
    let index_types = BTreeMap::from([(
        stable_key.to_string(),
        PackageSchemaIndexEntry {
            package_schema_type_id: type_id.clone(),
            public_path: Some(stable_key.to_string()),
            nameability: ContractTypeNameability::PublicNameable,
        },
    )]);
    let schema = ResolvedPackageSchema::new(
        "types".to_string(),
        package_id.to_string(),
        "1.0.0".to_string(),
        PackageBuildId::new("build"),
        PackageLocalAbiIdentity::new("abi"),
        PackageSchemaIndex {
            package_id: package_id.to_string(),
            package_schema_index_identity: package_schema_index_identity(package_id, &index_types)
                .unwrap(),
            types: index_types,
        },
        BTreeMap::from([(type_id.clone(), record)]),
    )
    .unwrap();
    let operation_id = contract_operation_id("example.echo", "1.0.0", "echo").unwrap();
    let operation = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: "echo".to_string(),
        contract: BoundaryOperationContract {
            parameters: vec![BoundaryParameter {
                name: "input".to_string(),
                ty: ContractTypeRef::package_schema(package_id, stable_key, type_id.clone()),
                value_plan: value_plan(BoundaryValueOwner::Caller),
            }],
            return_value: BoundaryReturn {
                ty: ContractTypeRef::package_schema(package_id, stable_key, type_id.clone()),
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
        },
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: "example.echo".to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, operation)]),
        package_type_requirements: vec![PackageTypeRequirement {
            package_id: package_id.to_string(),
            required_type_ids: vec![type_id],
        }],
        diagnostic_text: ContractDiagnosticText {
            service: "echo".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    (contract, schema)
}

fn requirement(alias: &str, contract: &ServiceContract) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
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

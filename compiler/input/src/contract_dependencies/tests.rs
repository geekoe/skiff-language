use std::collections::BTreeMap;

use serde_json::json;
use skiff_artifact_identity::{
    assign_service_contract_identities, contract_operation_id, contract_type_id,
};
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractRequirement, ContractSchemaType, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, ContractTypeShape, ServiceContract,
    ServiceProtocolIdentity, SERVICE_CONTRACT_SCHEMA_VERSION,
};

use super::*;

#[test]
fn contract_only_dependency_builds_strict_typed_indexes_without_provider_artifacts() {
    let contract = contract_fixture("example.echo", "1.0.0", &["echo", "status"]);
    let dependency = read_fixture("echo-contract", "echo", &contract).unwrap();
    let index = ContractDependencyIndex::build([dependency]).unwrap();

    assert_eq!(index.len(), 1);
    assert_eq!(
        index.requirement("echo").unwrap().service_id,
        "example.echo"
    );
    assert_eq!(
        index
            .operation_by_stable_key("echo", "status")
            .unwrap()
            .stable_key,
        "status"
    );
    assert_eq!(
        index
            .contract_schema_type_by_stable_key("echo", "payload")
            .unwrap()
            .stable_key,
        "payload"
    );
    assert_eq!(
        index
            .public_contract_type_id_by_stable_key("echo", "payload")
            .unwrap(),
        &contract_type_id("example.echo", "1.0.0", "payload").unwrap()
    );
    assert!(matches!(
        index.public_contract_type_id_by_stable_key("echo", "payloadClosure"),
        Err(ContractDependencyError::ContractTypeNotPublicNameable { .. })
    ));

    let wire = serde_json::to_string(index.contract("echo").unwrap()).unwrap();
    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "serviceAssembly",
        "serviceUnit",
        "packageUnit",
        "publicationAbi",
        "deploymentRevision",
        "route",
        "executableTarget",
    ] {
        assert!(
            !wire.contains(forbidden),
            "unexpected provider field {forbidden}"
        );
    }
}

#[test]
fn trust_boundary_rejects_provider_fields_and_duplicate_operation_keys() {
    let contract = contract_fixture("example.echo", "1.0.0", &["echo"]);
    let mut with_provider = serde_json::to_value(&contract).unwrap();
    with_provider
        .as_object_mut()
        .unwrap()
        .insert("providerBuildId".to_string(), json!("forbidden"));
    assert!(matches!(
        read_contract_dependency_json(
            "provider-field",
            &serde_json::to_vec(&with_provider).unwrap(),
            requirement("echo", &contract),
        ),
        Err(ContractDependencyError::Parse { .. })
    ));

    let duplicate_operations = contract_json_with_duplicate_operation_key(&contract);
    let error = read_contract_dependency_json(
        "duplicate-operation",
        duplicate_operations.as_bytes(),
        requirement("echo", &contract),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("duplicate JSON object key"), "{error}");
}

#[test]
fn coordinates_protocol_aliases_and_nominal_type_domain_fail_closed() {
    let contract = contract_fixture("example.echo", "1.0.0", &["echo"]);

    let mut wrong_coordinate = requirement("echo", &contract);
    wrong_coordinate.contract_version = "2.0.0".to_string();
    assert!(matches!(
        read_contract_dependency_json(
            "coordinate-mismatch",
            &serde_json::to_vec(&contract).unwrap(),
            wrong_coordinate,
        ),
        Err(ContractDependencyError::CoordinateMismatch { .. })
    ));

    let mut wrong_protocol = requirement("echo", &contract);
    wrong_protocol.expected_protocol_identity = ServiceProtocolIdentity::new("wrong-protocol");
    assert!(matches!(
        read_contract_dependency_json(
            "protocol-mismatch",
            &serde_json::to_vec(&contract).unwrap(),
            wrong_protocol,
        ),
        Err(ContractDependencyError::ProtocolIdentityMismatch { .. })
    ));

    let mut invalid_nominal = serde_json::to_value(&contract).unwrap();
    let operation_id = contract.operations.keys().next().unwrap().as_str();
    invalid_nominal["operations"][operation_id]["contract"]["parameters"][0]["ty"] =
        json!({ "kind": "contract", "abiTypeId": "package-local-type" });
    assert!(matches!(
        read_contract_dependency_json(
            "package-local-nominal",
            &serde_json::to_vec(&invalid_nominal).unwrap(),
            requirement("echo", &contract),
        ),
        Err(ContractDependencyError::Parse { .. })
    ));

    for alias in ["", "Echo", "root", "has-dash"] {
        assert!(matches!(
            read_contract_dependency_json(
                "invalid-alias",
                &serde_json::to_vec(&contract).unwrap(),
                requirement(alias, &contract),
            ),
            Err(ContractDependencyError::InvalidAlias { .. })
        ));
    }
}

#[test]
fn duplicate_and_unknown_index_keys_fail_closed() {
    let contract = contract_fixture("example.echo", "1.0.0", &["echo"]);
    let first = read_fixture("first", "echo", &contract).unwrap();
    let duplicate = read_fixture("duplicate", "echo", &contract).unwrap();
    assert!(matches!(
        ContractDependencyIndex::build([first, duplicate]),
        Err(ContractDependencyError::DuplicateAlias { .. })
    ));

    let index =
        ContractDependencyIndex::build([read_fixture("single", "echo", &contract).unwrap()])
            .unwrap();
    assert!(matches!(
        index.requirement("missing"),
        Err(ContractDependencyError::UnknownAlias { .. })
    ));
    assert!(matches!(
        index.operation(
            "echo",
            &skiff_artifact_model::ContractOperationId::new("unknown-operation")
        ),
        Err(ContractDependencyError::UnknownOperation { .. })
    ));
    assert!(matches!(
        index.contract_type(
            "echo",
            &skiff_artifact_model::ContractTypeId::new("unknown-type")
        ),
        Err(ContractDependencyError::UnknownType { .. })
    ));
}

#[test]
fn service_requirement_cycles_need_no_provider_closure_but_cross_contract_schema_refs_fail() {
    let contract_a = contract_fixture("example.a", "1.0.0", &["call_b"]);
    let contract_b = contract_fixture("example.b", "1.0.0", &["call_a"]);
    let index = ContractDependencyIndex::build([
        read_fixture("a", "a", &contract_a).unwrap(),
        read_fixture("b", "b", &contract_b).unwrap(),
    ])
    .unwrap();
    assert_eq!(
        index.len(),
        2,
        "flat contract lookup has no provider closure"
    );

    let mut cross_contract = contract_a.clone();
    let foreign_type_id = contract_b.boundary_schema.keys().next().unwrap().clone();
    cross_contract
        .operations
        .values_mut()
        .next()
        .unwrap()
        .contract
        .parameters[0]
        .ty = ContractTypeRef::contract(foreign_type_id);
    assert!(matches!(
        read_contract_dependency_json(
            "cross-contract-schema",
            &serde_json::to_vec(&cross_contract).unwrap(),
            requirement("a", &contract_a),
        ),
        Err(ContractDependencyError::InvalidContract { .. })
    ));
}

fn read_fixture(
    label: &str,
    alias: &str,
    contract: &ServiceContract,
) -> Result<ResolvedContractDependency, ContractDependencyError> {
    read_contract_dependency_json(
        label,
        &serde_json::to_vec(contract).unwrap(),
        requirement(alias, contract),
    )
}

fn requirement(alias: &str, contract: &ServiceContract) -> ContractRequirement {
    ContractRequirement {
        alias: alias.to_string(),
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        expected_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn contract_fixture(service_id: &str, version: &str, operation_keys: &[&str]) -> ServiceContract {
    let payload_type_id = contract_type_id(service_id, version, "payload").unwrap();
    let closure_type_id = contract_type_id(service_id, version, "payloadClosure").unwrap();
    let operations = operation_keys
        .iter()
        .map(|stable_key| {
            let operation_id = contract_operation_id(service_id, version, stable_key).unwrap();
            (
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id,
                    stable_key: (*stable_key).to_string(),
                    contract: BoundaryOperationContract {
                        parameters: vec![BoundaryParameter {
                            name: "input".to_string(),
                            ty: ContractTypeRef::contract(payload_type_id.clone()),
                            value_plan: linkable(BoundaryValueOwner::Caller),
                        }],
                        return_value: BoundaryReturn {
                            ty: ContractTypeRef::contract(payload_type_id.clone()),
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
                },
            )
        })
        .collect();
    let boundary_schema = BTreeMap::from([
        (
            payload_type_id.clone(),
            ContractSchemaType {
                contract_type_id: payload_type_id,
                stable_key: "payload".to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::PublicNameable,
                    descriptor: ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "message".to_string(),
                            ContractTypeRef::contract(closure_type_id.clone()),
                        )]),
                    },
                },
            },
        ),
        (
            closure_type_id.clone(),
            ContractSchemaType {
                contract_type_id: closure_type_id,
                stable_key: "payloadClosure".to_string(),
                shape: ContractTypeShape {
                    nameability: ContractTypeNameability::ClosureOnly,
                    descriptor: ContractTypeDescriptor::Record {
                        fields: BTreeMap::from([(
                            "value".to_string(),
                            ContractTypeRef::builtin("string"),
                        )]),
                    },
                },
            },
        ),
    ]);
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations,
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    assign_service_contract_identities(&mut contract).unwrap();
    contract
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn contract_json_with_duplicate_operation_key(contract: &ServiceContract) -> String {
    let value = serde_json::to_value(contract).unwrap();
    let object = value.as_object().unwrap();
    let (operation_id, operation) = object["operations"]
        .as_object()
        .unwrap()
        .iter()
        .next()
        .unwrap();
    let operation_pair = format!(
        "{}:{}",
        serde_json::to_string(operation_id).unwrap(),
        serde_json::to_string(operation).unwrap()
    );
    let operations = format!("{{{operation_pair},{operation_pair}}}");
    let fields = object
        .iter()
        .map(|(key, field)| {
            let value = if key == "operations" {
                operations.clone()
            } else {
                serde_json::to_string(field).unwrap()
            };
            format!("{}:{value}", serde_json::to_string(key).unwrap())
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{fields}}}")
}

#[test]
fn strict_json_value_preserves_ordinary_json_shapes() {
    let input = br#"{"array":[null,true,-2,3,1.5,"text"],"object":{"key":"value"}}"#;
    let parsed = serde_json::from_slice::<strict_json::StrictJsonValue>(input)
        .unwrap()
        .into_inner();
    assert_eq!(
        parsed,
        json!({
            "array": [null, true, -2, 3, 1.5, "text"],
            "object": { "key": "value" }
        })
    );
}

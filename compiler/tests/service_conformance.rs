mod common;

use std::collections::BTreeMap;

use serde_json::json;
use skiff_artifact_identity::validate_service_contract_identities;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryEffectGuarantee,
    BoundaryErrorContract, BoundaryOperationContract, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeDescriptor, ContractTypeNameability,
    ContractTypeRef, ContractTypeShape,
};
use skiff_compiler::{
    definition_contract_operation_id, definition_contract_type_id, definition_contract_type_ref,
    ContractDefinitionError, ServiceContractDefinition, ServiceContractDefinitionDiagnosticText,
};

use common::contracts::compile_service_contract;

const SERVICE_ID: &str = "example.echo";
const CONTRACT_VERSION: &str = "1.0.0";

#[test]
fn explicit_definition_compiles_to_a_code_free_service_contract() {
    let definition = contract_definition();
    let expected_operation = definition.operations["echo"].clone();
    let expected_schema = definition.boundary_schema["request"].clone();

    let contract = compile_service_contract(definition).expect("explicit contract should compile");
    validate_service_contract_identities(&contract).expect("contract identities should be valid");

    let operation_id =
        definition_contract_operation_id(SERVICE_ID, CONTRACT_VERSION, "echo").unwrap();
    let type_id = definition_contract_type_id(SERVICE_ID, CONTRACT_VERSION, "request").unwrap();
    let operation = contract
        .operations
        .get(&operation_id)
        .expect("stable operation key should derive a contract-owned identity");
    let schema = contract
        .boundary_schema
        .get(&type_id)
        .expect("stable type key should derive a contract-owned identity");

    assert_eq!(contract.service_id, SERVICE_ID);
    assert_eq!(contract.contract_version, CONTRACT_VERSION);
    assert_eq!(operation.operation_id, operation_id);
    assert_eq!(operation.stable_key, "echo");
    assert_eq!(operation.contract, expected_operation);
    assert_eq!(schema.contract_type_id, type_id);
    assert_eq!(schema.stable_key, "request");
    assert_eq!(schema.shape, expected_schema);
    assert_eq!(contract.diagnostic_text.operations[&operation_id], "Echo");
    assert_eq!(contract.diagnostic_text.types[&type_id], "Echo request");
}

#[test]
fn definition_wire_rejects_provider_and_deployment_state() {
    let value = serde_json::to_value(contract_definition()).unwrap();
    let definition: ServiceContractDefinition = serde_json::from_value(value.clone()).unwrap();
    compile_service_contract(definition).expect("strict code-free definition should compile");

    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "deploymentRevision",
        "operationBindings",
        "ingress",
        "configBindings",
        "runtimeReplica",
    ] {
        let mut invalid = value.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!("forbidden"));
        assert!(
            serde_json::from_value::<ServiceContractDefinition>(invalid).is_err(),
            "{forbidden} must not enter the contract definition"
        );
    }
}

#[test]
fn missing_contract_schema_reference_fails_closed() {
    let mut definition = contract_definition();
    definition.operations.get_mut("echo").unwrap().parameters[0].ty =
        definition_contract_type_ref(SERVICE_ID, CONTRACT_VERSION, "missing").unwrap();

    assert!(matches!(
        compile_service_contract(definition),
        Err(ContractDefinitionError::Identity(_))
    ));
}

#[test]
fn protocol_identity_tracks_semantics_but_not_diagnostic_text() {
    let baseline = compile_service_contract(contract_definition()).unwrap();

    let mut renamed = contract_definition();
    renamed.diagnostic_text.service = "Renamed service".to_string();
    renamed
        .diagnostic_text
        .operations
        .insert("echo".to_string(), "Renamed operation".to_string());
    renamed
        .diagnostic_text
        .types
        .insert("request".to_string(), "Renamed type".to_string());
    let renamed = compile_service_contract(renamed).unwrap();
    assert_eq!(
        baseline.service_protocol_identity,
        renamed.service_protocol_identity
    );

    let mut changed = contract_definition();
    changed.operations.get_mut("echo").unwrap().may_suspend = false;
    let changed = compile_service_contract(changed).unwrap();
    assert_ne!(
        baseline.service_protocol_identity,
        changed.service_protocol_identity
    );
}

fn contract_definition() -> ServiceContractDefinition {
    let request = definition_contract_type_ref(SERVICE_ID, CONTRACT_VERSION, "request").unwrap();
    ServiceContractDefinition {
        service_id: SERVICE_ID.to_string(),
        contract_version: CONTRACT_VERSION.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "request".to_string(),
                    ty: request,
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: ContractTypeRef::builtin("string"),
                    value_plan: linkable(BoundaryValueOwner::Provider),
                },
                errors: BoundaryErrorContract::None,
                stream: BoundaryStreamContract::Unary,
                cancellation: BoundaryCancellationContract::Cooperative,
                callbacks: BoundaryCallbackContract::None,
                may_suspend: true,
                effect_guarantee: BoundaryEffectGuarantee {
                    detached_parameters: true,
                    detached_return: true,
                    detached_error: true,
                    no_caller_reachable_mutation: true,
                    no_caller_value_escape: true,
                    no_same_heap_identity: true,
                },
            },
        )]),
        boundary_schema: BTreeMap::from([(
            "request".to_string(),
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                descriptor: ContractTypeDescriptor::Record {
                    fields: BTreeMap::from([(
                        "message".to_string(),
                        ContractTypeRef::builtin("string"),
                    )]),
                },
            },
        )]),
        diagnostic_text: ServiceContractDefinitionDiagnosticText {
            service: "Echo service".to_string(),
            operations: BTreeMap::from([("echo".to_string(), "Echo".to_string())]),
            types: BTreeMap::from([("request".to_string(), "Echo request".to_string())]),
        },
    }
}

fn linkable(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

use std::collections::BTreeMap;

use serde_json::json;
use skiff_artifact_identity::validate_service_contract_identities;
use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackOperation, BoundaryCancellationContract,
    BoundaryEffectGuarantee, BoundaryErrorContract, BoundaryOperationContract, BoundaryParameter,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeDescriptor,
    ContractTypeNameability, ContractTypeRef, ContractTypeShape,
};

use super::*;

mod schema_fidelity;

#[test]
fn definition_compiles_without_any_provider_code_or_artifact() {
    let definition = definition_fixture();
    let contract = compile_service_contract_definition(definition).unwrap();
    validate_service_contract_identities(&contract).unwrap();
    assert_eq!(contract.service_id, "example.echo");
    assert_eq!(contract.operations.len(), 1);
    assert_eq!(contract.boundary_schema.len(), 1);

    let round_trip = serde_json::from_value::<skiff_artifact_model::ServiceContract>(
        serde_json::to_value(&contract).unwrap(),
    )
    .unwrap();
    assert_eq!(round_trip, contract);
}

#[test]
fn compiled_service_contract_requires_real_operation_descriptors() {
    let contract = compile_service_contract_definition(definition_fixture()).unwrap();
    let wire = serde_json::to_value(&contract).unwrap();
    let descriptor = wire["operations"]
        .as_object()
        .and_then(|operations| operations.values().next())
        .expect("compiled contract operation descriptor");
    assert_eq!(descriptor["stableKey"], json!("echo"));
    assert!(descriptor.get("operationId").is_some());
    assert!(descriptor.get("contract").is_some());

    for required in ["operationId", "stableKey", "contract"] {
        let mut invalid = wire.clone();
        let descriptor = invalid["operations"]
            .as_object_mut()
            .and_then(|operations| operations.values_mut().next())
            .and_then(serde_json::Value::as_object_mut)
            .expect("compiled contract operation descriptor");
        descriptor.remove(required);
        assert!(
            serde_json::from_value::<skiff_artifact_model::ServiceContract>(invalid).is_err(),
            "ServiceContract descriptor must require {required}"
        );
    }
}

#[test]
fn definition_wire_rejects_provider_deployment_and_unknown_fields() {
    let value = serde_json::to_value(definition_fixture()).unwrap();
    for forbidden in [
        "providerPackageId",
        "providerBuildId",
        "deploymentRevision",
        "route",
        "runtimeState",
    ] {
        let mut invalid = value.clone();
        invalid
            .as_object_mut()
            .unwrap()
            .insert(forbidden.to_string(), json!("forbidden"));
        assert!(serde_json::from_value::<ServiceContractDefinition>(invalid).is_err());
    }
}

#[test]
fn missing_schema_reference_fails_closed() {
    let mut definition = definition_fixture();
    definition.operations.get_mut("echo").unwrap().parameters[0].ty =
        definition_contract_type_ref("example.echo", "1.0.0", "missing").unwrap();
    assert!(matches!(
        compile_service_contract_definition(definition),
        Err(ContractDefinitionError::Identity(
            skiff_artifact_identity::ArtifactIdentityError::InvalidServiceContract { .. }
        ))
    ));
}

#[test]
fn operation_and_type_map_order_do_not_change_output() {
    let first = compile_service_contract_definition(definition_fixture()).unwrap();
    let mut reordered = definition_fixture();
    let mut operations = reordered.operations.into_iter().collect::<Vec<_>>();
    operations.reverse();
    reordered.operations = operations.into_iter().collect();
    let mut schema = reordered.boundary_schema.into_iter().collect::<Vec<_>>();
    schema.reverse();
    reordered.boundary_schema = schema.into_iter().collect();
    let second = compile_service_contract_definition(reordered).unwrap();
    assert_eq!(first, second);
}

#[test]
fn generic_schema_validates_declarations_and_affects_protocol_identity() {
    let mut definition = definition_fixture();
    let payload = definition.boundary_schema.get_mut("payload").unwrap();
    payload.type_params = vec!["T".to_string(), "U".to_string()];
    let ContractTypeDescriptor::Record { fields } = &mut payload.descriptor else {
        unreachable!()
    };
    fields.insert(
        "first".to_string(),
        ContractTypeRef::TypeParam {
            name: "T".to_string(),
        },
    );
    fields.insert(
        "second".to_string(),
        ContractTypeRef::TypeParam {
            name: "U".to_string(),
        },
    );

    let first = compile_service_contract_definition(definition.clone()).unwrap();
    let rebuilt = compile_service_contract_definition(definition.clone()).unwrap();
    assert_eq!(first, rebuilt);

    let mut reordered = definition.clone();
    reordered
        .boundary_schema
        .get_mut("payload")
        .unwrap()
        .type_params
        .reverse();
    let reordered = compile_service_contract_definition(reordered).unwrap();
    assert_ne!(
        first.service_protocol_identity,
        reordered.service_protocol_identity
    );

    let mut reference_changed = definition.clone();
    let ContractTypeDescriptor::Record { fields } = &mut reference_changed
        .boundary_schema
        .get_mut("payload")
        .unwrap()
        .descriptor
    else {
        unreachable!()
    };
    fields.insert(
        "first".to_string(),
        ContractTypeRef::TypeParam {
            name: "U".to_string(),
        },
    );
    let reference_changed = compile_service_contract_definition(reference_changed).unwrap();
    assert_ne!(
        first.service_protocol_identity,
        reference_changed.service_protocol_identity
    );

    for type_params in [
        vec!["T".to_string(), "T".to_string()],
        vec!["9T".to_string()],
        vec!["".to_string()],
    ] {
        let mut invalid = definition.clone();
        invalid
            .boundary_schema
            .get_mut("payload")
            .unwrap()
            .type_params = type_params;
        assert!(compile_service_contract_definition(invalid).is_err());
    }

    let mut unknown = definition;
    let ContractTypeDescriptor::Record { fields } = &mut unknown
        .boundary_schema
        .get_mut("payload")
        .unwrap()
        .descriptor
    else {
        unreachable!()
    };
    fields.insert(
        "unknown".to_string(),
        ContractTypeRef::TypeParam {
            name: "Missing".to_string(),
        },
    );
    assert!(compile_service_contract_definition(unknown).is_err());
}

#[test]
fn callback_suspend_semantics_affect_protocol_identity() {
    let mut synchronous = definition_fixture();
    synchronous.boundary_schema.insert(
        "callback".to_string(),
        ContractTypeShape {
            nameability: ContractTypeNameability::PublicNameable,
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::CallbackInterface {
                operations: BTreeMap::from([(
                    "observe".to_string(),
                    BoundaryCallbackOperation {
                        parameters: vec![ContractTypeRef::builtin("string")],
                        return_type: ContractTypeRef::builtin("void"),
                        may_suspend: false,
                    },
                )]),
            },
        },
    );
    let synchronous = compile_service_contract_definition(synchronous).unwrap();

    let mut suspending = definition_fixture();
    suspending.boundary_schema.insert(
        "callback".to_string(),
        ContractTypeShape {
            nameability: ContractTypeNameability::PublicNameable,
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::CallbackInterface {
                operations: BTreeMap::from([(
                    "observe".to_string(),
                    BoundaryCallbackOperation {
                        parameters: vec![ContractTypeRef::builtin("string")],
                        return_type: ContractTypeRef::builtin("void"),
                        may_suspend: true,
                    },
                )]),
            },
        },
    );
    let suspending = compile_service_contract_definition(suspending).unwrap();
    assert_ne!(
        synchronous.service_protocol_identity,
        suspending.service_protocol_identity
    );
}

fn definition_fixture() -> ServiceContractDefinition {
    let service_id = "example.echo";
    let version = "1.0.0";
    let payload = definition_contract_type_ref(service_id, version, "payload").unwrap();
    ServiceContractDefinition {
        service_id: service_id.to_string(),
        contract_version: version.to_string(),
        operations: BTreeMap::from([(
            "echo".to_string(),
            BoundaryOperationContract {
                parameters: vec![BoundaryParameter {
                    name: "input".to_string(),
                    ty: payload.clone(),
                    value_plan: linkable(BoundaryValueOwner::Caller),
                }],
                return_value: BoundaryReturn {
                    ty: payload.clone(),
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
        )]),
        boundary_schema: BTreeMap::from([(
            "payload".to_string(),
            ContractTypeShape {
                nameability: ContractTypeNameability::PublicNameable,
                type_params: Vec::new(),
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
            types: BTreeMap::from([("payload".to_string(), "Payload".to_string())]),
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

use skiff_artifact_model::{
    BoundaryCancellationContract, BoundaryErrorContract, BoundaryStreamContract,
    BoundaryValueLifetime, BoundaryValuePlan, ContractTypeDescriptor, ContractTypeId,
    ContractTypeRef,
};

use super::*;

#[test]
fn contract_assign_validate_and_golden_identities() {
    let contract = contract_fixture();
    validate_service_contract_identities(&contract).unwrap();

    assert_eq!(
        contract.service_protocol_identity.as_str(),
        "skiff-service-protocol-v1:sha256:a3c8b78ed2018bf5b3edd8e8cd42749706e2c58f4d8035bf28828b1da4a73ffd"
    );
    assert_eq!(
        contract_operation_id("example.echo", "1.0.0", "echo")
            .unwrap()
            .as_str(),
        "skiff-contract-operation-v1:sha256:9662ad94d43bc4f9b4465193744fd1db363b3837f3d8c2a6ed57556075f04e2b"
    );
    assert_eq!(
        contract_type_id("example.echo", "1.0.0", "payload")
            .unwrap()
            .as_str(),
        "skiff-contract-type-v1:sha256:225ce35885dbb1d09d6a6a9a07626a6ca59649d42900d9890de2ecd7301000fc"
    );
}

#[test]
fn protocol_identity_includes_complete_descriptor_and_schema() {
    let base = contract_fixture();
    let baseline = service_protocol_identity(&base).unwrap();

    let mutations: Vec<fn(&mut skiff_artifact_model::ServiceContract)> = vec![
        |contract| {
            let operation = operation_mut(contract, "echo");
            let BoundaryValuePlan::Linkable { lifetime, .. } =
                &mut operation.contract.parameters[0].value_plan
            else {
                panic!("fixture linkable plan")
            };
            *lifetime = BoundaryValueLifetime::Request;
        },
        |contract| operation_mut(contract, "echo").contract.errors = BoundaryErrorContract::None,
        |contract| {
            operation_mut(contract, "echo").contract.stream = BoundaryStreamContract::Unsupported {
                reason: skiff_artifact_model::BoundaryFeatureUnavailableReason::LanguageUnsupported,
            }
        },
        |contract| {
            operation_mut(contract, "echo").contract.cancellation =
                BoundaryCancellationContract::NotCancellable
        },
        |contract| {
            operation_mut(contract, "echo").contract.callbacks =
                skiff_artifact_model::BoundaryCallbackContract::None
        },
        |contract| {
            operation_mut(contract, "echo")
                .contract
                .effect_guarantee
                .detached_return = false
        },
        |contract| {
            let schema = contract
                .boundary_schema
                .values_mut()
                .find(|schema| schema.stable_key == "payload")
                .unwrap();
            let ContractTypeDescriptor::Record { fields } = &mut schema.shape.descriptor else {
                panic!("fixture record")
            };
            fields.insert("sequence".to_string(), ContractTypeRef::builtin("number"));
        },
    ];

    for mutate in mutations {
        let mut changed = base.clone();
        mutate(&mut changed);
        assert_ne!(service_protocol_identity(&changed).unwrap(), baseline);
    }
}

#[test]
fn protocol_identity_excludes_diagnostic_text_and_map_insertion_order() {
    let base = contract_fixture();
    let baseline = service_protocol_identity(&base).unwrap();

    let mut diagnostics = base.clone();
    diagnostics.diagnostic_text.service = "entirely different prose".to_string();
    diagnostics
        .diagnostic_text
        .operations
        .values_mut()
        .for_each(|text| *text = "changed".to_string());
    assert_eq!(service_protocol_identity(&diagnostics).unwrap(), baseline);

    let mut reordered = base.clone();
    let mut operations = reordered.operations.into_iter().collect::<Vec<_>>();
    operations.reverse();
    reordered.operations = operations.into_iter().collect();
    let mut schema = reordered.boundary_schema.into_iter().collect::<Vec<_>>();
    schema.reverse();
    reordered.boundary_schema = schema.into_iter().collect();
    assert_eq!(service_protocol_identity(&reordered).unwrap(), baseline);
}

#[test]
fn operation_and_type_ids_exclude_descriptor_mutations() {
    let base = contract_fixture();
    let echo_id = operation(&base, "echo").operation_id.clone();
    let payload_id = type_id(&base, "payload");

    let mut changed = base.clone();
    operation_mut(&mut changed, "echo").contract.may_suspend = false;
    let schema = changed
        .boundary_schema
        .values_mut()
        .find(|schema| schema.stable_key == "payload")
        .unwrap();
    schema.shape.descriptor = ContractTypeDescriptor::Enumeration {
        variants: vec!["A".to_string()],
    };

    assert_eq!(operation(&changed, "echo").operation_id, echo_id);
    assert_eq!(type_id(&changed, "payload"), payload_id);
    assert_ne!(
        service_protocol_identity(&changed).unwrap(),
        service_protocol_identity(&base).unwrap()
    );
}

#[test]
fn schema_closure_and_declared_identity_fail_closed() {
    let mut contract = contract_fixture();
    operation_mut(&mut contract, "echo").contract.parameters[0].ty =
        ContractTypeRef::contract(ContractTypeId::new("skiff-contract-type-v1:sha256:missing"));
    assert!(matches!(
        service_protocol_identity(&contract),
        Err(ArtifactIdentityError::InvalidServiceContract { .. })
    ));

    let mut contract = contract_fixture();
    contract.service_protocol_identity = ServiceProtocolIdentity::new("tampered");
    assert!(matches!(
        validate_service_contract_identities(&contract),
        Err(ArtifactIdentityError::ServiceProtocolIdentityMismatch { .. })
    ));
}

fn operation<'a>(
    contract: &'a skiff_artifact_model::ServiceContract,
    stable_key: &str,
) -> &'a BoundaryOperationDescriptor {
    contract
        .operations
        .values()
        .find(|descriptor| descriptor.stable_key == stable_key)
        .unwrap()
}

fn operation_mut<'a>(
    contract: &'a mut skiff_artifact_model::ServiceContract,
    stable_key: &str,
) -> &'a mut BoundaryOperationDescriptor {
    contract
        .operations
        .values_mut()
        .find(|descriptor| descriptor.stable_key == stable_key)
        .unwrap()
}

fn type_id(contract: &skiff_artifact_model::ServiceContract, stable_key: &str) -> ContractTypeId {
    contract
        .boundary_schema
        .values()
        .find(|schema| schema.stable_key == stable_key)
        .unwrap()
        .contract_type_id
        .clone()
}

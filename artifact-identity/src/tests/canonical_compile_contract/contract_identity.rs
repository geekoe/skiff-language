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
        "skiff-service-protocol-v2:sha256:953133d6097f06a6383ad4c67d498ebb9fd7b0e1de2d72325315f2dd160832f3"
    );
    assert_eq!(
        contract_operation_id("example.echo", "1.0.0", "echo")
            .unwrap()
            .as_str(),
        "skiff-contract-operation-v1:sha256:e66aed0b87717b3767ad8bebbb8a8b53572d8966287966d5beb35934b90455d6"
    );
    assert_eq!(
        contract_type_id("example.echo", "1.0.0", "payload")
            .unwrap()
            .as_str(),
        "skiff-contract-type-v1:sha256:8b0490eda75300f7fba6df6167721059d564b7e648c2a190c4fc547ef27807fb"
    );
}

#[test]
fn human_version_label_is_not_a_service_api_identity_input() {
    let first = contract_fixture();
    let mut relabeled = first.clone();
    relabeled.contract_version = "99.4.0".to_string();
    assign_service_contract_identities(&mut relabeled).unwrap();

    assert_eq!(
        first.service_protocol_identity,
        relabeled.service_protocol_identity
    );
    assert_eq!(
        contract_operation_id("example.echo", "1.0.0", "echo").unwrap(),
        contract_operation_id("example.echo", "99.4.0", "echo").unwrap()
    );
    assert_eq!(
        contract_type_id("example.echo", "1.0.0", "payload").unwrap(),
        contract_type_id("example.echo", "99.4.0", "payload").unwrap()
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

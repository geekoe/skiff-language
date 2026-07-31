use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCallbackExpirationError, BoundaryCallbackLifetime,
    BoundaryEffectGuarantee, BoundaryFeatureUnavailableReason, BoundaryOperationContract,
    BoundaryParameter, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
    ContractDiagnosticText, ContractTypeDescriptor, PackageSchemaIndexEntry, PackageSchemaTypeRef,
    PackageTypeRequirement,
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
        index_entry(package_schema_type_id("example.pkg", "Unused", &descriptor("bool")).unwrap()),
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
        item_value_plan: value_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream),
    };
    mutations.push(streamed);

    let mut callback = base.clone();
    let callback_operation = &mut callback.operations.get_mut(&operation_id).unwrap().contract;
    callback_operation.parameters[0].ty = ContractTypeRef::AnyInterface {
        interface: Box::new(ContractTypeRef::package_schema(
            "example.pkg",
            "User",
            type_id.clone(),
        )),
        arguments: Vec::new(),
    };
    callback_operation.parameters[0].value_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::CallbackCapability,
        encoding: BoundaryValueEncoding::OpaqueCapability,
        owner: BoundaryValueOwner::CapabilityOwner,
        lifetime: BoundaryValueLifetime::Request,
    };
    callback_operation.callbacks = BoundaryCallbackContract::RequestScoped {
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
        item_value_plan: value_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Stream),
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
                    4 => {
                        set_plan_carrier(item_value_plan, BoundaryValueCarrier::CallbackCapability)
                    }
                    5 => {
                        set_plan_encoding(item_value_plan, BoundaryValueEncoding::OpaqueCapability)
                    }
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
    stale_identity.service_protocol_identity =
        ServiceProtocolIdentity::new(stale_identity.service_protocol_identity.as_str().replacen(
            SERVICE_PROTOCOL_IDENTITY_PREFIX,
            "skiff-service-protocol-v4:sha256",
            1,
        ));
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
                value_plan: value_plan(BoundaryValueOwner::Provider, BoundaryValueLifetime::Call),
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
mod suspension;

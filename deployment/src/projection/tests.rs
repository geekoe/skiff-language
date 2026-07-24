use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    assign_package_artifact_identities, assign_service_contract_identities, contract_operation_id,
    package_schema_index_identity, package_schema_type_id,
};
use skiff_artifact_model::*;

use super::*;

mod eligibility;

struct ProjectionFixture {
    input: ServiceDeploymentInput,
    contract: ServiceContract,
    implementation: PackageArtifact,
    callable_id: PackageCallableId,
    package_schema_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
}

impl ProjectionFixture {
    fn new() -> Self {
        let service_id = "example.echo";
        let contract_version = "1.0.0";
        let package_id = "example.provider";
        let payload_descriptor = PackageSchemaCanonicalDescriptor {
            type_params: Vec::new(),
            descriptor: ContractTypeDescriptor::Record {
                fields: BTreeMap::from([(
                    "message".to_string(),
                    ContractTypeRef::builtin("string"),
                )]),
            },
        };
        let payload_id =
            package_schema_type_id(package_id, "payload", &payload_descriptor).unwrap();
        let payload_ref =
            ContractTypeRef::package_schema(package_id, "payload", payload_id.clone());
        let echo_id = contract_operation_id(service_id, contract_version, "echo").unwrap();
        let repeat_id = contract_operation_id(service_id, contract_version, "repeat").unwrap();
        let operation_contract = operation_contract(payload_ref.clone());
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: service_id.to_string(),
            contract_version: contract_version.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([
                (
                    echo_id.clone(),
                    BoundaryOperationDescriptor {
                        operation_id: echo_id.clone(),
                        stable_key: "echo".to_string(),
                        contract: operation_contract.clone(),
                    },
                ),
                (
                    repeat_id.clone(),
                    BoundaryOperationDescriptor {
                        operation_id: repeat_id.clone(),
                        stable_key: "repeat".to_string(),
                        contract: operation_contract.clone(),
                    },
                ),
            ]),
            package_type_requirements: vec![PackageTypeRequirement {
                package_id: package_id.to_string(),
                required_type_ids: vec![payload_id.clone()],
            }],
            diagnostic_text: ContractDiagnosticText {
                service: "Echo".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        };
        assign_service_contract_identities(&mut contract).unwrap();

        let callable_id = PackageCallableId::new("callable:handle");
        let facts = safe_facts();
        let requirements = BoundaryImplementationRequirements {
            config: vec![BoundaryConfigRequirement {
                path: "echo.token".to_string(),
                value_type: "string".to_string(),
                required: true,
            }],
            state: vec![
                BoundaryStateRequirement {
                    key: "echo-state".to_string(),
                    kind: BoundaryStateKind::Database,
                },
                BoundaryStateRequirement {
                    key: "echo-db".to_string(),
                    kind: BoundaryStateKind::ExternalResource,
                },
            ],
            native_capabilities: Vec::new(),
            runtime_capabilities: vec!["async".to_string()],
            complete_may_effects: no_effects(),
            provenance: facts.provenance.clone(),
        };
        let file = FileIrRef::new("file-ir", "provider.main");
        let target = OperationTargetRef {
            file_ref: file.clone(),
            executable_index: 0,
            callable_abi_id: callable_id.to_string(),
            callable_kind: OperationCallableKind::PublicFunction,
        };
        let mut implementation = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "2.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: vec![file],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::from([(
                    "handle".to_string(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature: PackageCallableSignature {
                            parameters: vec![PackageCallableParameter {
                                name: "input".to_string(),
                                ty: PackageTypeRef::PackageSchema {
                                    package_id: package_id.to_string(),
                                    stable_schema_key: "payload".to_string(),
                                    package_schema_type_id: payload_id.clone(),
                                },
                            }],
                            return_type: PackageTypeRef::PackageSchema {
                                package_id: package_id.to_string(),
                                stable_schema_key: "payload".to_string(),
                                package_schema_type_id: payload_id.clone(),
                            },
                            throw_types: Vec::new(),
                            may_suspend: true,
                        },
                    },
                )]),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity: package_schema_index_identity(
                    package_id,
                    &BTreeMap::from([(
                        "payload".to_string(),
                        PackageSchemaIndexEntry {
                            package_schema_type_id: payload_id.clone(),
                            public_path: Some("payload".to_string()),
                            nameability: ContractTypeNameability::PublicNameable,
                        },
                    )]),
                )
                .unwrap(),
            },
            package_schema_type_records: BTreeMap::from([(
                payload_id.clone(),
                PackageSchemaTypeRecordRef {
                    package_id: package_id.to_string(),
                    package_schema_type_id: payload_id.clone(),
                },
            )]),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::from([(
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target,
                },
            )]),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: vec![PackageConfigRequirement {
                    path: "echo.token".to_string(),
                    value_type: "string".to_string(),
                    required: true,
                }],
                resources: vec![PackageResourceRequirement {
                    key: "echo-db".to_string(),
                    capability: "mongodb".to_string(),
                }],
                runtime_capabilities: vec![PackageRuntimeCapabilityRequirement {
                    capability: "async".to_string(),
                    required_version: "1".to_string(),
                }],
            },
            callable_semantic_facts: BTreeMap::from([(callable_id.clone(), facts)]),
            boundary_projections: BTreeMap::from([(
                callable_id.clone(),
                BoundaryCallableProjection::Available {
                    operation_contract,
                    implementation_requirements: requirements,
                },
            )]),
            service_call_refs: Vec::new(),
        };
        assign_package_artifact_identities(&mut implementation).unwrap();

        let implementation_ref = package_ref(&implementation);
        let input = ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: contract_ref(&contract),
            deployment_revision: DeploymentRevision::new("revision-1"),
            implementation: implementation_ref,
            operation_bindings: vec![
                ServiceDeploymentOperationInput {
                    contract_operation_id: repeat_id,
                    package_public_path: "handle".to_string(),
                },
                ServiceDeploymentOperationInput {
                    contract_operation_id: echo_id,
                    package_public_path: "handle".to_string(),
                },
            ],
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            ingress: Vec::new(),
            config_literals: vec![ConfigLiteralBinding {
                path: "echo.token".to_string(),
                value: MetadataValue::String("public-value".to_string()),
            }],
            secret_refs: Vec::new(),
            state_bindings: vec![StateBinding {
                requirement_key: "echo-state".to_string(),
                kind: StateBindingKind::Database,
                namespace: "echo".to_string(),
            }],
            resource_bindings: vec![ResourceBinding {
                requirement_key: "echo-db".to_string(),
                capability: "mongodb".to_string(),
                resource_ref: "resource:echo".to_string(),
            }],
            runtime_capability_bindings: vec![RuntimeCapabilityBinding {
                capability: "async".to_string(),
                version: "1".to_string(),
            }],
            policy: DeploymentPolicy {
                timeout_ms: 1_000,
                resources: ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_048_576,
                },
                activation: ActivationPolicy {
                    max_concurrency: 4,
                    idle_timeout_ms: None,
                },
                principal: "service:example.echo".to_string(),
            },
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "Echo deployment".to_string(),
                notes: BTreeMap::new(),
            },
        };
        let package_schema_records = BTreeMap::from([(
            payload_id.clone(),
            PackageSchemaTypeRecord {
                package_id: package_id.to_string(),
                stable_schema_key: "payload".to_string(),
                package_schema_type_id: payload_id,
                canonical_descriptor: payload_descriptor,
            },
        )]);
        Self {
            input,
            contract,
            implementation,
            callable_id,
            package_schema_records,
        }
    }

    fn project(&self) -> ProjectionResult<ServiceDeployment> {
        project_service_deployment(
            self.input.clone(),
            &self.contract,
            std::slice::from_ref(&self.implementation),
            &self.package_schema_records,
        )
    }

    fn refresh_implementation_ref(&mut self) {
        assign_package_artifact_identities(&mut self.implementation).unwrap();
        self.input.implementation = package_ref(&self.implementation);
    }
}

#[test]
fn projection_maps_every_operation_explicitly_and_emits_no_public_path() {
    let fixture = ProjectionFixture::new();
    let deployment = fixture.project().unwrap();
    assert_eq!(deployment.operation_bindings.len(), 2);
    assert!(deployment
        .operation_bindings
        .iter()
        .all(|binding| binding.package_callable_id == fixture.callable_id));
    assert!(deployment
        .operation_bindings
        .windows(2)
        .all(|pair| { pair[0].contract_operation_id < pair[1].contract_operation_id }));
    let wire = serde_json::to_string(&deployment).unwrap();
    assert!(!wire.contains("packagePublicPath"));
    skiff_artifact_identity::validate_service_deployment_identity(&deployment).unwrap();
}

#[test]
fn websocket_ingress_contract_validation_accepts_only_the_unified_abi() {
    let mut fixture = ProjectionFixture::new();
    let operation_id = fixture
        .contract
        .operations
        .keys()
        .next()
        .expect("fixture operation")
        .clone();
    fixture.input.ingress = vec![DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            host: "socket.example.test".to_string(),
            method: None,
            path: "/socket".to_string(),
        },
        contract_operation_id: operation_id.clone(),
    }];
    assert!(matches!(
        validate_ingress_contracts(
            &fixture.input,
            &fixture.contract,
            &fixture.package_schema_records
        ),
        Err(ProjectionError::InvalidWebSocketIngressContract { .. })
    ));

    let context = fixture
        .package_schema_records
        .keys()
        .next()
        .expect("fixture context type")
        .clone();
    let context_ref = package_context_ref(&fixture, &context);
    let descriptor = fixture.contract.operations.get_mut(&operation_id).unwrap();
    descriptor.stable_key = "websocket".to_string();
    descriptor.contract = websocket_ingress_operation(context_ref);
    validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records,
    )
    .expect("exact unified WebSocket ABI should pass deployment projection validation");

    let baseline = fixture.contract.operations[&operation_id].contract.clone();
    let mut invalid = baseline.clone();
    invalid.parameters[0].name = "request".to_string();
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline.clone();
    invalid.may_suspend = true;
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline.clone();
    invalid.parameters.push(invalid.parameters[0].clone());
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline.clone();
    invalid.errors = BoundaryErrorContract::Typed {
        payload_type: ContractTypeRef::builtin("string"),
        value_plan: linkable_plan(BoundaryValueOwner::Provider),
    };
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline.clone();
    invalid.parameters[0].ty = ContractTypeRef::Builtin {
        name: WEBSOCKET_INGRESS_EVENT_TYPE.to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline.clone();
    invalid.return_value.ty = ContractTypeRef::builtin("null");
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());

    let mut invalid = baseline;
    invalid.stream = BoundaryStreamContract::ServerStream {
        item_type: ContractTypeRef::builtin("string"),
        item_value_plan: linkable_plan(BoundaryValueOwner::Provider),
    };
    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = invalid;
    assert!(validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records
    )
    .is_err());
}

#[test]
fn websocket_ingress_admission_rejects_non_persistable_context_graphs() {
    let mut fixture = ProjectionFixture::new();
    let operation_id = fixture
        .contract
        .operations
        .keys()
        .next()
        .expect("fixture operation")
        .clone();
    fixture.input.ingress = vec![DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::WebSocket,
            host: "socket.example.test".to_string(),
            method: None,
            path: "/socket".to_string(),
        },
        contract_operation_id: operation_id.clone(),
    }];
    let context_id = fixture
        .package_schema_records
        .keys()
        .next()
        .expect("fixture context type")
        .clone();
    let descriptor = fixture.contract.operations.get_mut(&operation_id).unwrap();
    descriptor.stable_key = WEBSOCKET_INGRESS_OPERATION_NAME.to_string();
    descriptor.contract = websocket_ingress_operation(ContractTypeRef::builtin("null"));
    validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records,
    )
    .expect("null Context must pass deployment admission");

    fixture
        .contract
        .operations
        .get_mut(&operation_id)
        .unwrap()
        .contract = websocket_ingress_operation(package_context_ref(&fixture, &context_id));
    validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records,
    )
    .expect("persistable nominal Context must pass deployment admission");

    fixture
        .package_schema_records
        .get_mut(&context_id)
        .unwrap()
        .canonical_descriptor
        .descriptor = ContractTypeDescriptor::CallbackInterface {
        operations: BTreeMap::new(),
    };
    assert_websocket_admission_error_contains(&fixture, "CallbackInterface");

    fixture
        .package_schema_records
        .get_mut(&context_id)
        .unwrap()
        .canonical_descriptor
        .descriptor = ContractTypeDescriptor::Record {
        fields: BTreeMap::from([(
            "self".to_string(),
            package_context_ref(&fixture, &context_id),
        )]),
    };
    assert_websocket_admission_error_contains(&fixture, "package schema cycle");

    let missing_id = PackageSchemaTypeId::new("type:foreign-or-missing");
    fixture
        .package_schema_records
        .get_mut(&context_id)
        .unwrap()
        .canonical_descriptor
        .descriptor = ContractTypeDescriptor::Record {
        fields: BTreeMap::from([(
            "foreign".to_string(),
            ContractTypeRef::package_schema("example.provider", "missing", missing_id),
        )]),
    };
    assert_websocket_admission_error_contains(&fixture, "outside ServiceContract requirements");
}

fn assert_websocket_admission_error_contains(fixture: &ProjectionFixture, expected: &str) {
    let error = validate_ingress_contracts(
        &fixture.input,
        &fixture.contract,
        &fixture.package_schema_records,
    )
    .expect_err("invalid WebSocket Context must fail deployment admission");
    let ProjectionError::InvalidWebSocketIngressContract { message, .. } = error else {
        panic!("expected WebSocket ingress admission error, got {error:?}")
    };
    assert!(
        message.contains(expected),
        "expected `{expected}` in `{message}`"
    );
}

fn package_context_ref(
    fixture: &ProjectionFixture,
    type_id: &PackageSchemaTypeId,
) -> ContractTypeRef {
    let record = &fixture.package_schema_records[type_id];
    ContractTypeRef::package_schema(
        record.package_id.clone(),
        record.stable_schema_key.clone(),
        type_id.clone(),
    )
}

#[test]
fn operation_mapping_failures_are_structured_and_fail_closed() {
    let mut fixture = ProjectionFixture::new();
    fixture.input.operation_bindings.pop();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::MissingOperationBinding { .. })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture
        .input
        .operation_bindings
        .push(fixture.input.operation_bindings[0].clone());
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::DuplicateOperationBinding { .. })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.operation_bindings[0].contract_operation_id = ContractOperationId::new("unknown");
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::UnknownOperationBinding { .. })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.operation_bindings[0].package_public_path = "missing".to_string();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::UnknownPublicPath { .. })
    ));
}

#[test]
fn unavailable_callable_and_nominal_descriptor_mismatch_fail_closed() {
    let mut fixture = ProjectionFixture::new();
    fixture.implementation.boundary_projections.insert(
        fixture.callable_id.clone(),
        BoundaryCallableProjection::Unavailable {
            reasons: vec![BoundaryUnavailableReason::UnknownEffect],
        },
    );
    fixture.refresh_implementation_ref();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::BoundaryUnavailable { .. })
    ));

    let mut fixture = ProjectionFixture::new();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = fixture
        .implementation
        .boundary_projections
        .get_mut(&fixture.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    operation_contract.return_value.ty = ContractTypeRef::Record {
        fields: BTreeMap::from([("message".to_string(), ContractTypeRef::builtin("string"))]),
    };
    fixture.refresh_implementation_ref();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::OperationContractMismatch { .. })
    ));
}

#[test]
fn package_owned_operation_requires_exact_owner_key_and_type_id() {
    ProjectionFixture::new()
        .project()
        .expect("the exact Package-owned operation must pass");

    for replacement in [
        ContractTypeRef::package_schema(
            "example.foreign",
            "payload",
            ProjectionFixture::new()
                .package_schema_records
                .keys()
                .next()
                .unwrap()
                .clone(),
        ),
        ContractTypeRef::package_schema(
            "example.provider",
            "other",
            ProjectionFixture::new()
                .package_schema_records
                .keys()
                .next()
                .unwrap()
                .clone(),
        ),
        ContractTypeRef::package_schema(
            "example.provider",
            "payload",
            PackageSchemaTypeId::new("package-schema-type:wrong"),
        ),
    ] {
        let mut fixture = ProjectionFixture::new();
        let BoundaryCallableProjection::Available {
            operation_contract, ..
        } = fixture
            .implementation
            .boundary_projections
            .get_mut(&fixture.callable_id)
            .unwrap()
        else {
            unreachable!()
        };
        operation_contract.parameters[0].ty = replacement;
        fixture.refresh_implementation_ref();
        assert!(matches!(
            fixture.project(),
            Err(ProjectionError::OperationContractMismatch { .. })
        ));
    }
}

#[test]
fn deployment_requires_the_exact_validated_package_schema_closure() {
    let mut missing = ProjectionFixture::new();
    missing.package_schema_records.clear();
    assert!(matches!(
        missing.project(),
        Err(ProjectionError::PackageSchemaClosureMismatch { .. })
    ));

    let mut extra = ProjectionFixture::new();
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Enumeration {
            variants: vec!["extra".to_string()],
        },
    };
    let extra_id =
        package_schema_type_id("example.provider", "extra", &descriptor).expect("extra type id");
    extra.package_schema_records.insert(
        extra_id.clone(),
        PackageSchemaTypeRecord {
            package_id: "example.provider".to_string(),
            stable_schema_key: "extra".to_string(),
            package_schema_type_id: extra_id,
            canonical_descriptor: descriptor,
        },
    );
    assert!(matches!(
        extra.project(),
        Err(ProjectionError::PackageSchemaClosureMismatch { .. })
    ));

    let mut foreign_owner = ProjectionFixture::new();
    foreign_owner
        .package_schema_records
        .values_mut()
        .next()
        .unwrap()
        .package_id = "example.foreign".to_string();
    assert!(matches!(
        foreign_owner.project(),
        Err(ProjectionError::InvalidTypedArtifact {
            artifact: "PackageSchemaTypeRecord closure",
            ..
        })
    ));

    let mut bad_hash = ProjectionFixture::new();
    bad_hash
        .package_schema_records
        .values_mut()
        .next()
        .unwrap()
        .stable_schema_key = "tampered".to_string();
    assert!(matches!(
        bad_hash.project(),
        Err(ProjectionError::InvalidTypedArtifact {
            artifact: "PackageSchemaTypeRecord closure",
            ..
        })
    ));
}

#[test]
fn required_activation_bindings_and_protocol_identity_are_exact() {
    let mut fixture = ProjectionFixture::new();
    fixture.input.config_literals.clear();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::MissingRequirementBinding { kind: "config", .. })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.state_bindings.clear();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::MissingRequirementBinding { kind: "state", .. })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.resource_bindings.clear();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::MissingRequirementBinding {
            kind: "resource",
            ..
        })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.runtime_capability_bindings.clear();
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::MissingRequirementBinding {
            kind: "runtime capability",
            ..
        })
    ));

    let mut fixture = ProjectionFixture::new();
    fixture.input.contract.service_protocol_identity = ServiceProtocolIdentity::new("wrong");
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::ContractReferenceMismatch {
            field: "serviceProtocolIdentity",
            ..
        })
    ));
}

#[test]
fn secret_refs_remain_opaque_and_are_distinct_from_literals() {
    let fixture = ProjectionFixture::new();
    let literal = fixture.project().unwrap();

    let mut secret_fixture = ProjectionFixture::new();
    secret_fixture.input.config_literals.clear();
    secret_fixture.input.secret_refs = vec![SecretRefBinding {
        path: "echo.token".to_string(),
        secret_ref: "vault:echo/token".to_string(),
    }];
    let secret = secret_fixture.project().unwrap();
    assert_ne!(
        literal.deployment_artifact_identity,
        secret.deployment_artifact_identity
    );
    let wire = serde_json::to_string(&secret).unwrap();
    assert!(wire.contains("vault:echo/token"));
    assert!(!wire.contains("resolvedSecret"));
}

#[test]
fn service_dependencies_keep_only_exact_contract_selectors_by_caller_slot() {
    let mut fixture = ProjectionFixture::new();
    let dependency_operation = ContractOperationId::new("operation:payments.charge");
    let dependency = ContractRequirement {
        alias: "payments".to_string(),
        service_id: "example.payments".to_string(),
        contract_version: "3.0.0".to_string(),
        expected_protocol_identity: ServiceProtocolIdentity::new("protocol:payments-v3"),
    };
    fixture.implementation.contract_requirements = vec![dependency.clone()];
    fixture.implementation.service_requirements = vec![ServiceRequirement {
        contract_requirement: dependency.clone(),
        service_binding_slot: 7,
        used_operations: BTreeSet::from([dependency_operation.clone()]),
    }];
    fixture.implementation.service_call_refs = vec![ServiceCallRef {
        service_requirement_slot: 7,
        contract_operation_id: dependency_operation,
        expected_protocol_identity: dependency.expected_protocol_identity.clone(),
    }];
    fixture.refresh_implementation_ref();
    fixture.input.service_selectors = vec![ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: fixture.implementation.package_build_id.clone(),
            service_requirement_slot: 7,
        },
        contract: ServiceContractRef {
            service_id: dependency.service_id,
            contract_version: dependency.contract_version,
            service_protocol_identity: dependency.expected_protocol_identity,
        },
    }];
    let deployment = fixture.project().unwrap();
    assert_eq!(deployment.service_selectors.len(), 1);

    fixture.input.service_selectors[0]
        .contract
        .service_protocol_identity = ServiceProtocolIdentity::new("wrong");
    assert!(matches!(
        fixture.project(),
        Err(ProjectionError::RequirementBindingMismatch {
            kind: "service selector",
            ..
        })
    ));
}

#[test]
fn exact_package_closure_is_required_and_binding_changes_identity() {
    let mut fixture = ProjectionFixture::new();
    let dependency_a = dependency_artifact("resource-a");
    fixture.implementation.package_requirements = vec![PackageRequirement {
        alias: "util".to_string(),
        package_id: dependency_a.package_id.clone(),
        exact_version: dependency_a.package_version.clone(),
        expected_local_abi: dependency_a.package_local_abi.local_abi_identity.clone(),
    }];
    fixture.refresh_implementation_ref();
    let binding_key = PackageRequirementKey {
        caller_package_build_id: fixture.implementation.package_build_id.clone(),
        package_requirement_alias: "util".to_string(),
    };
    fixture.input.package_bindings = vec![PackageBinding {
        key: binding_key,
        package: package_ref(&dependency_a),
    }];

    assert!(matches!(
        project_service_deployment(
            fixture.input.clone(),
            &fixture.contract,
            std::slice::from_ref(&fixture.implementation),
            &fixture.package_schema_records,
        ),
        Err(ProjectionError::MissingRequirementBinding {
            kind: "package artifact",
            ..
        })
    ));

    let first = project_service_deployment(
        fixture.input.clone(),
        &fixture.contract,
        &[fixture.implementation.clone(), dependency_a],
        &fixture.package_schema_records,
    )
    .unwrap();
    let dependency_b = dependency_artifact("resource-b");
    fixture.input.package_bindings[0].package = package_ref(&dependency_b);
    let second = project_service_deployment(
        fixture.input,
        &fixture.contract,
        &[fixture.implementation, dependency_b],
        &fixture.package_schema_records,
    )
    .unwrap();
    assert_ne!(
        first.deployment_artifact_identity,
        second.deployment_artifact_identity
    );
}

#[test]
fn transitive_requirement_cannot_fill_an_invalid_callable_projection() {
    let mut fixture = ProjectionFixture::new();
    let mut dependency = dependency_artifact("resource");
    dependency.runtime_requirements.config = vec![PackageConfigRequirement {
        path: "dependency.only".to_string(),
        value_type: "string".to_string(),
        required: true,
    }];
    assign_package_artifact_identities(&mut dependency).unwrap();
    fixture.implementation.package_requirements = vec![PackageRequirement {
        alias: "util".to_string(),
        package_id: dependency.package_id.clone(),
        exact_version: dependency.package_version.clone(),
        expected_local_abi: dependency.package_local_abi.local_abi_identity.clone(),
    }];
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = fixture
        .implementation
        .boundary_projections
        .get_mut(&fixture.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    implementation_requirements
        .config
        .push(BoundaryConfigRequirement {
            path: "dependency.only".to_string(),
            value_type: "string".to_string(),
            required: true,
        });
    fixture.refresh_implementation_ref();
    fixture.input.package_bindings = vec![PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: fixture.implementation.package_build_id.clone(),
            package_requirement_alias: "util".to_string(),
        },
        package: package_ref(&dependency),
    }];
    fixture.input.config_literals.push(ConfigLiteralBinding {
        path: "dependency.only".to_string(),
        value: MetadataValue::String("value".to_string()),
    });

    assert!(matches!(
        project_service_deployment(
            fixture.input,
            &fixture.contract,
            &[fixture.implementation, dependency],
            &fixture.package_schema_records,
        ),
        Err(ProjectionError::CallableFactsMismatch { .. })
    ));
}

fn dependency_artifact(resource_hash: &str) -> PackageArtifact {
    let mut artifact = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.util".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: vec![PublicationResourceRef {
            path: "data.txt".to_string(),
            sha256: resource_hash.to_string(),
            byte_len: 1,
            content_type: None,
            artifact_path: None,
        }],
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.util".to_string(),
            package_schema_index_identity: package_schema_index_identity(
                "example.util",
                &BTreeMap::new(),
            )
            .unwrap(),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    };
    assign_package_artifact_identities(&mut artifact).unwrap();
    artifact
}

fn package_ref(artifact: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn websocket_ingress_operation(context: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "event".to_string(),
            ty: ContractTypeRef::Builtin {
                name: WEBSOCKET_INGRESS_EVENT_TYPE.to_string(),
                arguments: vec![context.clone()],
            },
            value_plan: linkable_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::Nullable {
                inner: Box::new(ContractTypeRef::Builtin {
                    name: WEBSOCKET_CONNECT_RESULT_TYPE.to_string(),
                    arguments: vec![context],
                }),
            },
            value_plan: linkable_plan(BoundaryValueOwner::Provider),
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

fn operation_contract(payload_type: ContractTypeRef) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "input".to_string(),
            ty: payload_type.clone(),
            value_plan: linkable_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: payload_type,
            value_plan: linkable_plan(BoundaryValueOwner::Provider),
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
    }
}

fn linkable_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    }
}

fn safe_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: no_effects(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: true,
    }
}

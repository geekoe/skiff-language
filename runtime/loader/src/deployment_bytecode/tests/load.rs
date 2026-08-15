use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryDropPlan, BoundaryEffectGuarantee, BoundaryErrorAdmission,
    BoundaryErrorFallbackIdentity, BoundaryErrorPlan, BoundaryErrorPolicy,
    BoundaryOperationContract, BoundaryOperationDescriptor, BoundaryReturn, BoundaryStreamContract,
    BoundaryTransfer, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, BytecodePoolEntry, BytecodeRelocation,
    ContractOperationId, ContractRequirement, ContractTypeDescriptor, ContractTypeRef,
    DeploymentOperationBinding, PackageCallableId, PackageExecutableCoordinate, PackageRequirement,
    PackageSchemaCanonicalDescriptor, PackageSchemaIndexEntry, PackageSchemaTypeRecord,
    ResumeDescriptor, ResumeErrorMode, ServiceCallBoundaryFacts, ServiceCallRef,
    ServiceCallbackPlan, ServiceRequirement, ValueProvenance,
};

use super::*;

struct InMemoryResolver {
    deployment_reference: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    bytecodes: BTreeMap<(PackageArtifactRef, String), Arc<ValidatedBytecodeArtifact>>,
    deployment_calls: Cell<usize>,
    unexpected_deployment_calls: Cell<usize>,
}

impl DeploymentBytecodeContentResolver for InMemoryResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployment_calls.set(self.deployment_calls.get() + 1);
        if reference != &self.deployment_reference {
            self.unexpected_deployment_calls
                .set(self.unexpected_deployment_calls.get() + 1);
            anyhow::bail!("provider deployment resolution is forbidden")
        }
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing contract {reference:?}"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package {reference:?}"))
    }

    fn resolve_package_bytecode(
        &self,
        package: &PackageArtifactRef,
        reference: &BytecodeArtifactRef,
    ) -> anyhow::Result<Arc<ValidatedBytecodeArtifact>> {
        self.bytecodes
            .get(&(package.clone(), reference.bytecode_identity.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing bytecode for {package:?} {reference:?}"))
    }
}

struct LoadFixture {
    resolver: InMemoryResolver,
    deployment_reference: ServiceDeploymentRef,
    implementation_reference: PackageArtifactRef,
    dependency_reference: PackageArtifactRef,
    dependency_contract: ServiceContractRef,
    dependency_operation: ContractOperationId,
}

impl LoadFixture {
    fn new(implementation_has_bytecode: bool, dependency_contract_has_operation: bool) -> Self {
        let (own_contract, own_contract_reference, _) = valid_contract("example.consumer", false);
        let (dependency_contract, dependency_contract_reference, declared_operation) =
            valid_contract("example.provider", dependency_contract_has_operation);
        let dependency_operation = declared_operation.unwrap_or_else(|| {
            skiff_artifact_identity::contract_operation_id("example.provider", "1.0.0", "call")
                .unwrap()
        });
        let implementation_bytecode =
            implementation_has_bytecode.then(|| admitted_bytecode("implementation"));
        let dependency_bytecode = admitted_bytecode("dependency");
        let dependency_package = valid_package(
            "example.dependency",
            Some(dependency_bytecode.reference().clone()),
            Vec::new(),
            None,
        );
        let dependency_reference = package_reference(&dependency_package);
        let contract_requirement = ContractRequirement {
            alias: "provider".to_string(),
            service_id: dependency_contract_reference.service_id.clone(),
            contract_version: dependency_contract_reference.contract_version.clone(),
            expected_protocol_identity: dependency_contract_reference
                .service_protocol_identity
                .clone(),
        };
        let implementation_package = valid_package(
            "example.implementation",
            implementation_bytecode
                .as_ref()
                .map(|bytecode| bytecode.reference().clone()),
            vec![PackageRequirement {
                alias: "dependency".to_string(),
                package_id: dependency_reference.package_id.clone(),
                exact_version: dependency_reference.package_version.clone(),
                expected_local_abi: dependency_reference.package_local_abi_identity.clone(),
                expected_package_build: None,
            }],
            Some((contract_requirement, dependency_operation.clone())),
        );
        let implementation_reference = package_reference(&implementation_package);
        let service_key = ServiceRequirementKey {
            caller_package_build_id: implementation_reference.package_build_id.clone(),
            service_requirement_slot: 3,
        };
        let package_key = PackageRequirementKey {
            caller_package_build_id: implementation_reference.package_build_id.clone(),
            package_requirement_alias: "dependency".to_string(),
        };
        let mut deployment = deployment(
            implementation_reference.clone(),
            own_contract_reference.clone(),
            vec![ServiceSelectorBinding {
                key: service_key,
                contract: dependency_contract_reference.clone(),
            }],
        )
        .as_ref()
        .clone();
        deployment.package_bindings = vec![skiff_artifact_model::PackageBinding {
            key: package_key,
            package: dependency_reference.clone(),
        }];
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment_reference = deployment_reference(&deployment);

        let packages = BTreeMap::from([
            (implementation_reference.clone(), implementation_package),
            (dependency_reference.clone(), dependency_package),
        ]);
        let contracts = BTreeMap::from([
            (own_contract_reference, own_contract),
            (dependency_contract_reference.clone(), dependency_contract),
        ]);
        let mut bytecodes = BTreeMap::from([(
            (
                dependency_reference.clone(),
                dependency_bytecode.reference().bytecode_identity.clone(),
            ),
            dependency_bytecode,
        )]);
        if let Some(bytecode) = implementation_bytecode {
            bytecodes.insert(
                (
                    implementation_reference.clone(),
                    bytecode.reference().bytecode_identity.clone(),
                ),
                bytecode,
            );
        }
        let resolver = InMemoryResolver {
            deployment_reference: deployment_reference.clone(),
            deployment: Arc::new(deployment),
            contracts,
            packages,
            bytecodes,
            deployment_calls: Cell::new(0),
            unexpected_deployment_calls: Cell::new(0),
        };
        Self {
            resolver,
            deployment_reference,
            implementation_reference,
            dependency_reference,
            dependency_contract: dependency_contract_reference,
            dependency_operation,
        }
    }
}

fn valid_contract(
    service_id: &str,
    with_operation: bool,
) -> (
    Arc<ServiceContract>,
    ServiceContractRef,
    Option<ContractOperationId>,
) {
    let placeholder = contract_reference(service_id);
    let mut contract = contract(&placeholder).as_ref().clone();
    let operation = if with_operation {
        let operation = skiff_artifact_identity::contract_operation_id(
            service_id,
            &placeholder.contract_version,
            "call",
        )
        .unwrap();
        contract.operations.insert(
            operation.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation.clone(),
                stable_key: "call".to_string(),
                contract: boundary_operation_contract(),
            },
        );
        Some(operation)
    } else {
        None
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    let reference = skiff_artifact_identity::service_contract_ref(&contract).unwrap();
    (Arc::new(contract), reference, operation)
}

fn boundary_operation_contract() -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("bool"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
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
    }
}

fn valid_package(
    package_id: &str,
    bytecode: Option<BytecodeArtifactRef>,
    package_requirements: Vec<PackageRequirement>,
    service_dependency: Option<(ContractRequirement, ContractOperationId)>,
) -> Arc<PackageArtifact> {
    let mut artifact = package_artifact(package_id, "unassigned", bytecode)
        .as_ref()
        .clone();
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            package_id,
            &BTreeMap::<String, PackageSchemaIndexEntry>::new(),
        )
        .unwrap();
    artifact.package_requirements = package_requirements;
    if let Some((contract_requirement, operation)) = service_dependency {
        artifact.contract_requirements = vec![contract_requirement.clone()];
        artifact.service_requirements = vec![ServiceRequirement {
            contract_requirement: contract_requirement.clone(),
            service_binding_slot: 3,
            used_operations: BTreeSet::from([operation.clone()]),
        }];
        artifact.service_call_refs = vec![ServiceCallRef {
            service_requirement_slot: 3,
            contract_operation_id: operation,
            expected_protocol_identity: contract_requirement.expected_protocol_identity,
        }];
    }
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    Arc::new(artifact)
}

#[test]
fn load_hydrates_exact_consumer_closure_and_symbolic_service_rows() {
    let fixture = LoadFixture::new(true, true);
    let hydrated = DeploymentBytecodeLoader::new(&fixture.resolver)
        .load(&fixture.deployment_reference)
        .unwrap();

    assert_eq!(hydrated.reference(), &fixture.deployment_reference);
    assert_eq!(hydrated.packages().len(), 2);
    assert!(hydrated
        .packages()
        .contains_key(&fixture.implementation_reference.package_build_id));
    assert!(hydrated
        .packages()
        .contains_key(&fixture.dependency_reference.package_build_id));
    let dependency = hydrated.service_dependencies().values().next().unwrap();
    assert_eq!(dependency.contract(), &fixture.dependency_contract);
    assert!(dependency
        .used_operations()
        .contains(&fixture.dependency_operation));
    assert_eq!(fixture.resolver.deployment_calls.get(), 1);
    assert_eq!(fixture.resolver.unexpected_deployment_calls.get(), 0);
}

#[test]
fn load_rejects_missing_bytecode_before_publication() {
    let fixture = LoadFixture::new(false, true);
    assert!(matches!(
        DeploymentBytecodeLoader::new(&fixture.resolver).load(&fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::MissingBytecode { package })
            if package.as_ref() == &fixture.implementation_reference
    ));

    let mut fixture = LoadFixture::new(true, true);
    let implementation = fixture.implementation_reference.clone();
    fixture
        .resolver
        .bytecodes
        .retain(|(package, _), _| package != &implementation);
    assert!(matches!(
        DeploymentBytecodeLoader::new(&fixture.resolver).load(&fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ContentResolution {
            reference,
            ..
        }) if matches!(
            reference.as_ref(),
            DeploymentBytecodeReference::PackageBytecode { package, .. }
                if package == &fixture.implementation_reference
        )
    ));
}

#[test]
fn load_rejects_package_and_bytecode_reference_tampering() {
    let mut package_fixture = LoadFixture::new(true, true);
    let replacement_bytecode = admitted_bytecode("replacement-package");
    let replacement = valid_package(
        "example.replacement",
        Some(replacement_bytecode.reference().clone()),
        Vec::new(),
        None,
    );
    package_fixture.resolver.packages.insert(
        package_fixture.implementation_reference.clone(),
        replacement,
    );
    assert!(matches!(
        DeploymentBytecodeLoader::new(&package_fixture.resolver)
            .load(&package_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
            expected,
            actual,
        }) if matches!(expected.as_ref(), DeploymentBytecodeReference::Package(_))
            && matches!(actual.as_ref(), DeploymentBytecodeReference::Package(_))
    ));

    let mut bytecode_fixture = LoadFixture::new(true, true);
    let replacement = admitted_bytecode("replacement-bytecode");
    let declared = bytecode_fixture
        .resolver
        .packages
        .get(&bytecode_fixture.implementation_reference)
        .unwrap()
        .bytecode
        .as_ref()
        .unwrap();
    bytecode_fixture.resolver.bytecodes.insert(
        (
            bytecode_fixture.implementation_reference.clone(),
            declared.bytecode_identity.clone(),
        ),
        replacement,
    );
    assert!(matches!(
        DeploymentBytecodeLoader::new(&bytecode_fixture.resolver)
            .load(&bytecode_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ReferenceMismatch {
            expected,
            actual,
        }) if matches!(
            expected.as_ref(),
            DeploymentBytecodeReference::PackageBytecode { .. }
        ) && matches!(
            actual.as_ref(),
            DeploymentBytecodeReference::PackageBytecode { .. }
        )
    ));
}

#[test]
fn load_never_resolves_a_provider_deployment() {
    let fixture = LoadFixture::new(true, true);
    DeploymentBytecodeLoader::new(&fixture.resolver)
        .load(&fixture.deployment_reference)
        .unwrap();
    assert_eq!(fixture.resolver.deployment_calls.get(), 1);
    assert_eq!(fixture.resolver.unexpected_deployment_calls.get(), 0);
}

#[test]
fn load_rejects_duplicate_bindings_and_missing_operation_coverage() {
    let mut duplicate_fixture = LoadFixture::new(true, true);
    let duplicate = duplicate_fixture.resolver.deployment.package_bindings[0].clone();
    Arc::make_mut(&mut duplicate_fixture.resolver.deployment)
        .package_bindings
        .push(duplicate.clone());
    assert!(matches!(
        DeploymentBytecodeLoader::new(&duplicate_fixture.resolver)
            .load(&duplicate_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::DuplicatePackageBinding { key })
            if key == duplicate.key
    ));

    let mut duplicate_slot_fixture = LoadFixture::new(true, true);
    let duplicate = duplicate_slot_fixture.resolver.deployment.service_selectors[0].clone();
    Arc::make_mut(&mut duplicate_slot_fixture.resolver.deployment)
        .service_selectors
        .push(duplicate.clone());
    assert!(matches!(
        DeploymentBytecodeLoader::new(&duplicate_slot_fixture.resolver)
            .load(&duplicate_slot_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::DuplicateServiceSlot { key })
            if key == duplicate.key
    ));

    let mut deployment_coverage_fixture = LoadFixture::new(true, true);
    let undeclared_operation =
        skiff_artifact_identity::contract_operation_id("example.consumer", "1.0.0", "undeclared")
            .unwrap();
    let deployment = Arc::make_mut(&mut deployment_coverage_fixture.resolver.deployment);
    deployment
        .operation_bindings
        .push(DeploymentOperationBinding {
            contract_operation_id: undeclared_operation.clone(),
            package_callable_id: PackageCallableId::new("callable:undeclared"),
        });
    skiff_artifact_identity::assign_service_deployment_identity(deployment).unwrap();
    let reference = deployment_reference(deployment);
    deployment_coverage_fixture.resolver.deployment_reference = reference.clone();
    deployment_coverage_fixture.deployment_reference = reference;
    assert!(matches!(
        DeploymentBytecodeLoader::new(&deployment_coverage_fixture.resolver)
            .load(&deployment_coverage_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::OperationCoverageMismatch {
            actual,
            ..
        }) if actual == BTreeSet::from([undeclared_operation])
    ));

    let service_coverage_fixture = LoadFixture::new(true, false);
    assert!(matches!(
        DeploymentBytecodeLoader::new(&service_coverage_fixture.resolver)
            .load(&service_coverage_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::MissingOperation {
            operation,
            ..
        }) if operation == service_coverage_fixture.dependency_operation
    ));
}

#[test]
fn load_fails_closed_for_missing_package_and_contract_records() {
    let mut package_fixture = LoadFixture::new(true, true);
    package_fixture
        .resolver
        .packages
        .remove(&package_fixture.dependency_reference);
    assert!(matches!(
        DeploymentBytecodeLoader::new(&package_fixture.resolver)
            .load(&package_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ContentResolution {
            reference,
            ..
        }) if matches!(reference.as_ref(), DeploymentBytecodeReference::Package(_))
    ));

    let mut contract_fixture = LoadFixture::new(true, true);
    contract_fixture
        .resolver
        .contracts
        .remove(&contract_fixture.dependency_contract);
    assert!(matches!(
        DeploymentBytecodeLoader::new(&contract_fixture.resolver)
            .load(&contract_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ContentResolution {
            reference,
            ..
        }) if matches!(
            reference.as_ref(),
            DeploymentBytecodeReference::ServiceContract(_)
        )
    ));
}

#[test]
fn load_rejects_binding_and_slot_owner_mismatches() {
    let mut package_fixture = LoadFixture::new(true, true);
    let expected_key = package_fixture.resolver.deployment.package_bindings[0]
        .key
        .clone();
    let mismatched_owner = package_fixture
        .dependency_reference
        .package_build_id
        .clone();
    let deployment = Arc::make_mut(&mut package_fixture.resolver.deployment);
    deployment.package_bindings[0].key.caller_package_build_id = mismatched_owner;
    skiff_artifact_identity::assign_service_deployment_identity(deployment).unwrap();
    let reference = deployment_reference(deployment);
    package_fixture.resolver.deployment_reference = reference.clone();
    package_fixture.deployment_reference = reference;
    assert!(matches!(
        DeploymentBytecodeLoader::new(&package_fixture.resolver)
            .load(&package_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::MissingPackageBinding { key })
            if key == expected_key
    ));

    let mut service_fixture = LoadFixture::new(true, true);
    let expected_key = service_fixture.resolver.deployment.service_selectors[0]
        .key
        .clone();
    let mismatched_owner = service_fixture
        .dependency_reference
        .package_build_id
        .clone();
    let deployment = Arc::make_mut(&mut service_fixture.resolver.deployment);
    deployment.service_selectors[0].key.caller_package_build_id = mismatched_owner;
    skiff_artifact_identity::assign_service_deployment_identity(deployment).unwrap();
    let reference = deployment_reference(deployment);
    service_fixture.resolver.deployment_reference = reference.clone();
    service_fixture.deployment_reference = reference;
    assert!(matches!(
        DeploymentBytecodeLoader::new(&service_fixture.resolver)
            .load(&service_fixture.deployment_reference),
        Err(DeploymentBytecodeHydrationError::ContractMismatch {
            key: Some(key),
            actual: None,
            ..
        }) if key == expected_key
    ));
}

#[test]
fn load_hydrates_service_fallback_schema_from_bytecode_relocation() {
    let record = service_internal_error_record();
    let fallback = ContractTypeRef::package_schema(
        record.package_id.clone(),
        record.stable_schema_key.clone(),
        record.package_schema_type_id.clone(),
    );
    let (provider_contract, provider_reference, provider_operation) =
        valid_contract("example.provider", true);
    let provider_operation = provider_operation.expect("provider contract has an operation");
    let (bytecode, coordinate, callable) = service_operation_bytecode(
        fallback,
        provider_operation.clone(),
        provider_reference.service_protocol_identity.clone(),
    );
    let provider_requirement = ContractRequirement {
        alias: "provider".to_string(),
        service_id: provider_reference.service_id.clone(),
        contract_version: provider_reference.contract_version.clone(),
        expected_protocol_identity: provider_reference.service_protocol_identity.clone(),
    };
    let std_package = std_fallback_package(record);
    let std_reference = package_reference(&std_package);
    let mut implementation = callable_package(
        &bytecode,
        &coordinate,
        &callable,
        skiff_artifact_model::OperationCallableKind::InternalFunction,
    )
    .as_ref()
    .clone();
    implementation.package_requirements = vec![PackageRequirement {
        alias: "std".to_string(),
        package_id: std_reference.package_id.clone(),
        exact_version: std_reference.package_version.clone(),
        expected_local_abi: std_reference.package_local_abi_identity.clone(),
        expected_package_build: None,
    }];
    implementation.contract_requirements = vec![provider_requirement.clone()];
    implementation.service_requirements = vec![ServiceRequirement {
        contract_requirement: provider_requirement,
        service_binding_slot: 3,
        used_operations: BTreeSet::from([provider_operation.clone()]),
    }];
    implementation.service_call_refs = vec![ServiceCallRef {
        service_requirement_slot: 3,
        contract_operation_id: provider_operation.clone(),
        expected_protocol_identity: provider_reference.service_protocol_identity.clone(),
    }];
    let implementation = Arc::new(implementation);
    let implementation_reference = package_reference(&implementation);
    let implementation_hydrated = HydratedBytecodePackage::checked(
        implementation_reference.clone(),
        Arc::clone(&implementation),
        Arc::clone(&bytecode),
    )
    .expect("caller bytecode package hydrates");
    let std_hydrated =
        HydratedBytecodePackage::checked_type_only(std_reference.clone(), Arc::clone(&std_package))
            .expect("type-only std fallback package hydrates");
    let service_key = ServiceRequirementKey {
        caller_package_build_id: implementation_reference.package_build_id.clone(),
        service_requirement_slot: 3,
    };
    let std_key = skiff_artifact_model::PackageRequirementKey {
        caller_package_build_id: implementation_reference.package_build_id.clone(),
        package_requirement_alias: "std".to_string(),
    };
    let (own_contract, own_contract_reference, _) = valid_contract("example.consumer", false);
    let mut deployment_record = deployment(
        implementation_reference.clone(),
        own_contract_reference.clone(),
        vec![ServiceSelectorBinding {
            key: service_key.clone(),
            contract: provider_reference.clone(),
        }],
    )
    .as_ref()
    .clone();
    deployment_record.package_bindings = vec![skiff_artifact_model::PackageBinding {
        key: std_key,
        package: std_reference.clone(),
    }];
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment_record).unwrap();
    let deployment_reference = deployment_reference(&deployment_record);
    let contracts = BTreeMap::from([
        (own_contract_reference, own_contract),
        (provider_reference.clone(), provider_contract),
    ]);
    let dependency = HydratedServiceDependency::new(
        service_key,
        provider_reference,
        BTreeSet::from([provider_operation]),
    );
    let hydrated = HydratedDeploymentBytecode::checked(
        deployment_reference,
        Arc::new(deployment_record),
        contracts,
        vec![dependency],
        vec![implementation_hydrated, std_hydrated],
    )
    .expect("service fallback PackageSchema must hydrate from the bytecode relocation");

    assert!(hydrated
        .packages()
        .contains_key(&std_reference.package_build_id));
    assert_eq!(
        hydrated
            .packages()
            .get(&std_reference.package_build_id)
            .unwrap()
            .reference(),
        &std_reference
    );
}

fn service_operation_bytecode(
    fallback: ContractTypeRef,
    operation: ContractOperationId,
    protocol: ServiceProtocolIdentity,
) -> (
    Arc<ValidatedBytecodeArtifact>,
    PackageExecutableCoordinate,
    PackageCallableId,
) {
    let (bytecode, coordinate, callable) = callable_bytecode(false);
    let mut artifact = bytecode.artifact().clone();
    let function = artifact
        .image
        .functions
        .get_mut("manifest::run")
        .expect("callable fixture function");
    function.words = vec![0x22, 0, 0, 0, 0, 0x25];
    function.source_map[0].end_pc = 6;
    function.relocations = vec![BytecodeRelocation::ServiceOperationRef {
        service_call: ServiceCallBoundaryFacts::new(
            ServiceCallRef {
                service_requirement_slot: 3,
                contract_operation_id: operation,
                expected_protocol_identity: protocol,
            },
            service_boundary_plan(fallback),
        ),
    }];
    artifact
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "manifest::run".to_string(),
            site_pc: 0,
            resume_pc: 5,
            end_resume_pc: None,
            expected_stack_height_before_result: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            result_materializations: Vec::new(),
            emit_stream_item_shape_ref: None,
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));
    skiff_artifact_identity::assign_bytecode_identity(&mut artifact).unwrap();
    (
        Arc::new(ValidatedBytecodeArtifact::admit(artifact).unwrap()),
        coordinate,
        callable,
    )
}

fn service_boundary_plan(fallback: ContractTypeRef) -> skiff_artifact_model::ServiceBoundaryPlan {
    skiff_artifact_model::ServiceBoundaryPlan {
        arguments: Vec::new(),
        results: Vec::new(),
        error: BoundaryErrorPlan {
            fallback_contract_type: fallback,
            fallback: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Caller,
                lifetime: BoundaryValueLifetime::Call,
            },
            policy: BoundaryErrorPolicy::DynamicPublicSchema {
                admission: BoundaryErrorAdmission::PublicNameableSchemaClosed,
                fallback_identity: BoundaryErrorFallbackIdentity::StdServiceInternalError,
            },
            transfer: BoundaryTransfer::Move,
            drop: BoundaryDropPlan::SnapshotRelease,
            source: ValueProvenance::Fresh,
        },
        stream_item: None,
        callbacks: ServiceCallbackPlan::None,
        effects: skiff_artifact_model::CallableEffectSummary::analysis_pending(),
    }
}

fn service_internal_error_record() -> PackageSchemaTypeRecord {
    let descriptor = PackageSchemaCanonicalDescriptor {
        type_params: Vec::new(),
        descriptor: ContractTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("message".to_string(), ContractTypeRef::builtin("string")),
                ("traceId".to_string(), ContractTypeRef::builtin("string")),
                ("errorId".to_string(), ContractTypeRef::builtin("string")),
            ]),
        },
    };
    let type_id = skiff_artifact_model::derive_package_schema_type_id(
        "skiff.run/std",
        "std.service.InternalError",
        &descriptor,
    )
    .unwrap();
    PackageSchemaTypeRecord {
        package_id: "skiff.run/std".to_string(),
        stable_schema_key: "std.service.InternalError".to_string(),
        package_schema_type_id: type_id,
        canonical_descriptor: descriptor,
    }
}

fn std_fallback_package(record: PackageSchemaTypeRecord) -> Arc<PackageArtifact> {
    let mut artifact = package_artifact("skiff.run/std", "build:std-fallback", None)
        .as_ref()
        .clone();
    artifact.package_schema_index.package_schema_index_identity =
        skiff_artifact_identity::package_schema_index_identity(
            &artifact.package_id,
            &BTreeMap::<String, PackageSchemaIndexEntry>::new(),
        )
        .unwrap();
    artifact
        .bytecode_schema_records
        .insert(record.package_schema_type_id.clone(), record);
    skiff_artifact_identity::assign_package_artifact_identities(&mut artifact).unwrap();
    Arc::new(artifact)
}

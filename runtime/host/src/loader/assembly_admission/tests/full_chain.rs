use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use skiff_artifact_model::*;
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use super::super::*;

struct CountingResolver {
    assembly: Arc<RuntimeAssembly>,
    deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
    reads: AtomicUsize,
}

impl RuntimeAssemblyRecordResolver for CountingResolver {
    fn resolve_runtime_assembly(
        &self,
        _reference: &RuntimeAssemblyRef,
    ) -> anyhow::Result<Arc<RuntimeAssembly>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::clone(&self.assembly))
    }
}

impl RuntimeAssemblyContentResolver for CountingResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.deployments
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, deployment)| Arc::clone(deployment))
            .ok_or_else(|| anyhow::anyhow!("missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.contracts
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, contract)| Arc::clone(contract))
            .ok_or_else(|| anyhow::anyhow!("missing contract"))
    }

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("missing package schema record {reference:?}")
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.packages
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, package)| Arc::clone(package))
            .ok_or_else(|| anyhow::anyhow!("missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.files
            .iter()
            .find(|(candidate_package, candidate_file, _)| {
                candidate_package == package && candidate_file == reference
            })
            .map(|(_, _, file)| Arc::clone(file))
            .ok_or_else(|| anyhow::anyhow!("missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("fixture has no static resources")
    }
}

struct FullChainFixture {
    assembly: RuntimeAssembly,
    resolver: CountingResolver,
    provider_contract_ref: ServiceContractRef,
    provider_contract: Arc<ServiceContract>,
    provider_operation_id: ContractOperationId,
    provider_callable_id: PackageCallableId,
    provider_deployment_ref: ServiceDeploymentRef,
    consumer_deployment_ref: ServiceDeploymentRef,
    consumer_package_ref: PackageArtifactRef,
    consumer_file_ir_identity: String,
    ingress: IngressSelector,
}

impl FullChainFixture {
    fn new() -> Self {
        let operation_contract = operation_contract();
        let (provider_contract, provider_operation_id) = service_contract(
            "example.phase-three.provider",
            "health",
            "Phase three provider",
            operation_contract.clone(),
        );
        let provider_contract_ref = contract_ref(&provider_contract);
        let (consumer_contract, consumer_operation_id) = service_contract(
            "example.phase-three.consumer",
            "check",
            "Phase three consumer",
            operation_contract.clone(),
        );
        let consumer_contract_ref = contract_ref(&consumer_contract);

        let provider_callable_id = PackageCallableId::new("callable:provider-health");
        let provider_file = implementation_file("provider.main", "health", None);
        let provider_file_ref = file_ref(&provider_file);
        let provider_package = implementation_package(
            "example.phase-three-provider",
            "health",
            provider_callable_id.clone(),
            &provider_file,
            operation_contract.clone(),
            None,
        );
        let provider_package_ref = package_ref(&provider_package);

        let service_requirement_slot = 7;
        let provider_call = ServiceCallRef {
            service_requirement_slot,
            contract_operation_id: provider_operation_id.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let provider_requirement = ContractRequirement {
            alias: "provider".to_string(),
            service_id: provider_contract_ref.service_id.clone(),
            contract_version: provider_contract_ref.contract_version.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let consumer_callable_id = PackageCallableId::new("callable:consumer-check");
        let consumer_file =
            implementation_file("consumer.main", "check", Some(provider_call.clone()));
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let consumer_package = implementation_package(
            "example.phase-three-consumer",
            "check",
            consumer_callable_id,
            &consumer_file,
            operation_contract,
            Some((provider_requirement, provider_call)),
        );
        let consumer_package_ref = package_ref(&consumer_package);

        let ingress = IngressSelector {
            protocol: IngressProtocol::Http,
            host: "phase-three.test".to_string(),
            method: Some("GET".to_string()),
            path: "/check".to_string(),
        };
        let provider_deployment = project_service_deployment(
            ServiceDeploymentInput {
                schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
                contract: provider_contract_ref.clone(),
                deployment_revision: DeploymentRevision::new("provider-revision-1"),
                implementation: provider_package_ref.clone(),
                operation_bindings: vec![ServiceDeploymentOperationInput {
                    contract_operation_id: provider_operation_id.clone(),
                    package_public_path: "health".to_string(),
                }],
                package_bindings: Vec::new(),
                service_selectors: Vec::new(),
                ingress: Vec::new(),
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                runtime_capability_bindings: Vec::new(),
                policy: policy(),
                diagnostic_text: DeploymentDiagnosticText {
                    display_name: "Phase three provider deployment".to_string(),
                    notes: BTreeMap::new(),
                },
            },
            &provider_contract,
            std::slice::from_ref(&provider_package),
            &BTreeMap::new(),
        )
        .unwrap();
        let provider_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment);
        let consumer_deployment = project_service_deployment(
            ServiceDeploymentInput {
                schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
                contract: consumer_contract_ref.clone(),
                deployment_revision: DeploymentRevision::new("consumer-revision-1"),
                implementation: consumer_package_ref.clone(),
                operation_bindings: vec![ServiceDeploymentOperationInput {
                    contract_operation_id: consumer_operation_id.clone(),
                    package_public_path: "check".to_string(),
                }],
                package_bindings: Vec::new(),
                service_selectors: vec![ServiceSelectorBinding {
                    key: ServiceRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        service_requirement_slot,
                    },
                    contract: provider_contract_ref.clone(),
                }],
                ingress: vec![DeploymentIngressBinding {
                    selector: ingress.clone(),
                    contract_operation_id: consumer_operation_id,
                }],
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                runtime_capability_bindings: Vec::new(),
                policy: policy(),
                diagnostic_text: DeploymentDiagnosticText {
                    display_name: "Phase three consumer deployment".to_string(),
                    notes: BTreeMap::new(),
                },
            },
            &consumer_contract,
            std::slice::from_ref(&consumer_package),
            &BTreeMap::new(),
        )
        .unwrap();
        let consumer_deployment_ref =
            skiff_artifact_identity::service_deployment_ref(&consumer_deployment);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&consumer_deployment_ref),
            &[consumer_deployment.clone(), provider_deployment.clone()],
            &[consumer_contract.clone(), provider_contract.clone()],
            &[consumer_package.clone(), provider_package.clone()],
        )
        .unwrap();
        let provider_contract = Arc::new(provider_contract);
        let resolver = CountingResolver {
            assembly: Arc::new(assembly.clone()),
            deployments: vec![
                (
                    consumer_deployment_ref.clone(),
                    Arc::new(consumer_deployment),
                ),
                (
                    provider_deployment_ref.clone(),
                    Arc::new(provider_deployment),
                ),
            ],
            contracts: vec![
                (consumer_contract_ref, Arc::new(consumer_contract)),
                (
                    provider_contract_ref.clone(),
                    Arc::clone(&provider_contract),
                ),
            ],
            packages: vec![
                (consumer_package_ref.clone(), Arc::new(consumer_package)),
                (provider_package_ref.clone(), Arc::new(provider_package)),
            ],
            files: vec![
                (
                    consumer_package_ref.clone(),
                    consumer_file_ref,
                    Arc::new(consumer_file),
                ),
                (
                    provider_package_ref,
                    provider_file_ref,
                    Arc::new(provider_file),
                ),
            ],
            reads: AtomicUsize::new(0),
        };
        Self {
            assembly,
            resolver,
            provider_contract_ref,
            provider_contract,
            provider_operation_id,
            provider_callable_id,
            provider_deployment_ref,
            consumer_deployment_ref,
            consumer_package_ref,
            consumer_file_ir_identity,
            ingress,
        }
    }
}

#[tokio::test]
async fn committed_recovery_nonempty_generation_survives_restart_with_exact_registration() {
    let fixture = FullChainFixture::new();
    let reference = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    let first = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    let first_active = first
        .recover_committed("prod", 7, &reference, &fixture.resolver, None)
        .await
        .expect("non-empty committed generation must recover");
    let first_reads = fixture.resolver.reads.load(Ordering::SeqCst);
    assert_eq!(first_active.generation(), 7);
    assert!(!first_active.is_empty());
    assert!(first_reads > 1);
    assert!(matches!(
        first.registration().unwrap(),
        Some(AssemblyActivationControl::Register {
            generation: 7,
            assembly,
            replica_id,
            ..
        }) if assembly == reference && replica_id == "runtime-a"
    ));

    let restarted = AssemblyAdmissionController::new(
        "runtime-a",
        skiff_runtime_capability_context::DbProviderSource::unavailable(),
    );
    let restarted_active = restarted
        .recover_committed("prod", 7, &reference, &fixture.resolver, None)
        .await
        .expect("restart must rebuild the same non-empty committed generation");
    assert_eq!(restarted_active.generation(), 7);
    assert_eq!(restarted_active.identity(), &reference.assembly_identity);
    assert!(fixture.resolver.reads.load(Ordering::SeqCst) > first_reads);
    assert!(matches!(
        restarted.registration().unwrap(),
        Some(AssemblyActivationControl::Register {
            generation: 7,
            assembly,
            replica_id,
            ..
        }) if assembly == reference && replica_id == "runtime-a"
    ));
}

#[tokio::test]
async fn projected_nonempty_assembly_admits_and_active_lookup_is_io_free() {
    let fixture = FullChainFixture::new();
    let controller = AssemblyAdmissionController::default();

    let active = controller
        .admit(fixture.assembly.clone(), &fixture.resolver)
        .await
        .expect("projected non-empty assembly should admit");

    assert_eq!(active.identity(), &fixture.assembly.assembly_identity);
    assert_eq!(active.candidate().shared_image().code_slots().len(), 2);
    assert_eq!(active.candidate().activations().len(), 2);
    let stored_contract = active
        .contract_store()
        .contract(&fixture.provider_contract_ref)
        .unwrap();
    assert!(Arc::ptr_eq(stored_contract, &fixture.provider_contract));
    assert_eq!(
        stored_contract.service_protocol_identity,
        fixture.provider_contract_ref.service_protocol_identity
    );
    let expected_descriptor = fixture
        .provider_contract
        .operations
        .get(&fixture.provider_operation_id)
        .unwrap();
    let linked_call = active
        .candidate()
        .shared_image()
        .resolve_activation_relative_service_call(
            &fixture.consumer_package_ref.package_build_id,
            &fixture.consumer_file_ir_identity,
            ServiceCallRefIndex::new(0),
        )
        .unwrap();
    assert_eq!(
        linked_call.caller_package_build_id(),
        &fixture.consumer_package_ref.package_build_id
    );
    assert_eq!(linked_call.service_requirement_slot(), 7);
    assert_eq!(linked_call.operation_id(), &fixture.provider_operation_id);
    assert_eq!(
        linked_call.expected_protocol_identity(),
        &fixture.provider_contract_ref.service_protocol_identity
    );
    let binding = active
        .candidate()
        .resolve_activation_relative_service_call(&fixture.consumer_deployment_ref, &linked_call)
        .unwrap();
    assert_eq!(
        &binding.key().caller_package_build_id,
        &fixture.consumer_package_ref.package_build_id
    );
    assert_eq!(binding.key().service_requirement_slot, 7);
    assert_eq!(binding.contract(), &fixture.provider_contract_ref);
    assert_eq!(binding.provider(), &fixture.provider_deployment_ref);
    let provider_operation = active
        .activation(binding.provider())
        .unwrap()
        .operation(linked_call.operation_id())
        .unwrap();
    assert_eq!(
        provider_operation.package_callable_id(),
        &fixture.provider_callable_id
    );
    let active_descriptor = active
        .operation_descriptor(binding.contract(), linked_call.operation_id())
        .unwrap();
    assert!(std::ptr::eq(active_descriptor, expected_descriptor));
    assert!(std::ptr::eq(
        active_descriptor,
        active
            .contract_store()
            .operation_descriptor(binding.contract(), linked_call.operation_id())
            .unwrap()
    ));
    assert_eq!(
        active_descriptor.contract.return_value.value_plan,
        expected_descriptor.contract.return_value.value_plan
    );
    assert!(matches!(
        &active_descriptor.contract.return_value.value_plan,
        BoundaryValuePlan::Linkable {
            owner: BoundaryValueOwner::Provider,
            ..
        }
    ));

    let reads_after_admit = fixture.resolver.reads.load(Ordering::SeqCst);
    assert!(reads_after_admit > 0);
    let route = controller.route(&fixture.ingress).unwrap().unwrap();
    assert_eq!(route.assembly_identity(), active.identity());
    assert_eq!(
        &route.activation().identity().deployment,
        &fixture.consumer_deployment_ref
    );
    assert_eq!(route.operation_descriptor().stable_key, "check");
    let binding_wire =
        serde_json::to_string(&active.candidate().assembly().service_binding_templates).unwrap();
    assert!(!binding_wire.contains("stableKey"));
    assert!(!binding_wire.contains("valuePlan"));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "active service binding, contract, provider, and route lookup must not trigger artifact I/O"
    );

    let mut tampered = fixture.assembly.clone();
    tampered.assembly_identity = AssemblyIdentity::new("tampered-candidate");
    assert!(controller.admit(tampered, &fixture.resolver).await.is_err());
    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "failed reload must fail before content I/O and preserve active"
    );
}

fn service_contract(
    service_id: &str,
    stable_key: &str,
    display_name: &str,
    operation_contract: BoundaryOperationContract,
) -> (ServiceContract, ContractOperationId) {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, stable_key)
            .unwrap();
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: stable_key.to_string(),
                contract: operation_contract,
            },
        )]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: display_name.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    (contract, operation_id)
}

fn implementation_file(
    module_path: &str,
    symbol: &str,
    service_call: Option<ServiceCallRef>,
) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    });
    if let Some(service_call) = service_call {
        file.external_refs.service_call_refs.push(service_call);
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(0),
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    file
}

fn implementation_package(
    package_id: &str,
    public_path: &str,
    callable_id: PackageCallableId,
    file: &FileIrUnit,
    operation_contract: BoundaryOperationContract,
    service_dependency: Option<(ContractRequirement, ServiceCallRef)>,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut contract_requirements = Vec::new();
    let mut service_requirements = Vec::new();
    let mut service_call_refs = Vec::new();
    if let Some((contract_requirement, service_call)) = service_dependency {
        contract_requirements.push(contract_requirement.clone());
        service_requirements.push(ServiceRequirement {
            contract_requirement,
            service_binding_slot: service_call.service_requirement_slot,
            used_operations: BTreeSet::from([service_call.contract_operation_id.clone()]),
        });
        service_call_refs.push(service_call);
    }
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                public_path.to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: PackageCallableSignature {
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("bool"),
                        },
                        throw_types: Vec::new(),
                        may_suspend: false,
                    },
                },
            )]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: package_id.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::from([(
            callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable_id.clone(),
                target: OperationTargetRef {
                    file_ref,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        package_requirements: Vec::new(),
        contract_requirements,
        service_requirements,
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::from([(
            callable_id.clone(),
            CallableSemanticFacts {
                effects: CallableEffectSummary::Analyzed {
                    effects: effects.clone(),
                },
                provenance: provenance.clone(),
                resolved_call_targets: BTreeMap::new(),
            },
        )]),
        boundary_projections: BTreeMap::from([(
            callable_id,
            BoundaryCallableProjection::Available {
                operation_contract,
                implementation_requirements: BoundaryImplementationRequirements {
                    config: Vec::new(),
                    state: Vec::new(),
                    native_capabilities: Vec::new(),
                    runtime_capabilities: Vec::new(),
                    complete_may_effects: effects,
                    provenance,
                },
            },
        )]),
        service_call_refs,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    package
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn operation_contract() -> BoundaryOperationContract {
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

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend: false,
    }
}

fn policy() -> DeploymentPolicy {
    DeploymentPolicy {
        timeout_ms: Some(1_000),
        resources: ResourcePolicy {
            cpu_millis: 100,
            memory_bytes: 1_048_576,
        },
        activation: ActivationPolicy {
            max_concurrency: 4,
            idle_timeout_ms: None,
        },
        principal: "service:phase-three".to_string(),
    }
}

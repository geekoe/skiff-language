use std::{
    collections::BTreeMap,
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
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    contract_ref: ServiceContractRef,
    contract: Arc<ServiceContract>,
    package_ref: PackageArtifactRef,
    package: Arc<PackageArtifact>,
    file_ref: FileIrRef,
    file: Arc<FileIrUnit>,
    reads: AtomicUsize,
}

impl RuntimeAssemblyContentResolver for CountingResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if reference != &self.deployment_ref {
            anyhow::bail!("missing deployment")
        }
        Ok(Arc::clone(&self.deployment))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if reference != &self.contract_ref {
            anyhow::bail!("missing contract")
        }
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if reference != &self.package_ref {
            anyhow::bail!("missing package")
        }
        Ok(Arc::clone(&self.package))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if package != &self.package_ref || reference != &self.file_ref {
            anyhow::bail!("missing File IR")
        }
        Ok(Arc::clone(&self.file))
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
    operation_id: ContractOperationId,
    ingress: IngressSelector,
}

impl FullChainFixture {
    fn new() -> Self {
        let service_id = "example.phase-three";
        let contract_version = "1.0.0";
        let operation_id =
            skiff_artifact_identity::contract_operation_id(service_id, contract_version, "health")
                .unwrap();
        let operation_contract = operation_contract();
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: service_id.to_string(),
            contract_version: contract_version.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(
                operation_id.clone(),
                BoundaryOperationDescriptor {
                    operation_id: operation_id.clone(),
                    stable_key: "health".to_string(),
                    contract: operation_contract.clone(),
                },
            )]),
            boundary_schema: BTreeMap::new(),
            diagnostic_text: ContractDiagnosticText {
                service: "Phase three".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
        let contract_ref = contract_ref(&contract);

        let callable_id = PackageCallableId::new("callable:health");
        let mut file = FileIrUnit::empty("provider.main", "source:provider.main");
        file.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: "health".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::native("bool"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody::default(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
        let file_ref = FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        };
        let effects = no_effects();
        let provenance = CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        };
        let mut package = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.phase-three-provider".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: vec![file_ref.clone()],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::from([(
                    "health".to_string(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature: PackageCallableSignature {
                            parameters: Vec::new(),
                            return_type: PackageTypeRef::Local {
                                local_type: TypeRefIr::native("bool"),
                            },
                            throw_types: Vec::new(),
                            may_suspend: false,
                        },
                    },
                )]),
            },
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::from([(
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target: OperationTargetRef {
                        file_ref: file_ref.clone(),
                        executable_index: 0,
                        callable_abi_id: callable_id.to_string(),
                        callable_kind: OperationCallableKind::PublicFunction,
                    },
                },
            )]),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
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
                callable_id.clone(),
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
            service_call_refs: Vec::new(),
        };
        skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
        let package_ref = package_ref(&package);

        let ingress = IngressSelector {
            protocol: IngressProtocol::Http,
            host: "phase-three.test".to_string(),
            method: Some("GET".to_string()),
            path: "/health".to_string(),
        };
        let deployment = project_service_deployment(
            ServiceDeploymentInput {
                schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
                contract: contract_ref.clone(),
                deployment_revision: DeploymentRevision::new("revision-1"),
                implementation: package_ref.clone(),
                operation_bindings: vec![ServiceDeploymentOperationInput {
                    contract_operation_id: operation_id.clone(),
                    package_public_path: "health".to_string(),
                }],
                package_bindings: Vec::new(),
                service_selectors: Vec::new(),
                ingress: vec![DeploymentIngressBinding {
                    selector: ingress.clone(),
                    contract_operation_id: operation_id.clone(),
                }],
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                runtime_capability_bindings: Vec::new(),
                policy: policy(),
                diagnostic_text: DeploymentDiagnosticText {
                    display_name: "Phase three deployment".to_string(),
                    notes: BTreeMap::new(),
                },
            },
            &contract,
            std::slice::from_ref(&package),
        )
        .unwrap();
        let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&deployment_ref),
            std::slice::from_ref(&deployment),
            std::slice::from_ref(&contract),
            std::slice::from_ref(&package),
        )
        .unwrap();
        let resolver = CountingResolver {
            deployment_ref,
            deployment: Arc::new(deployment),
            contract_ref,
            contract: Arc::new(contract),
            package_ref,
            package: Arc::new(package),
            file_ref,
            file: Arc::new(file),
            reads: AtomicUsize::new(0),
        };
        Self {
            assembly,
            resolver,
            operation_id,
            ingress,
        }
    }
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
    assert_eq!(active.candidate().shared_image().code_slots().len(), 1);
    let stored_contract = active
        .contract_store()
        .contract(&fixture.resolver.contract_ref)
        .unwrap();
    assert!(Arc::ptr_eq(stored_contract, &fixture.resolver.contract));
    assert_eq!(
        stored_contract.service_protocol_identity,
        fixture.resolver.contract_ref.service_protocol_identity
    );
    let expected_descriptor = fixture
        .resolver
        .contract
        .operations
        .get(&fixture.operation_id)
        .unwrap();
    let active_descriptor = active
        .operation_descriptor(&fixture.resolver.contract_ref, &fixture.operation_id)
        .unwrap();
    assert!(std::ptr::eq(active_descriptor, expected_descriptor));

    let reads_after_admit = fixture.resolver.reads.load(Ordering::SeqCst);
    assert!(reads_after_admit > 0);
    let route = controller.route(&fixture.ingress).unwrap().unwrap();
    assert_eq!(route.assembly_identity(), active.identity());
    assert!(std::ptr::eq(
        route.operation_descriptor().unwrap(),
        expected_descriptor
    ));
    let activation_wire = serde_json::to_string(route.activation().unwrap().source()).unwrap();
    assert!(!activation_wire.contains("stableKey"));
    assert!(!activation_wire.contains("valuePlan"));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "active contract/route lookup must not trigger artifact I/O"
    );

    let mut tampered = fixture.assembly;
    tampered.assembly_identity = AssemblyIdentity::new("tampered-candidate");
    assert!(controller.admit(tampered, &fixture.resolver).await.is_err());
    assert!(Arc::ptr_eq(&active, &controller.active().unwrap().unwrap()));
    assert_eq!(
        fixture.resolver.reads.load(Ordering::SeqCst),
        reads_after_admit,
        "failed reload must fail before content I/O and preserve active"
    );
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
        timeout_ms: 1_000,
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

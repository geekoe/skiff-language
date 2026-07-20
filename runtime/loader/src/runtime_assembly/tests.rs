use std::{cell::Cell, collections::BTreeMap, sync::Arc};

use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActivationPolicy, ActivationTemplate, BoundaryCallableProjection, BoundaryCallbackContract,
    BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallableEffectSummary,
    CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts, ContractDiagnosticText,
    ContractOperationId, DeploymentArtifactIdentity, DeploymentDiagnosticText,
    DeploymentOperationBinding, DeploymentPolicy, DeploymentRevision, ExecutableBody, ExecutableIr,
    ExecutableKind, FileIrRef, FileIrUnit, PackageArtifact, PackageArtifactRef, PackageBuildId,
    PackageCallableId, PackageCallableLinkFact, PackageCallableSignature, PackageCodeSlot,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageTypeRef, PublicationResourceRef, ResourcePolicy,
    RuntimeAssembly, ServiceBindingTemplate, ServiceContract, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity, SlotLayout, TypeRefIr,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

use super::*;

#[derive(Default)]
struct PanicResolver;

impl RuntimeAssemblyContentResolver for PanicResolver {
    fn resolve_deployment(
        &self,
        _reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        panic!("empty assembly must not resolve a deployment")
    }

    fn resolve_contract(
        &self,
        _reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        panic!("empty assembly must not resolve a contract")
    }

    fn resolve_package(
        &self,
        _reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        panic!("empty assembly must not resolve a package")
    }

    fn resolve_file_ir(
        &self,
        _package: &PackageArtifactRef,
        _reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        panic!("empty assembly must not resolve File IR")
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        panic!("empty assembly must not resolve a resource")
    }
}

struct FixtureResolver {
    deployment_ref: ServiceDeploymentRef,
    deployment: Arc<ServiceDeployment>,
    additional_deployment: Option<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    contract_ref: ServiceContractRef,
    contract: Arc<ServiceContract>,
    package_ref: PackageArtifactRef,
    package: Arc<PackageArtifact>,
    file_ref: FileIrRef,
    file: Arc<FileIrUnit>,
    resource_ref: PublicationResourceRef,
    resource: Arc<[u8]>,
    package_loads: Cell<usize>,
}

impl RuntimeAssemblyContentResolver for FixtureResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        if reference == &self.deployment_ref {
            return Ok(Arc::clone(&self.deployment));
        }
        if let Some((additional_ref, deployment)) = &self.additional_deployment {
            if reference == additional_ref {
                return Ok(Arc::clone(deployment));
            }
        }
        anyhow::bail!("missing deployment")
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        if reference != &self.contract_ref {
            anyhow::bail!("missing contract")
        }
        Ok(Arc::clone(&self.contract))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        if reference != &self.package_ref {
            anyhow::bail!("missing package")
        }
        self.package_loads.set(self.package_loads.get() + 1);
        Ok(Arc::clone(&self.package))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        if package != &self.package_ref || reference != &self.file_ref {
            anyhow::bail!("missing File IR")
        }
        Ok(Arc::clone(&self.file))
    }

    fn resolve_static_resource(
        &self,
        package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        if package != &self.package_ref || reference != &self.resource_ref {
            anyhow::bail!("missing static resource")
        }
        Ok(Arc::clone(&self.resource))
    }
}

struct Fixture {
    operation_id: ContractOperationId,
    callable_id: PackageCallableId,
    contract: ServiceContract,
    package: PackageArtifact,
    file: FileIrUnit,
    resource: Arc<[u8]>,
    deployment: ServiceDeployment,
    assembly: RuntimeAssembly,
}

impl Fixture {
    fn new() -> Self {
        let service_id = "example.health";
        let contract_version = "1.0.0";
        let operation_id =
            skiff_artifact_identity::contract_operation_id(service_id, contract_version, "health")
                .unwrap();
        let operation_contract = operation_contract();
        let descriptor = BoundaryOperationDescriptor {
            operation_id: operation_id.clone(),
            stable_key: "health".to_string(),
            contract: operation_contract.clone(),
        };
        let mut contract = ServiceContract {
            schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: service_id.to_string(),
            contract_version: contract_version.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
            operations: BTreeMap::from([(operation_id.clone(), descriptor)]),
            boundary_schema: BTreeMap::new(),
            diagnostic_text: ContractDiagnosticText {
                service: "Health".to_string(),
                operations: BTreeMap::from([(operation_id.clone(), "Health".to_string())]),
                types: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
        let contract_ref = contract_ref(&contract);

        let mut file = FileIrUnit::empty("provider.main", "source-hash");
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

        let callable_id = PackageCallableId::new("callable:health");
        let target = skiff_artifact_model::OperationTargetRef {
            file_ref: file_ref.clone(),
            executable_index: 0,
            callable_abi_id: callable_id.to_string(),
            callable_kind: skiff_artifact_model::OperationCallableKind::PublicFunction,
        };
        let resource: Arc<[u8]> = Arc::from(b"health-resource".as_slice());
        let resource_ref = PublicationResourceRef {
            path: "assets/health.txt".to_string(),
            sha256: hex::encode(Sha256::digest(resource.as_ref())),
            byte_len: resource.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: None,
        };
        let effects = no_effects();
        let provenance = CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        };
        let mut package = PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: "example.health-provider".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("unassigned"),
            files: vec![file_ref],
            static_resources: vec![resource_ref],
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
                    target,
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
                    effects: CallableEffectSummary::Analyzed { effects },
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

        let mut deployment = ServiceDeployment {
            schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
            contract: contract_ref.clone(),
            deployment_revision: DeploymentRevision::new("revision-1"),
            deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
            implementation: package_ref.clone(),
            operation_bindings: vec![DeploymentOperationBinding {
                contract_operation_id: operation_id.clone(),
                package_callable_id: callable_id.clone(),
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
                display_name: "Health deployment".to_string(),
                notes: BTreeMap::new(),
            },
        };
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
        let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);

        let mut assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
            roots: vec![deployment_ref.clone()],
            resolved_deployments: vec![deployment_ref.clone()],
            resolved_contracts: vec![contract_ref],
            resolved_packages: vec![package_ref.clone()],
            package_link_plan: skiff_artifact_model::CanonicalPackageLinkPlan {
                code_slots: vec![PackageCodeSlot {
                    package: package_ref,
                }],
                package_links: Vec::new(),
            },
            service_binding_templates: vec![ServiceBindingTemplate {
                activation: deployment_ref.clone(),
                bindings: Vec::new(),
            }],
            activation_templates: vec![ActivationTemplate {
                deployment: deployment_ref,
                implementation_package_build_id: package.package_build_id.clone(),
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                policy: deployment.policy.clone(),
            }],
            global_ingress: Vec::new(),
        };
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

        Self {
            operation_id,
            callable_id,
            contract,
            package,
            file,
            resource,
            deployment,
            assembly,
        }
    }

    fn resolver(&self) -> FixtureResolver {
        FixtureResolver {
            deployment_ref: skiff_artifact_identity::service_deployment_ref(&self.deployment),
            deployment: Arc::new(self.deployment.clone()),
            additional_deployment: None,
            contract_ref: contract_ref(&self.contract),
            contract: Arc::new(self.contract.clone()),
            package_ref: package_ref(&self.package),
            package: Arc::new(self.package.clone()),
            file_ref: self.package.files[0].clone(),
            file: Arc::new(self.file.clone()),
            resource_ref: self.package.static_resources[0].clone(),
            resource: Arc::clone(&self.resource),
            package_loads: Cell::new(0),
        }
    }

    fn refresh_deployment_chain(&mut self) {
        skiff_artifact_identity::assign_service_deployment_identity(&mut self.deployment).unwrap();
        let reference = skiff_artifact_identity::service_deployment_ref(&self.deployment);
        self.assembly.roots = vec![reference.clone()];
        self.assembly.resolved_deployments = vec![reference.clone()];
        self.assembly.service_binding_templates[0].activation = reference.clone();
        self.assembly.activation_templates[0].deployment = reference;
        skiff_artifact_identity::assign_runtime_assembly_identity(&mut self.assembly).unwrap();
    }

    fn refresh_package_chain(&mut self) {
        skiff_artifact_identity::assign_package_artifact_identities(&mut self.package).unwrap();
        let reference = package_ref(&self.package);
        self.deployment.implementation = reference.clone();
        self.assembly.resolved_packages = vec![reference.clone()];
        self.assembly.package_link_plan.code_slots = vec![PackageCodeSlot { package: reference }];
        self.assembly.activation_templates[0].implementation_package_build_id =
            self.package.package_build_id.clone();
        self.refresh_deployment_chain();
    }
}

#[test]
fn canonical_empty_assembly_hydrates_without_storage_reads() {
    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new("unassigned"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: skiff_artifact_model::CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

    let hydrated = RuntimeAssemblyLoader::new(&PanicResolver)
        .load(assembly)
        .unwrap();
    assert!(hydrated.code_slots().is_empty());
    assert!(hydrated.contract_store().is_empty());
    assert_eq!(hydrated.deployments().len(), 0);
}

#[test]
fn typed_loader_preserves_contract_store_and_deterministic_code_lookup() {
    let mut fixture = Fixture::new();
    let mut second_deployment = fixture.deployment.clone();
    second_deployment.deployment_revision = DeploymentRevision::new("revision-2");
    skiff_artifact_identity::assign_service_deployment_identity(&mut second_deployment).unwrap();
    let second_ref = skiff_artifact_identity::service_deployment_ref(&second_deployment);
    fixture.assembly.roots.push(second_ref.clone());
    fixture
        .assembly
        .resolved_deployments
        .push(second_ref.clone());
    fixture
        .assembly
        .service_binding_templates
        .push(ServiceBindingTemplate {
            activation: second_ref.clone(),
            bindings: Vec::new(),
        });
    let mut second_activation = fixture.assembly.activation_templates[0].clone();
    second_activation.deployment = second_ref.clone();
    fixture
        .assembly
        .activation_templates
        .push(second_activation);
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut fixture.assembly).unwrap();

    let mut resolver = fixture.resolver();
    resolver.additional_deployment = Some((second_ref.clone(), Arc::new(second_deployment)));
    let contract_ref = contract_ref(&fixture.contract);
    let package_ref = package_ref(&fixture.package);

    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap();

    assert_eq!(resolver.package_loads.get(), 1);
    assert!(hydrated.deployment(&second_ref).is_some());
    assert_eq!(hydrated.code_slots().len(), 1);
    assert_eq!(
        hydrated.code_slot_index(&fixture.package.package_build_id),
        Some(0)
    );
    assert_eq!(hydrated.code_slot(0).unwrap().reference(), &package_ref);
    assert_eq!(
        hydrated
            .package(&fixture.package.package_build_id)
            .unwrap()
            .resource("assets/health.txt")
            .unwrap()
            .bytes()
            .as_ref(),
        b"health-resource"
    );
    let descriptor = hydrated
        .contract_store()
        .operation(&contract_ref, &fixture.operation_id)
        .unwrap();
    assert_eq!(descriptor.stable_key, "health");
    assert_eq!(
        hydrated.assembly().activation_templates[0].implementation_package_build_id,
        fixture.package.package_build_id
    );
}

#[test]
fn tampered_assembly_contract_deployment_package_and_file_fail_closed() {
    let fixture = Fixture::new();

    let mut assembly = fixture.assembly.clone();
    assembly.activation_templates[0].implementation_package_build_id =
        PackageBuildId::new("tampered");
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(assembly)
        .unwrap_err()
        .to_string()
        .contains("before hydration"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.contract)
        .operations
        .get_mut(&fixture.operation_id)
        .unwrap()
        .stable_key = "tampered".to_string();
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("contract content is invalid"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.deployment).policy.timeout_ms += 1;
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("deployment content mismatches ref"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.package).package_version = "2.0.0".to_string();
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("package content is invalid"));

    let mut resolver = fixture.resolver();
    Arc::make_mut(&mut resolver.file).executables[0].symbol = "tampered".to_string();
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly)
        .unwrap_err()
        .to_string()
        .contains("File IR content is invalid"));
}

#[test]
fn resource_hash_size_and_storage_path_fail_before_linking() {
    let fixture = Fixture::new();
    let mut resolver = fixture.resolver();
    resolver.resource = Arc::from(b"tamper-resource".as_slice());
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("hash mismatch"));

    let mut resolver = fixture.resolver();
    resolver.resource = Arc::from(b"tampered".as_slice());
    assert!(RuntimeAssemblyLoader::new(&resolver)
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("size mismatch"));

    let mut fixture = Fixture::new();
    fixture.package.static_resources[0].artifact_path = Some("../escape".to_string());
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err()
        .to_string()
        .contains("escape"));
}

#[test]
fn missing_file_link_target_and_contract_operation_mismatch_fail_closed() {
    let mut fixture = Fixture::new();
    fixture
        .package
        .callable_links
        .get_mut(&fixture.callable_id)
        .unwrap()
        .target
        .file_ref = FileIrRef::new("missing-file", "missing.module");
    fixture.refresh_package_chain();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly.clone())
        .unwrap_err()
        .to_string()
        .contains("targets missing File IR"));

    let mut fixture = Fixture::new();
    fixture.deployment.operation_bindings[0].contract_operation_id =
        ContractOperationId::new("missing-operation");
    fixture.refresh_deployment_chain();
    assert!(RuntimeAssemblyLoader::new(&fixture.resolver())
        .load(fixture.assembly)
        .unwrap_err()
        .to_string()
        .contains("operation bindings do not exactly match"));
}

#[test]
fn runtime_assembly_filesystem_resolver_hydrates_exact_canonical_closure() {
    let fixture = Fixture::new();
    let temp = TestArtifactRoot::new();
    let store = skiff_deployment::storage::CanonicalArtifactStore::create(temp.path()).unwrap();
    let package_ref = package_ref(&fixture.package);
    let contract_ref = contract_ref(&fixture.contract);
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&fixture.deployment);
    let assembly_ref = skiff_artifact_identity::runtime_assembly_ref(&fixture.assembly).unwrap();

    store.write_service_contract(&fixture.contract).unwrap();
    store.write_package_artifact(&fixture.package).unwrap();
    store
        .write_file_ir(&package_ref, &fixture.package.files[0], &fixture.file)
        .unwrap();
    store
        .write_static_resource(
            &package_ref,
            &fixture.package.static_resources[0],
            fixture.resource.as_ref(),
        )
        .unwrap();
    store.write_service_deployment(&fixture.deployment).unwrap();
    store.write_runtime_assembly(&fixture.assembly).unwrap();

    let resolver = crate::FilesystemRuntimeAssemblyContentResolver::from_store(store);
    let hydrated = resolver.load_runtime_assembly(&assembly_ref).unwrap();
    assert_eq!(
        hydrated.assembly().assembly_identity,
        assembly_ref.assembly_identity
    );
    assert!(hydrated.deployment(&deployment_ref).is_some());
    assert!(hydrated.contract_store().contract(&contract_ref).is_some());
    assert_eq!(
        hydrated
            .package(&package_ref.package_build_id)
            .unwrap()
            .resource("assets/health.txt")
            .unwrap()
            .bytes()
            .as_ref(),
        fixture.resource.as_ref()
    );
}

struct TestArtifactRoot(std::path::PathBuf);

impl TestArtifactRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-assembly-resolver-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).expect("create runtime assembly test root");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestArtifactRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
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
            ty: skiff_artifact_model::ContractTypeRef::builtin("bool"),
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
            memory_bytes: 1_024,
        },
        activation: ActivationPolicy {
            max_concurrency: 1,
            idle_timeout_ms: None,
        },
        principal: "service:health".to_string(),
    }
}

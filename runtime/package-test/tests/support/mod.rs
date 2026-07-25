use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::{
    ActivationPolicy, BoundaryCallableProjection, BoundaryCallbackContract,
    BoundaryCancellationContract, BoundaryEffectGuarantee, BoundaryErrorContract,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallIr, CallTargetIr,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    ContractDiagnosticText, ContractOperationId, ContractRequirement, DeploymentArtifactIdentity,
    DeploymentDiagnosticText, DeploymentIngressBinding, DeploymentOperationBinding,
    DeploymentPolicy, DeploymentRevision, ExecutableBody, ExecutableIr, ExecutableKind, ExprIr,
    FileIrRef, FileIrUnit, IngressProtocol, IngressSelector, OperationCallableKind,
    OperationTargetRef, PackageArtifact, PackageArtifactRef, PackageBinding, PackageBuildId,
    PackageCallableId, PackageCallableLinkFact, PackageCallableRef, PackageCallableSignature,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRefIr, PackageRequirement, PackageRequirementKey, PackageRuntimeRequirements,
    PackageSchemaIndexRef, PackageTypeRef, PublicationResourceRef, ResourcePolicy, RuntimeAssembly,
    ServiceCallRef, ServiceContract, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceProtocolIdentity, ServiceRequirement, ServiceRequirementKey, ServiceSelectorBinding,
    SlotLayout, TypeRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION, SERVICE_CONTRACT_SCHEMA_VERSION,
    SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_loader::RuntimeAssemblyContentResolver;

pub struct CanonicalFixture {
    pub assembly: RuntimeAssembly,
    pub contracts: Vec<ServiceContract>,
    pub packages: Vec<PackageArtifact>,
    pub files: Vec<(PackageArtifactRef, FileIrUnit)>,
    pub deployments: Vec<ServiceDeployment>,
    pub root: ServiceDeploymentRef,
    pub root_contract: ServiceContractRef,
    pub root_operation: ContractOperationId,
    pub ingress: IngressSelector,
    pub direct_caller: Option<PackageArtifactRef>,
    pub direct_dependency: Option<PackageArtifactRef>,
    pub direct_callable: Option<PackageCallableId>,
    pub service_provider: Option<ServiceDeploymentRef>,
}

impl CanonicalFixture {
    pub fn package_only() -> Self {
        let contract = contract("test.skiff/package-only");
        let (package, file) = implementation_package(
            "test.skiff/package-only-implementation",
            &contract,
            &[],
            &[],
        );
        let mut deployment = deployment(
            &contract,
            &package,
            "package-only-r1",
            Vec::new(),
            Vec::new(),
        );
        let ingress = IngressSelector {
            protocol: IngressProtocol::Http,
            host: "package-only.test".to_string(),
            method: Some("POST".to_string()),
            path: "/run".to_string(),
        };
        add_http_ingress(&mut deployment, &contract, &ingress.host, &ingress.path);
        Self::finish(
            vec![contract],
            vec![package],
            vec![file],
            vec![deployment],
            ingress,
            None,
            None,
            None,
            None,
        )
    }

    pub fn package_dependency() -> Self {
        let package_contract = contract("test.skiff/package-dependent");
        let helper_contract = contract("test.skiff/helper-contract");
        let (helper, helper_file) =
            implementation_package("test.skiff/direct-helper", &helper_contract, &[], &[]);
        let (caller, caller_file) = implementation_package(
            "test.skiff/direct-caller",
            &package_contract,
            &[("helper", &helper)],
            &[],
        );
        let mut deployment = deployment(
            &package_contract,
            &caller,
            "package-dependent-r1",
            vec![package_binding(&caller, "helper", &helper)],
            Vec::new(),
        );
        let ingress = IngressSelector {
            protocol: IngressProtocol::Http,
            host: "package-dependent.test".to_string(),
            method: Some("POST".to_string()),
            path: "/mutate".to_string(),
        };
        add_http_ingress(
            &mut deployment,
            &package_contract,
            &ingress.host,
            &ingress.path,
        );
        let direct_caller = package_ref(&caller);
        let direct_dependency = package_ref(&helper);
        let direct_callable = Some(PackageCallableId::new("callable.fixture"));
        Self::finish(
            vec![package_contract, helper_contract],
            vec![caller, helper],
            vec![caller_file, helper_file],
            vec![deployment],
            ingress,
            Some(direct_caller),
            Some(direct_dependency),
            direct_callable,
            None,
        )
    }

    pub fn provider_consumer() -> Self {
        let consumer_contract = contract("test.skiff/consumer");
        let provider_contract = contract("test.skiff/provider");
        let (provider, provider_file) = implementation_package(
            "test.skiff/provider-implementation",
            &provider_contract,
            &[],
            &[],
        );
        let (consumer, consumer_file) = implementation_package(
            "test.skiff/consumer-implementation",
            &consumer_contract,
            &[],
            &[("provider", &provider_contract, 0)],
        );
        let provider_deployment = deployment(
            &provider_contract,
            &provider,
            "provider-r1",
            Vec::new(),
            Vec::new(),
        );
        let provider_ref = deployment_ref(&provider_deployment);
        let mut consumer_deployment = deployment(
            &consumer_contract,
            &consumer,
            "consumer-r1",
            Vec::new(),
            vec![service_selector(&consumer, 0, &provider_contract)],
        );
        let ingress = IngressSelector {
            protocol: IngressProtocol::Http,
            host: "consumer.test".to_string(),
            method: Some("POST".to_string()),
            path: "/consume".to_string(),
        };
        add_http_ingress(
            &mut consumer_deployment,
            &consumer_contract,
            &ingress.host,
            &ingress.path,
        );
        Self::finish(
            vec![consumer_contract, provider_contract],
            vec![consumer, provider],
            vec![consumer_file, provider_file],
            vec![consumer_deployment, provider_deployment],
            ingress,
            None,
            None,
            None,
            Some(provider_ref),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn finish(
        contracts: Vec<ServiceContract>,
        packages: Vec<PackageArtifact>,
        files: Vec<FileIrUnit>,
        deployments: Vec<ServiceDeployment>,
        ingress: IngressSelector,
        direct_caller: Option<PackageArtifactRef>,
        direct_dependency: Option<PackageArtifactRef>,
        direct_callable: Option<PackageCallableId>,
        service_provider: Option<ServiceDeploymentRef>,
    ) -> Self {
        let root = deployment_ref(&deployments[0]);
        let root_contract = contract_ref(&contracts[0]);
        let root_operation = operation(&contracts[0]);
        let assembly = skiff_deployment::assembly::resolve_runtime_assembly(
            std::slice::from_ref(&root),
            &deployments,
            &contracts,
            &packages,
        )
        .expect("canonical fixture must resolve");
        let files = packages
            .iter()
            .zip(files)
            .map(|(package, file)| (package_ref(package), file))
            .collect();
        Self {
            assembly,
            contracts,
            packages,
            files,
            deployments,
            root,
            root_contract,
            root_operation,
            ingress,
            direct_caller,
            direct_dependency,
            direct_callable,
            service_provider,
        }
    }

    pub fn resolver(&self) -> FixtureResolver {
        FixtureResolver {
            contracts: self
                .contracts
                .iter()
                .map(|contract| (contract_ref(contract), Arc::new(contract.clone())))
                .collect(),
            packages: self
                .packages
                .iter()
                .map(|package| (package_ref(package), Arc::new(package.clone())))
                .collect(),
            files: self
                .files
                .iter()
                .map(|(package, file)| {
                    (
                        (package.clone(), file.file_ir_identity.clone()),
                        Arc::new(file.clone()),
                    )
                })
                .collect(),
            deployments: self
                .deployments
                .iter()
                .map(|deployment| (deployment_ref(deployment), Arc::new(deployment.clone())))
                .collect(),
        }
    }
}

#[derive(Clone)]
pub struct FixtureResolver {
    pub contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    pub packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    pub files: BTreeMap<(PackageArtifactRef, String), Arc<FileIrUnit>>,
    pub deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
}

impl RuntimeAssemblyContentResolver for FixtureResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing deployment {reference:?}"))
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

    fn resolve_package_schema_type(
        &self,
        reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("missing package schema record {reference:?}")
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

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.files
            .get(&(package.clone(), reference.file_ir_identity.clone()))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing File IR {reference:?}"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("unexpected static resource {reference:?}")
    }
}

fn implementation_package(
    package_id: &str,
    contract: &ServiceContract,
    package_dependencies: &[(&str, &PackageArtifact)],
    service_dependencies: &[(&str, &ServiceContract, u32)],
) -> (PackageArtifact, FileIrUnit) {
    let mut file = FileIrUnit::empty(format!("{}.main", package_id.replace('/', ".")), "fixture");
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "call".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    });
    for (alias, dependency) in package_dependencies {
        let callable_id = PackageCallableId::new("callable.fixture");
        let package_ref = PackageRefIr::Dependency {
            dependency_ref: (*alias).to_string(),
        };
        file.external_refs
            .package_callables
            .push(PackageCallableRef {
                package_ref: package_ref.clone(),
                package_callable_id: callable_id.clone(),
            });
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::PackageCallable {
                    package_ref,
                    package_callable_id: callable_id,
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
        assert_eq!(dependency.package_version, "1.0.0");
    }
    for (_, service, slot) in service_dependencies {
        let call = skiff_artifact_model::ServiceCallRef {
            service_requirement_slot: *slot,
            contract_operation_id: operation(service),
            expected_protocol_identity: service.service_protocol_identity.clone(),
        };
        file.external_refs.service_call_refs.push(call);
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: skiff_artifact_model::ServiceCallRefIndex::new(
                        u32::try_from(file.external_refs.service_call_refs.len() - 1)
                            .expect("fixture service call index"),
                    ),
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("File IR identity");
    let callable_id = PackageCallableId::new("callable.fixture");
    let file_ref = file_ref(&file);
    let operation_contract = contract
        .operations
        .values()
        .next()
        .expect("fixture operation")
        .contract
        .clone();
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut package = base_package(package_id, package_dependencies, service_dependencies);
    package.files = vec![file_ref.clone()];
    package.package_local_abi.public_symbols.insert(
        "call".to_string(),
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
    );
    package.callable_links.insert(
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
    );
    package.callable_semantic_facts.insert(
        callable_id.clone(),
        CallableSemanticFacts {
            effects: CallableEffectSummary::Analyzed { effects },
            provenance: provenance.clone(),
            resolved_call_targets: BTreeMap::new(),
        },
    );
    package.boundary_projections.insert(
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
    );
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("PackageArtifact identities");
    (package, file)
}

fn file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
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

fn contract(service_id: &str) -> ServiceContract {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, "call")
            .expect("contract operation identity");
    let descriptor = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: "call".to_string(),
        contract: operation_contract(),
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id, descriptor)]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: service_id.to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("ServiceContract identities");
    contract
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

fn contract_ref(contract: &ServiceContract) -> ServiceContractRef {
    ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    }
}

fn operation(contract: &ServiceContract) -> ContractOperationId {
    contract
        .operations
        .keys()
        .next()
        .expect("fixture contract operation")
        .clone()
}

fn base_package(
    package_id: &str,
    package_dependencies: &[(&str, &PackageArtifact)],
    service_dependencies: &[(&str, &ServiceContract, u32)],
) -> PackageArtifact {
    let package_requirements = package_dependencies
        .iter()
        .map(|(alias, dependency)| PackageRequirement {
            alias: (*alias).to_string(),
            package_id: dependency.package_id.clone(),
            exact_version: dependency.package_version.clone(),
            expected_local_abi: dependency.package_local_abi.local_abi_identity.clone(),
            expected_package_build: None,
        })
        .collect();
    let contract_requirements = service_dependencies
        .iter()
        .map(|(alias, contract, _)| ContractRequirement {
            alias: (*alias).to_string(),
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        })
        .collect::<Vec<_>>();
    let service_requirements = service_dependencies
        .iter()
        .zip(&contract_requirements)
        .map(|((_, contract, slot), requirement)| ServiceRequirement {
            contract_requirement: requirement.clone(),
            service_binding_slot: *slot,
            used_operations: BTreeSet::from([operation(contract)]),
        })
        .collect::<Vec<_>>();
    let service_call_refs = service_dependencies
        .iter()
        .map(|(_, contract, slot)| ServiceCallRef {
            service_requirement_slot: *slot,
            contract_operation_id: operation(contract),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        })
        .collect();
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: Vec::new(),
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
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
        callable_links: BTreeMap::new(),
        package_requirements,
        contract_requirements,
        service_requirements,
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            state: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs,
    }
}

fn package_binding(
    caller: &PackageArtifact,
    alias: &str,
    provider: &PackageArtifact,
) -> PackageBinding {
    PackageBinding {
        key: PackageRequirementKey {
            caller_package_build_id: caller.package_build_id.clone(),
            package_requirement_alias: alias.to_string(),
        },
        package: package_ref(provider),
    }
}

fn service_selector(
    caller: &PackageArtifact,
    slot: u32,
    contract: &ServiceContract,
) -> ServiceSelectorBinding {
    ServiceSelectorBinding {
        key: ServiceRequirementKey {
            caller_package_build_id: caller.package_build_id.clone(),
            service_requirement_slot: slot,
        },
        contract: contract_ref(contract),
    }
}

fn deployment(
    contract: &ServiceContract,
    implementation: &PackageArtifact,
    revision: &str,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
) -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref(contract),
        deployment_revision: DeploymentRevision::new(revision),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: package_ref(implementation),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation(contract),
            package_callable_id: PackageCallableId::new("callable.fixture"),
        }],
        package_bindings,
        service_selectors,
        ingress: Vec::new(),
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: DeploymentPolicy {
            timeout_ms: Some(1_000),
            resources: ResourcePolicy {
                cpu_millis: 100,
                memory_bytes: 1_048_576,
            },
            activation: ActivationPolicy {
                max_concurrency: 4,
                idle_timeout_ms: None,
            },
            principal: format!("service:{}", contract.service_id),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: contract.service_id.clone(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("ServiceDeployment identity");
    deployment
}

fn deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    skiff_artifact_identity::service_deployment_ref(deployment)
}

fn add_http_ingress(
    deployment: &mut ServiceDeployment,
    contract: &ServiceContract,
    host: &str,
    path: &str,
) {
    deployment.ingress.push(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            host: host.to_string(),
            method: Some("POST".to_string()),
            path: path.to_string(),
        },
        contract_operation_id: operation(contract),
    });
    skiff_artifact_identity::assign_service_deployment_identity(deployment)
        .expect("ServiceDeployment ingress identity");
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

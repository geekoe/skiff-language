use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::*;
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};
use skiff_runtime_activation::{ActivationContext, ActivationId, RequestActivationContext};
use skiff_runtime_eval::{
    RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget, RuntimeAssemblyServiceCallTarget,
};
use skiff_runtime_linked_program::{ActivationRelativeServiceCall, LinkedPackageDirectCall};

use super::super::*;

mod async_stream_cancel;
mod callback_native;
mod ordinary;

struct TypedResolver {
    deployments: Vec<(ServiceDeploymentRef, Arc<ServiceDeployment>)>,
    contracts: Vec<(ServiceContractRef, Arc<ServiceContract>)>,
    packages: Vec<(PackageArtifactRef, Arc<PackageArtifact>)>,
    files: Vec<(PackageArtifactRef, FileIrRef, Arc<FileIrUnit>)>,
}

impl RuntimeAssemblyContentResolver for TypedResolver {
    fn resolve_deployment(
        &self,
        reference: &ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<ServiceDeployment>> {
        self.deployments
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, deployment)| Arc::clone(deployment))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &ServiceContractRef,
    ) -> anyhow::Result<Arc<ServiceContract>> {
        self.contracts
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, contract)| Arc::clone(contract))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing contract"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, package)| Arc::clone(package))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.files
            .iter()
            .find(|(candidate_package, candidate_file, _)| {
                candidate_package == package && candidate_file == reference
            })
            .map(|(_, _, file)| Arc::clone(file))
            .ok_or_else(|| anyhow::anyhow!("typed execution fixture missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("typed execution fixture has no static resources")
    }
}

struct AdmittedEvalResolver {
    activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    operation_targets: BTreeMap<(ActivationId, ContractOperationId), OperationTargetRef>,
}

impl RuntimeAssemblyEvalResolver for AdmittedEvalResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        self.activations.get(activation_id).cloned()
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        self.activations
            .values()
            .find(|activation| activation.activation_id().as_str() == activation_id)
            .cloned()
    }

    fn contract(&self, contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        self.contracts.get(contract).cloned()
    }

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        self.operation_targets
            .get(&(activation_id.clone(), operation.clone()))
            .cloned()
    }
}

struct TypedExecutionFixture {
    _active: Arc<ActiveAssembly>,
    eval_target: RuntimeAssemblyEvalTarget,
    service_call: ActivationRelativeServiceCall,
    package_direct_call: LinkedPackageDirectCall,
    provider_deployment: ServiceDeploymentRef,
    provider_operation: ContractOperationId,
}

#[derive(Clone)]
struct TypedExecutionContract {
    operation: BoundaryOperationContract,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
}

impl TypedExecutionContract {
    fn new(
        operation: BoundaryOperationContract,
        boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    ) -> Self {
        Self {
            operation,
            boundary_schema,
        }
    }

    fn unary() -> Self {
        Self::new(unary_contract(), BTreeMap::new())
    }
}

impl TypedExecutionFixture {
    async fn admit() -> Self {
        Self::admit_contract(TypedExecutionContract::unary()).await
    }

    async fn admit_contract(contract: TypedExecutionContract) -> Self {
        let projected = ProjectedFixture::new(contract);
        let controller = AssemblyAdmissionController::default();
        let active = controller
            .admit(projected.assembly.clone(), &projected.resolver)
            .await
            .expect("typed provider/consumer assembly should load, link, validate, and admit");
        let eval_resolver = admitted_eval_resolver(&active);
        let consumer = eval_resolver
            .activations
            .values()
            .find(|activation| activation.identity().deployment == projected.consumer_deployment)
            .cloned()
            .expect("consumer ActivationContext should be built from admitted templates");
        let request = RequestActivationContext::begin(consumer)
            .expect("typed fixture request generation should be available");
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(eval_resolver);
        let eval_target = RuntimeAssemblyEvalTarget::new(
            Arc::clone(active.candidate().execution_image()),
            request,
            resolver,
        )
        .expect("admitted execution image and activation owner should form an eval target");
        let service_call = active
            .candidate()
            .execution_image()
            .resolve_activation_relative_service_call(
                &projected.consumer_package.package_build_id,
                &projected.consumer_file_ir_identity,
                ServiceCallRefIndex::new(0),
            )
            .expect("canonical service call should remain activation relative");
        let package_direct_call = active
            .candidate()
            .execution_image()
            .resolve_package_direct_call(
                &projected.consumer_package.package_build_id,
                &PackageRefIr::Dependency {
                    dependency_ref: "providerPackage".to_string(),
                },
                &projected.provider_callable,
            )
            .expect("canonical package call should resolve without an activation binding");
        Self {
            _active: active,
            eval_target,
            service_call,
            package_direct_call,
            provider_deployment: projected.provider_deployment,
            provider_operation: projected.provider_operation,
        }
    }

    fn resolve_provider(&self) -> RuntimeAssemblyServiceCallTarget {
        self.eval_target
            .resolve_service_call(&self.service_call)
            .expect("typed call should resolve only through the current activation binding")
    }
}

fn admitted_eval_resolver(active: &ActiveAssembly) -> AdmittedEvalResolver {
    let mut activations = BTreeMap::new();
    let mut operation_targets = BTreeMap::new();
    for (deployment, linked) in active.candidate().activations() {
        let binding_template = active
            .candidate()
            .assembly()
            .service_binding_templates
            .iter()
            .find(|template| &template.activation == deployment)
            .expect("admitted activation should retain its typed binding template");
        let activation = ActivationContext::from_assembly_templates(
            active.identity().clone(),
            active.generation(),
            "typed-execution-fixture-replica",
            linked.source(),
            binding_template,
        )
        .expect("admitted templates should construct an ActivationContext");
        for (operation, linked_operation) in linked.operations() {
            operation_targets.insert(
                (activation.activation_id().clone(), operation.clone()),
                linked_operation.target().clone(),
            );
        }
        assert!(
            activations
                .insert(activation.activation_id().clone(), activation)
                .is_none(),
            "activation ids must be unique within one admitted generation"
        );
    }
    let contracts = active
        .contract_store()
        .contracts()
        .map(|(reference, contract)| (reference.clone(), Arc::clone(contract)))
        .collect();
    AdmittedEvalResolver {
        activations,
        contracts,
        operation_targets,
    }
}

pub(super) async fn assert_typed_execution_fixture() {
    let fixture = TypedExecutionFixture::admit().await;
    fixture
        .eval_target
        .ensure_execution_ready()
        .expect("typed admitted target should be execution ready");
    fixture
        .eval_target
        .ensure_package_direct_target(&fixture.package_direct_call)
        .expect("typed package-direct call should remain inside the same execution image");
    assert_eq!(
        fixture.package_direct_call.caller_package_build_id(),
        fixture
            .eval_target
            .activation_context()
            .implementation_package_build_id()
    );
    let provider = fixture.resolve_provider();
    assert_eq!(
        provider.provider_activation().identity().deployment,
        fixture.provider_deployment
    );
    assert_eq!(
        provider.descriptor().operation_id,
        fixture.provider_operation
    );
    assert_eq!(
        provider.provider_request().generation(),
        fixture.eval_target.request_activation().generation(),
        "provider switch must preserve the explicit request generation"
    );
    assert_ne!(
        provider.provider_activation().activation_id(),
        fixture.eval_target.activation_context().activation_id(),
        "service boundary must switch to a distinct provider owner"
    );
    let provider_eval = fixture
        .eval_target
        .with_request_activation(provider.provider_request().clone())
        .expect("provider continuation should retain the same image and resolver");
    assert_eq!(
        provider_eval.activation_context().activation_id(),
        provider.provider_activation().activation_id()
    );
    assert_eq!(
        provider_eval.request_activation().generation(),
        fixture.eval_target.request_activation().generation()
    );
    let opaque_owner = fixture
        .eval_target
        .activation_by_opaque_id(provider.provider_activation().activation_id().as_str())
        .expect("callback owner lookup should use the admitted activation owner set");
    assert!(Arc::ptr_eq(&opaque_owner, provider.provider_activation()));
}

#[tokio::test]
async fn typed_execution_fixture_uses_projected_admitted_targets() {
    assert_typed_execution_fixture().await;
}

struct ProjectedFixture {
    assembly: RuntimeAssembly,
    resolver: TypedResolver,
    consumer_deployment: ServiceDeploymentRef,
    provider_deployment: ServiceDeploymentRef,
    provider_operation: ContractOperationId,
    provider_callable: PackageCallableId,
    consumer_package: PackageArtifactRef,
    consumer_file_ir_identity: String,
}

impl ProjectedFixture {
    fn new(contract_fixture: TypedExecutionContract) -> Self {
        let operation_contract = contract_fixture.operation;
        let boundary_schema = contract_fixture.boundary_schema;
        let (provider_contract, provider_operation) = service_contract(
            "example.phase-four.provider",
            "provide",
            operation_contract.clone(),
            boundary_schema.clone(),
        );
        let provider_contract_ref = contract_ref(&provider_contract);
        let (consumer_contract, consumer_operation) = service_contract(
            "example.phase-four.consumer",
            "consume",
            operation_contract.clone(),
            boundary_schema,
        );
        let consumer_contract_ref = contract_ref(&consumer_contract);

        let provider_callable = PackageCallableId::new("callable:phase-four-provider");
        let provider_file = implementation_file(
            "phase_four.provider",
            "provide",
            operation_contract.may_suspend,
            None,
            None,
        );
        let provider_file_ref = file_ref(&provider_file);
        let provider_package = implementation_package(
            "example.phase-four-provider",
            "provide",
            provider_callable.clone(),
            &provider_file,
            operation_contract.clone(),
            None,
            None,
        );
        let provider_package_ref = package_ref(&provider_package);

        let service_requirement_slot = 0;
        let service_call = ServiceCallRef {
            service_requirement_slot,
            contract_operation_id: provider_operation.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let provider_requirement = ContractRequirement {
            alias: "provider".to_string(),
            service_id: provider_contract_ref.service_id.clone(),
            contract_version: provider_contract_ref.contract_version.clone(),
            expected_protocol_identity: provider_contract_ref.service_protocol_identity.clone(),
        };
        let consumer_file = implementation_file(
            "phase_four.consumer",
            "consume",
            operation_contract.may_suspend,
            Some(service_call.clone()),
            Some(("providerPackage".to_string(), provider_callable.clone())),
        );
        let consumer_file_ref = file_ref(&consumer_file);
        let consumer_file_ir_identity = consumer_file_ref.file_ir_identity.clone();
        let consumer_package = implementation_package(
            "example.phase-four-consumer",
            "consume",
            PackageCallableId::new("callable:phase-four-consumer"),
            &consumer_file,
            operation_contract,
            Some((provider_requirement, service_call)),
            Some(("providerPackage".to_string(), provider_package_ref.clone())),
        );
        let consumer_package_ref = package_ref(&consumer_package);

        let provider_deployment_artifact = project_service_deployment(
            deployment_input(
                provider_contract_ref.clone(),
                DeploymentRevision::new("phase-four-provider-r1"),
                provider_package_ref.clone(),
                provider_operation.clone(),
                "provide",
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
            &provider_contract,
            std::slice::from_ref(&provider_package),
        )
        .expect("provider deployment should project from typed contract/package artifacts");
        let provider_deployment =
            skiff_artifact_identity::service_deployment_ref(&provider_deployment_artifact);
        let ingress = DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: "phase-four.test".to_string(),
                method: Some("POST".to_string()),
                path: "/consume".to_string(),
            },
            contract_operation_id: consumer_operation.clone(),
        };
        let consumer_deployment_artifact = project_service_deployment(
            deployment_input(
                consumer_contract_ref.clone(),
                DeploymentRevision::new("phase-four-consumer-r1"),
                consumer_package_ref.clone(),
                consumer_operation,
                "consume",
                vec![PackageBinding {
                    key: PackageRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        package_requirement_alias: "providerPackage".to_string(),
                    },
                    package: provider_package_ref.clone(),
                }],
                vec![ServiceSelectorBinding {
                    key: ServiceRequirementKey {
                        caller_package_build_id: consumer_package_ref.package_build_id.clone(),
                        service_requirement_slot,
                    },
                    contract: provider_contract_ref.clone(),
                }],
                vec![ingress],
            ),
            &consumer_contract,
            &[consumer_package.clone(), provider_package.clone()],
        )
        .expect("consumer deployment should project from typed contract/package artifacts");
        let consumer_deployment =
            skiff_artifact_identity::service_deployment_ref(&consumer_deployment_artifact);
        let assembly = resolve_runtime_assembly(
            std::slice::from_ref(&consumer_deployment),
            &[
                consumer_deployment_artifact.clone(),
                provider_deployment_artifact.clone(),
            ],
            &[consumer_contract.clone(), provider_contract.clone()],
            &[consumer_package.clone(), provider_package.clone()],
        )
        .expect("typed provider/consumer closure should resolve into a RuntimeAssembly");
        let resolver = TypedResolver {
            deployments: vec![
                (
                    consumer_deployment.clone(),
                    Arc::new(consumer_deployment_artifact),
                ),
                (
                    provider_deployment.clone(),
                    Arc::new(provider_deployment_artifact),
                ),
            ],
            contracts: vec![
                (consumer_contract_ref, Arc::new(consumer_contract)),
                (provider_contract_ref, Arc::new(provider_contract)),
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
        };
        Self {
            assembly,
            resolver,
            consumer_deployment,
            provider_deployment,
            provider_operation,
            provider_callable,
            consumer_package: consumer_package_ref,
            consumer_file_ir_identity,
        }
    }
}

fn deployment_input(
    contract: ServiceContractRef,
    deployment_revision: DeploymentRevision,
    implementation: PackageArtifactRef,
    operation: ContractOperationId,
    public_path: &str,
    package_bindings: Vec<PackageBinding>,
    service_selectors: Vec<ServiceSelectorBinding>,
    ingress: Vec<DeploymentIngressBinding>,
) -> ServiceDeploymentInput {
    ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract,
        deployment_revision,
        implementation,
        operation_bindings: vec![ServiceDeploymentOperationInput {
            contract_operation_id: operation,
            package_public_path: public_path.to_string(),
        }],
        package_bindings,
        service_selectors,
        ingress,
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
        policy: policy(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Phase four typed execution fixture".to_string(),
            notes: BTreeMap::new(),
        },
    }
}

fn service_contract(
    service_id: &str,
    stable_key: &str,
    operation_contract: BoundaryOperationContract,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
) -> (ServiceContract, ContractOperationId) {
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, stable_key)
            .expect("fixture operation identity should be canonical");
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
        boundary_schema,
        diagnostic_text: ContractDiagnosticText {
            service: "Phase four typed execution fixture".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("fixture contract should receive canonical identities");
    (contract, operation_id)
}

fn implementation_file(
    module_path: &str,
    symbol: &str,
    may_suspend: bool,
    service_call: Option<ServiceCallRef>,
    package_call: Option<(String, PackageCallableId)>,
) -> FileIrUnit {
    let mut file = FileIrUnit::empty(module_path, format!("source:{module_path}"));
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend,
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
    if let Some((dependency_ref, package_callable_id)) = package_call {
        let package_ref = PackageRefIr::Dependency { dependency_ref };
        file.external_refs
            .package_callables
            .push(PackageCallableRef {
                package_ref: package_ref.clone(),
                package_callable_id: package_callable_id.clone(),
            });
        file.executables[0].body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::PackageCallable {
                    package_ref,
                    package_callable_id,
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("fixture File IR should receive a canonical identity");
    file
}

fn implementation_package(
    package_id: &str,
    public_path: &str,
    callable_id: PackageCallableId,
    file: &FileIrUnit,
    operation_contract: BoundaryOperationContract,
    service_dependency: Option<(ContractRequirement, ServiceCallRef)>,
    package_dependency: Option<(String, PackageArtifactRef)>,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let may_suspend = operation_contract.may_suspend;
    let effects = no_effects(may_suspend);
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
    let package_requirements = package_dependency
        .into_iter()
        .map(|(alias, package)| PackageRequirement {
            alias,
            package_id: package.package_id,
            exact_version: package.package_version,
            expected_local_abi: package.package_local_abi_identity,
        })
        .collect();
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
                            local_type: TypeRefIr::native("bool"),
                        },
                        throw_types: Vec::new(),
                        may_suspend,
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
                    file_ref,
                    executable_index: 0,
                    callable_abi_id: callable_id.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        package_requirements,
        contract_requirements,
        service_requirements,
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
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("fixture package should receive canonical identities");
    package
}

fn unary_contract() -> BoundaryOperationContract {
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

fn no_effects(may_suspend: bool) -> CallableMayEffects {
    CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend,
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
        principal: "service:phase-four-fixture".to_string(),
    }
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

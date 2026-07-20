use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use skiff_artifact_model::*;
use skiff_deployment::{
    assembly::resolve_runtime_assembly, projection::project_service_deployment,
};

use super::resolver::TypedResolver;

const CALLBACK_INTERFACE_SYMBOL: &str = "CallbackProbe";
const CALLBACK_INTERFACE_METHOD: &str = "invoke";

#[derive(Clone)]
pub(super) struct TypedExecutionContract {
    operation: BoundaryOperationContract,
    boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
}

impl TypedExecutionContract {
    pub(super) fn new(
        operation: BoundaryOperationContract,
        boundary_schema: BTreeMap<ContractTypeId, ContractSchemaType>,
    ) -> Self {
        Self {
            operation,
            boundary_schema,
        }
    }

    pub(super) fn unary() -> Self {
        Self::new(unary_contract(), BTreeMap::new())
    }
}

pub(super) struct ProjectedFixture {
    pub(super) assembly: RuntimeAssembly,
    pub(super) resolver: TypedResolver,
    pub(super) consumer_deployment: ServiceDeploymentRef,
    pub(super) provider_deployment: ServiceDeploymentRef,
    pub(super) provider_operation: ContractOperationId,
    pub(super) provider_callable: PackageCallableId,
    pub(super) consumer_package: PackageArtifactRef,
    pub(super) consumer_file_ir_identity: String,
}

impl ProjectedFixture {
    pub(super) fn new(contract_fixture: TypedExecutionContract) -> Self {
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
    let mut entry = ExecutableIr {
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
    };
    if let Some(service_call) = service_call {
        file.external_refs.service_call_refs.push(service_call);
        entry.body.expressions.push(ExprIr::Call {
            call: CallIr {
                target: CallTargetIr::ServiceCall {
                    service_call_ref_index: ServiceCallRefIndex::new(0),
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        });
        entry.body.statements.push(StmtIr::Expr {
            value: ExprRefIr { expression: 0 },
        });
        entry.body.blocks.push(BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }],
        });
    }
    file.executables.push(entry);
    if let Some((dependency_ref, package_callable_id)) = package_call {
        let package_ref = PackageRefIr::Dependency { dependency_ref };
        file.external_refs
            .package_callables
            .push(PackageCallableRef {
                package_ref: package_ref.clone(),
                package_callable_id: package_callable_id.clone(),
            });
        file.executables.push(checkpoint_call_executable(
            format!("{symbol}_package_direct"),
            CallTargetIr::PackageCallable {
                package_ref,
                package_callable_id,
            },
            Vec::new(),
        ));
        let callback_interface = callback_interface_ref(module_path);
        let callback_method_abi_id = skiff_artifact_identity::canonical_interface_method_abi_id(
            &callback_interface,
            CALLBACK_INTERFACE_METHOD,
        );
        file.declarations.types.insert(
            CALLBACK_INTERFACE_SYMBOL.to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: format!("{module_path}.{CALLBACK_INTERFACE_SYMBOL}"),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            CALLBACK_INTERFACE_SYMBOL.to_string(),
            InterfaceDeclIr {
                name: CALLBACK_INTERFACE_SYMBOL.to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: CALLBACK_INTERFACE_METHOD.to_string(),
                    type_params: Vec::new(),
                    params: vec![FunctionTypeParamIr {
                        name: "self".to_string(),
                        ty: TypeRefIr::native("Self"),
                    }],
                    return_type: TypeRefIr::native("bool"),
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        file.type_table.push(TypeDeclIr {
            name: CALLBACK_INTERFACE_SYMBOL.to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::new(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.executables.push(callback_checkpoint_executable(
            format!("{symbol}_callback"),
            callback_interface,
            callback_method_abi_id,
        ));
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("fixture File IR should receive a canonical identity");
    file
}

fn checkpoint_call_executable(
    symbol: String,
    target: CallTargetIr,
    args: Vec<ExprRefIr>,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            }],
            expressions: vec![ExprIr::Call {
                call: CallIr {
                    target,
                    args,
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            }],
        },
        source_span: None,
    }
}

fn callback_checkpoint_executable(
    symbol: String,
    interface: InterfaceInstantiationRef,
    method_abi_id: String,
) -> ExecutableIr {
    let mut executable = checkpoint_call_executable(
        symbol,
        CallTargetIr::InterfaceMethod {
            interface: interface.clone(),
            method_abi_id,
            slot: 0,
        },
        vec![ExprRefIr { expression: 0 }],
    );
    executable.params.push(ParamIr {
        name: "callback".to_string(),
        slot: 0,
        ty: TypeRefIr::AnyInterface { interface },
    });
    executable.slots = SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "callback".to_string(),
            kind: SlotKind::Param,
        }],
        frame_size: 1,
    };
    executable
        .body
        .expressions
        .insert(0, ExprIr::LoadSlot { slot: 0 });
    executable.body.statements[0] = StmtIr::Expr {
        value: ExprRefIr { expression: 1 },
    };
    executable
}

pub(super) fn callback_interface_ref(module_path: &str) -> InterfaceInstantiationRef {
    skiff_artifact_identity::interface_instantiation_ref(
        TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: CALLBACK_INTERFACE_SYMBOL.to_string(),
            },
        },
        Vec::new(),
    )
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

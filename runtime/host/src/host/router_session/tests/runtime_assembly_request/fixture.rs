use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::*;

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

#[path = "../../../../loader/assembly_admission/tests/execution/resolver.rs"]
mod resolver;

// Reuse the checked-in nested-provider builder without editing loader-owned fixtures.
#[allow(dead_code)]
#[path = "../../../../loader/assembly_admission/tests/execution/artifacts.rs"]
mod nested_artifacts;

use nested_artifacts::{ProjectedFixture, TypedExecutionContract};
use resolver::TypedResolver;

pub(super) async fn admitted_nested_host() -> (RuntimeHost, ActiveAssemblyRoute) {
    let projected = ProjectedFixture::new(TypedExecutionContract::unary());
    admit(projected.assembly, projected.resolver).await
}

pub(super) async fn admitted_void_host(may_suspend: bool) -> (RuntimeHost, ActiveAssemblyRoute) {
    let fixture = void_fixture(may_suspend, false);
    admit(fixture.assembly, fixture.resolver).await
}

pub(super) async fn admitted_spawn_host() -> (RuntimeHost, ActiveAssemblyRoute) {
    let fixture = void_fixture(true, true);
    admit(fixture.assembly, fixture.resolver).await
}

pub(super) async fn reloaded_nested_host() -> (RuntimeHost, ActiveAssemblyRoute, ActiveAssemblyRoute)
{
    let projected = ProjectedFixture::new(TypedExecutionContract::unary());
    let selector = projected.assembly.global_ingress[0].selector.clone();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(projected.assembly.clone(), &projected.resolver)
        .await
        .expect("generation one should admit");
    let pinned = host
        .lookup_active_assembly_request_route(&selector)
        .expect("generation one route");
    host.assembly_admission
        .admit(projected.assembly, &projected.resolver)
        .await
        .expect("generation two should admit");
    let current = host
        .lookup_active_assembly_request_route(&selector)
        .expect("generation two route");
    (host, pinned, current)
}

pub(crate) async fn reloaded_websocket_host(
) -> (RuntimeHost, ActiveAssemblyRoute, ActiveAssemblyRoute) {
    let mut projected = ProjectedFixture::new_with_consumer_service_id(
        TypedExecutionContract::unary(),
        "example.com/consumer",
    );
    let consumer_index = projected
        .resolver
        .deployments
        .iter()
        .position(|(reference, _)| reference == &projected.consumer_deployment)
        .expect("consumer deployment fixture");
    let mut consumer = projected.resolver.deployments[consumer_index]
        .1
        .as_ref()
        .clone();
    consumer.ingress[0].selector.protocol = IngressProtocol::WebSocket;
    consumer.ingress[0].selector.method = None;
    skiff_artifact_identity::assign_service_deployment_identity(&mut consumer)
        .expect("WebSocket deployment identity");
    let consumer_ref = skiff_artifact_identity::service_deployment_ref(&consumer);
    projected.resolver.deployments[consumer_index] = (consumer_ref.clone(), Arc::new(consumer));
    let deployments = projected
        .resolver
        .deployments
        .iter()
        .map(|(_, deployment)| deployment.as_ref().clone())
        .collect::<Vec<_>>();
    let contracts = projected
        .resolver
        .contracts
        .iter()
        .map(|(_, contract)| contract.as_ref().clone())
        .collect::<Vec<_>>();
    let packages = projected
        .resolver
        .packages
        .iter()
        .map(|(_, package)| package.as_ref().clone())
        .collect::<Vec<_>>();
    projected.assembly = skiff_deployment::assembly::resolve_runtime_assembly(
        std::slice::from_ref(&consumer_ref),
        &deployments,
        &contracts,
        &packages,
    )
    .expect("WebSocket RuntimeAssembly should resolve");
    let selector = projected.assembly.global_ingress[0].selector.clone();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(projected.assembly.clone(), &projected.resolver)
        .await
        .expect("WebSocket generation one should admit");
    let pinned = host
        .lookup_active_assembly_request_route(&selector)
        .expect("WebSocket generation one route");
    host.assembly_admission
        .admit(projected.assembly, &projected.resolver)
        .await
        .expect("WebSocket generation two should admit");
    let current = host
        .lookup_active_assembly_request_route(&selector)
        .expect("WebSocket generation two route");
    (host, pinned, current)
}

async fn admit(
    assembly: RuntimeAssembly,
    resolver: TypedResolver,
) -> (RuntimeHost, ActiveAssemblyRoute) {
    let selector = assembly.global_ingress[0].selector.clone();
    let host = super::super::test_host();
    host.assembly_admission
        .admit(assembly, &resolver)
        .await
        .expect("canonical request assembly should admit");
    let route = host
        .lookup_active_assembly_request_route(&selector)
        .expect("canonical ingress should have one active route");
    (host, route)
}

struct VoidFixture {
    assembly: RuntimeAssembly,
    resolver: TypedResolver,
}

fn void_fixture(may_suspend: bool, submits_spawn: bool) -> VoidFixture {
    let operation_contract = void_unary_contract(may_suspend);
    let (contract, operation_id) = void_contract(operation_contract.clone());
    let contract_ref = ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    };
    let file = void_file(may_suspend, submits_spawn);
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let package = void_package(&file_ref, operation_contract, may_suspend);
    let package_ref = PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    };
    let deployment = void_deployment(
        contract_ref.clone(),
        package_ref.clone(),
        operation_id,
        &contract,
        &package,
    );
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);
    let assembly = skiff_deployment::assembly::resolve_runtime_assembly(
        std::slice::from_ref(&deployment_ref),
        std::slice::from_ref(&deployment),
        std::slice::from_ref(&contract),
        std::slice::from_ref(&package),
    )
    .expect("void assembly should resolve");
    VoidFixture {
        assembly,
        resolver: TypedResolver {
            deployments: vec![(deployment_ref, Arc::new(deployment))],
            contracts: vec![(contract_ref, Arc::new(contract))],
            packages: vec![(package_ref.clone(), Arc::new(package))],
            files: vec![(package_ref, file_ref, Arc::new(file))],
            package_schema_records: Vec::new(),
        },
    }
}

fn void_contract(
    operation_contract: BoundaryOperationContract,
) -> (ServiceContract, ContractOperationId) {
    let service_id = "example.canonical-void";
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, "invoke")
            .expect("void operation identity");
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(
            operation_id.clone(),
            BoundaryOperationDescriptor {
                operation_id: operation_id.clone(),
                stable_key: "invoke".to_string(),
                contract: operation_contract.clone(),
            },
        )]),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "Canonical void fixture".to_string(),
            operations: BTreeMap::new(),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract)
        .expect("void contract identities");
    (contract, operation_id)
}

fn void_file(may_suspend: bool, submits_spawn: bool) -> FileIrUnit {
    let mut file = FileIrUnit::empty("canonical.void", "source:canonical.void".to_string());
    let (statements, expressions) = if submits_spawn {
        (
            vec![
                StmtIr::Spawn {
                    call: ExprRefIr { expression: 0 },
                },
                StmtIr::Return { value: None },
            ],
            vec![ExprIr::Call {
                call: CallIr {
                    target: CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::from([(
                        "spawnSubmit".to_string(),
                        MetadataValue::Object(BTreeMap::from([
                            (
                                "targetKind".to_string(),
                                MetadataValue::String("function".to_string()),
                            ),
                            (
                                "target".to_string(),
                                MetadataValue::String(
                                    "function:canonical.void.spawnTarget".to_string(),
                                ),
                            ),
                        ])),
                    )]),
                },
            }],
        )
    } else {
        (vec![StmtIr::Return { value: None }], Vec::new())
    };
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "invoke".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: (0..statements.len())
                    .map(|statement| StmtRefIr {
                        statement: statement as u32,
                    })
                    .collect(),
            }],
            statements,
            expressions,
        },
        source_span: None,
    });
    if submits_spawn {
        file.executables.push(ExecutableIr {
            kind: ExecutableKind::Function,
            symbol: "canonical.void.spawnTarget".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            slots: SlotLayout::default(),
            may_suspend: false,
            body: ExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }],
                }],
                statements: vec![StmtIr::Return { value: None }],
                expressions: Vec::new(),
            },
            source_span: None,
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).expect("void File IR identity");
    file
}

fn void_package(
    file_ref: &FileIrRef,
    operation_contract: BoundaryOperationContract,
    may_suspend: bool,
) -> PackageArtifact {
    let callable_id = PackageCallableId::new("callable:canonical-void");
    let effects = CallableMayEffects {
        writes_caller_reachable: false,
        returns_caller_alias: false,
        throws_caller_alias: false,
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_suspend,
    };
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: "example.canonical-void-package".to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                "invoke".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: callable_id.clone(),
                    signature: PackageCallableSignature {
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("void"),
                        },
                        throw_types: Vec::new(),
                        may_suspend,
                    },
                },
            )]),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: "example.canonical-void-package".to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                "example.canonical-void-package",
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
            callable_id.clone(),
            BoundaryCallableProjection::Available {
                operation_contract: operation_contract.clone(),
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
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("void package identities");
    package
}

fn void_deployment(
    contract_ref: ServiceContractRef,
    package_ref: PackageArtifactRef,
    operation_id: ContractOperationId,
    contract: &ServiceContract,
    package: &PackageArtifact,
) -> ServiceDeployment {
    let deployment_input = ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: contract_ref,
        deployment_revision: DeploymentRevision::new("canonical-void-r1"),
        implementation: package_ref,
        operation_bindings: vec![ServiceDeploymentOperationInput {
            contract_operation_id: operation_id.clone(),
            package_public_path: "invoke".to_string(),
        }],
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        ingress: vec![DeploymentIngressBinding {
            selector: IngressSelector {
                protocol: IngressProtocol::Http,
                host: "canonical-void.test".to_string(),
                method: Some("POST".to_string()),
                path: "/invoke".to_string(),
            },
            contract_operation_id: operation_id,
        }],
        config_literals: Vec::new(),
        secret_refs: Vec::new(),
        state_bindings: Vec::new(),
        resource_bindings: Vec::new(),
        runtime_capability_bindings: Vec::new(),
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
            principal: "service:canonical-void".to_string(),
        },
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Canonical void fixture".to_string(),
            notes: BTreeMap::new(),
        },
    };
    let deployment = skiff_deployment::projection::project_service_deployment(
        deployment_input,
        contract,
        std::slice::from_ref(package),
        &BTreeMap::new(),
    )
    .expect("void deployment should project");
    deployment
}

fn void_unary_contract(may_suspend: bool) -> BoundaryOperationContract {
    BoundaryOperationContract {
        parameters: Vec::new(),
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("void"),
            value_plan: BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::DetachedValueGraph,
                encoding: BoundaryValueEncoding::CanonicalValue,
                owner: BoundaryValueOwner::Provider,
                lifetime: BoundaryValueLifetime::Call,
            },
        },
        errors: BoundaryErrorContract::None,
        stream: BoundaryStreamContract::Unary,
        cancellation: if may_suspend {
            BoundaryCancellationContract::Cooperative
        } else {
            BoundaryCancellationContract::NotCancellable
        },
        callbacks: BoundaryCallbackContract::None,
        may_suspend,
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

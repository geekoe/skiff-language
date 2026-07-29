use std::{
    sync::Arc,
    task::{Poll, Wake, Waker},
    time::Duration,
};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_runtime_capability_context::StreamPoll;
use skiff_runtime_linked_program::{
    ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay,
    LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedCallTarget, LinkedExprIr,
    LinkedFileUnit, LinkedTypeRef, PublicationResourceTable, RuntimeTypeContext, ServiceSymbolRef,
    SourceMapDto, UnitAddr,
};
use skiff_runtime_model::request_heap::RequestHeap;

use super::*;
use crate::{
    actor_executor::ActorExecutionFrame,
    actor_instance::{
        ActorActivationRequest, ActorExecutorAuthority, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceHandle, ActorInstanceStore, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    assembly_execution::{
        ordinary::tests::{
            service_error_consumer::{
                ConsumerTopology, ProviderFailureKind, ServiceErrorConsumerFixture,
            },
            test_runtime,
        },
        RuntimeAssemblyExecutionProjection,
    },
    env::Env,
    error::RuntimeError,
    EvalRuntimeProgram, Interpreter,
};

const ACTOR_FILE: &str = "file:f445h-e4r-activation-actor";

struct ActorFrameFixture {
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
}

impl ActorFrameFixture {
    fn new() -> Self {
        let owner = actor_owner();
        let file = Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: ACTOR_FILE.to_string(),
            source_ast_hash: "source:f445h-e4r-activation-actor".to_string(),
            module_path: "actors".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: SourceMapDto::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            actor_declarations: vec![LinkedActorDeclaration {
                actor_type: ServiceSymbolRef {
                    module_path: "actors".to_string(),
                    symbol: "ActivationProbe".to_string(),
                },
                implementation_owner: Some(owner.clone()),
                actor_abi_identity: actor_abi(),
                actor_implementation_identity: actor_implementation(),
                actor_name: "ActivationProbe".to_string(),
                actor_id_type: LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
                fields: Vec::new(),
                public_methods: Vec::new(),
                actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
            }],
            types: Vec::new(),
            constants: Vec::new(),
            executables: Vec::new(),
            external_refs: ExternalRefTable::default(),
        });
        let program = Arc::new(EvalRuntimeProgram::new(
            "skiff.run/f445h-e4r-activation",
            vec![file],
            Vec::new(),
            Vec::new(),
            PublicationResourceTable::default(),
            Vec::new(),
            Default::default(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let store = ActorInstanceStore::new();
        let id = br#""activation-probe""#.to_vec();
        let handle = store
            .activate(ActorActivationRequest {
                fence: ActorInstanceFence {
                    incarnation: ActorIncarnationKey {
                        logical_key: ActorLogicalKey {
                            service_id: "skiff.run/f445h-e4r-activation".to_string(),
                            actor_type_identity: "actors.ActivationProbe".to_string(),
                            actor_id_type_identity: "builtin:string".to_string(),
                            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                            actor_id_hash: "sha256:a9d57d9dc2127eaf51681c67636a67bfd14056cf6f4ee552f48d3a8c5a306420".to_string(),
                            canonical_actor_id_key_bytes: id,
                        },
                        epoch: 1,
                    },
                    actor_abi_identity: actor_abi(),
                    actor_implementation_identity: actor_implementation(),
                    declaration_owner: owner,
                },
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: b"{}",
                program: program.projection().type_view(),
            })
            .expect("activate fieldless Actor probe");
        Self { store, handle }
    }

    async fn frame(&self) -> (ActorExecutionFrame, RequestHeap) {
        let authority = ActorExecutorAuthority::new();
        let mut lease = self
            .store
            .acquire_execution(&authority, &self.handle)
            .await
            .expect("acquire Actor probe");
        let heap = lease.take_heap();
        (
            ActorExecutionFrame::new(self.store.clone(), self.handle.clone(), lease, Vec::new()),
            heap,
        )
    }
}

fn actor_owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(ACTOR_FILE.to_string()),
        actor_symbol: "ActivationProbe".to_string(),
    }
}

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:f445h-e4r-activation")
}

fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:f445h-e4r-activation")
}

fn unary_fixture() -> (
    ServiceErrorConsumerFixture,
    Interpreter,
    RuntimeAssemblyExecutionProjection,
    skiff_runtime_linked_program::CallIr,
) {
    let fixture = ServiceErrorConsumerFixture::new(
        ProviderFailureKind::PublicRecord,
        ConsumerTopology::OneHop,
        true,
        false,
    );
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let target = fixture.caller_eval_target();
    let projection =
        RuntimeAssemblyExecutionProjection::from_image(Arc::clone(target.execution_image()));
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked activation-relative caller");
    let call = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call }
                if matches!(
                    call.target,
                    LinkedCallTarget::ActivationRelativeService { .. }
                ) =>
            {
                Some(call.clone())
            }
            _ => None,
        })
        .expect("activation-relative unary call");
    (fixture, interpreter, projection, call)
}

mod server_stream_fixture {
    use std::collections::{BTreeMap, BTreeSet};

    use skiff_artifact_model as artifact;
    use skiff_runtime_activation::{
        ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
        ActivationServiceBinding, RequestActivationContext, RuntimeActivation,
    };
    use skiff_runtime_capability_context::DbCapabilityContext;
    use skiff_runtime_linked_program::{ExecutableAddr, FileAddr, ServiceMeta, UnitAddr};
    use skiff_runtime_model::request_heap::RequestHeapLimits;

    use super::*;
    use crate::{
        capabilities::TimeCapabilityContext,
        program_execution::{ProgramExecutionContext, ProgramExecutionInput},
        RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
    };

    const CALLER_PACKAGE: &str = "example.f445h.activation-stream-caller";
    const PROVIDER_PACKAGE: &str = "example.f445h.activation-stream-provider";
    const SERVICE_ID: &str = "example.f445h.activation-stream-service";
    const OPERATION_ID: &str = "operation:f445h-e4r:activation-stream";

    pub(super) struct Fixture {
        pub(super) target: RuntimeAssemblyEvalTarget,
        pub(super) caller_addr: ExecutableAddr,
    }

    struct Resolver {
        activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
        contract: Arc<artifact::ServiceContract>,
        contract_ref: artifact::ServiceContractRef,
        operation: artifact::ContractOperationId,
        provider: ActivationId,
        target: artifact::OperationTargetRef,
    }

    impl RuntimeAssemblyEvalResolver for Resolver {
        fn activation(&self, id: &ActivationId) -> Option<Arc<ActivationContext>> {
            self.activations.get(id).cloned()
        }

        fn activation_by_opaque_id(&self, id: &str) -> Option<Arc<ActivationContext>> {
            self.activations
                .values()
                .find(|activation| activation.activation_id().as_str() == id)
                .cloned()
        }

        fn contract(
            &self,
            contract: &artifact::ServiceContractRef,
        ) -> Option<Arc<artifact::ServiceContract>> {
            (contract == &self.contract_ref).then(|| Arc::clone(&self.contract))
        }

        fn admitted_schema_records(
            &self,
            contract: &artifact::ServiceContractRef,
        ) -> Option<crate::AdmittedPackageSchemaRecords> {
            (contract == &self.contract_ref).then(|| Arc::new(BTreeMap::new()))
        }

        fn operation_target(
            &self,
            activation: &ActivationId,
            operation: &artifact::ContractOperationId,
        ) -> Option<artifact::OperationTargetRef> {
            (activation == &self.provider && operation == &self.operation)
                .then(|| self.target.clone())
        }
    }

    pub(super) fn fixture() -> Fixture {
        let contract = Arc::new(service_contract());
        let contract_ref = contract_ref(&contract);
        let operation = artifact::ContractOperationId::new(OPERATION_ID);
        let service_call = artifact::ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: operation.clone(),
            expected_protocol_identity: contract_ref.service_protocol_identity.clone(),
        };
        let caller_file = caller_file(&service_call);
        let provider_file = provider_file();

        let requirement = contract_requirement(&contract_ref);
        let mut caller_package = private_package(CALLER_PACKAGE, &caller_file);
        caller_package
            .contract_requirements
            .push(requirement.clone());
        caller_package
            .service_requirements
            .push(artifact::ServiceRequirement {
                contract_requirement: requirement,
                service_binding_slot: 0,
                used_operations: BTreeSet::from([operation.clone()]),
            });
        caller_package.service_call_refs.push(service_call);
        skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
            .expect("activation stream caller package identities");
        let caller_ref = package_ref(&caller_package);

        let mut provider_package = private_package(PROVIDER_PACKAGE, &provider_file);
        skiff_artifact_identity::assign_package_artifact_identities(&mut provider_package)
            .expect("activation stream provider package identities");
        let provider_callable = artifact::PackageCallableId::new(OPERATION_ID);
        let receiver_type = artifact::TypeRefIr::builtin("string");
        let provider_target = artifact::OperationTargetRef {
            file_ref: file_ref(&provider_file),
            executable_index: 0,
            callable_abi_id: provider_callable.to_string(),
            callable_kind: artifact::OperationCallableKind::ImplMethod,
        };
        provider_package.implementation_links.constants.insert(
            "worker".to_string(),
            artifact::ConstExport {
                file: file_ref(&provider_file),
                const_index: 0,
                symbol: "worker".to_string(),
                ty: receiver_type.clone(),
            },
        );
        provider_package.package_local_abi.public_symbols.insert(
            "worker".to_string(),
            artifact::PackageLocalAbiSymbol::PublicInstance {
                instance_id: "worker".to_string(),
                declared_receiver_type: receiver_type,
                interfaces: vec![artifact::TypeRefIr::builtin("EventSource")],
                methods: BTreeMap::from([("events".to_string(), provider_callable.clone())]),
            },
        );
        provider_package.callable_links.insert(
            provider_callable.clone(),
            artifact::PackageCallableLinkFact {
                callable_id: provider_callable,
                target: provider_target.clone(),
            },
        );
        let provider_ref = package_ref(&provider_package);

        let assembly_identity =
            artifact::AssemblyIdentity::new("assembly:f445h-e4r-activation-stream");
        let assembly = artifact::RuntimeAssembly {
            schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: assembly_identity.clone(),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![caller_ref.clone(), provider_ref.clone()],
            package_link_plan: artifact::CanonicalPackageLinkPlan {
                code_slots: vec![
                    artifact::PackageCodeSlot {
                        package: caller_ref.clone(),
                    },
                    artifact::PackageCodeSlot {
                        package: provider_ref.clone(),
                    },
                ],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let image = crate::test_support::link_package_fixture(
            assembly,
            vec![
                (caller_package, vec![caller_file.clone()]),
                (provider_package, vec![provider_file.clone()]),
            ],
        );
        let provider = ActivationContext::new(
            activation_identity(
                assembly_identity.clone(),
                SERVICE_ID,
                "activation-stream-provider-r1",
            ),
            provider_ref.package_build_id.clone(),
            activation_bindings(),
            Vec::new(),
        )
        .expect("activation stream provider activation");
        let binding = ActivationServiceBinding::new(
            artifact::ServiceRequirementKey {
                caller_package_build_id: caller_ref.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            provider.activation_id().clone(),
            contract_ref.clone(),
            vec![operation.clone()],
        )
        .expect("activation stream service binding");
        let caller = ActivationContext::new(
            activation_identity(
                assembly_identity,
                CALLER_PACKAGE,
                "activation-stream-caller-r1",
            ),
            caller_ref.package_build_id.clone(),
            activation_bindings(),
            vec![binding],
        )
        .expect("activation stream caller activation");
        let activations = BTreeMap::from([
            (caller.activation_id().clone(), Arc::clone(&caller)),
            (provider.activation_id().clone(), Arc::clone(&provider)),
        ]);
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(Resolver {
            activations,
            contract,
            contract_ref,
            operation,
            provider: provider.activation_id().clone(),
            target: provider_target,
        });
        let request =
            RequestActivationContext::begin(Arc::clone(&caller)).expect("stream caller request");
        let target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
            .expect("activation stream eval target");
        Fixture {
            target,
            caller_addr: ExecutableAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::LoadedFileIndex(0),
                executable: 0,
            },
        }
    }

    pub(super) fn execution_context<'a>(
        interpreter: &Interpreter,
        target: RuntimeAssemblyEvalTarget,
    ) -> ProgramExecutionContext<'a> {
        let execution = test_runtime::execution_control();
        let effects = test_runtime::effects_context();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: test_runtime::config_context(),
            db: DbCapabilityContext::unavailable(),
            file: test_runtime::file_context(),
            file_source_stream: test_runtime::file_source_stream_context(
                interpreter.stream_runtime.clone(),
            ),
            time: TimeCapabilityContext::new(execution),
            websocket: test_runtime::websocket_context(),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                interpreter.http_options.clone(),
                interpreter.stream_runtime.clone(),
                interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: interpreter.test_effect_double_context(),
            runtime_activation: Arc::new(RuntimeActivation {
                service: ServiceMeta {
                    id: CALLER_PACKAGE.to_string(),
                    display_name: None,
                    metadata: BTreeMap::new(),
                },
                version: "1.0.0".to_string(),
                package_configs: Vec::new(),
                service_dependencies: Vec::new(),
                timeout: Default::default(),
                operation_route_bindings: Vec::new(),
                db: Vec::new(),
                actors: Vec::new(),
                gateway: Default::default(),
            }),
            actor: test_runtime::actor_context(),
            spawn: test_runtime::actor_context(),
            outbound: test_runtime::outbound_context(),
            request_heap_limits: RequestHeapLimits::default(),
        })
        .with_websocket_capability_rebinder(test_runtime::websocket_rebinder())
        .with_runtime_assembly_target(target)
    }

    fn provider_file() -> artifact::FileIrUnit {
        let mut file = artifact::FileIrUnit::empty(
            "activation_stream.provider",
            "source:f445h-e4r-activation-stream-provider",
        );
        file.constants.push(artifact::ConstIr {
            name: "worker".to_string(),
            ty: artifact::TypeRefIr::builtin("string"),
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![artifact::StmtRefIr { statement: 0 }],
                }],
                statements: vec![artifact::StmtIr::Return {
                    value: Some(artifact::ExprRefIr { expression: 0 }),
                }],
                expressions: vec![artifact::ExprIr::Literal {
                    value: artifact::LiteralIr::String {
                        value: "receiver-stream-item".to_string(),
                    },
                }],
            },
            source_span: None,
        });
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::ImplMethod,
            symbol: "Worker.events".to_string(),
            type_params: Vec::new(),
            params: vec![artifact::ParamIr {
                name: "self".to_string(),
                slot: 0,
                ty: artifact::TypeRefIr::builtin("string"),
            }],
            return_type: artifact::TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![artifact::TypeRefIr::builtin("string")],
            },
            self_type: Some(artifact::TypeRefIr::builtin("string")),
            slots: artifact::SlotLayout {
                slots: vec![artifact::SlotIr {
                    index: 0,
                    name: "self".to_string(),
                    kind: artifact::SlotKind::SelfValue,
                }],
                frame_size: 1,
            },
            may_suspend: true,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![
                        artifact::StmtRefIr { statement: 0 },
                        artifact::StmtRefIr { statement: 1 },
                    ],
                }],
                statements: vec![
                    artifact::StmtIr::Emit {
                        operation: "events".to_string(),
                        value: artifact::ExprRefIr { expression: 0 },
                    },
                    artifact::StmtIr::Return { value: None },
                ],
                expressions: vec![artifact::ExprIr::LoadSlot { slot: 0 }],
            },
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("activation stream provider file identity");
        file
    }

    fn caller_file(service_call: &artifact::ServiceCallRef) -> artifact::FileIrUnit {
        let mut file = artifact::FileIrUnit::empty(
            "activation_stream.caller",
            "source:f445h-e4r-activation-stream-caller",
        );
        file.external_refs
            .service_call_refs
            .push(service_call.clone());
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::Function,
            symbol: "call_stream".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: artifact::TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![artifact::TypeRefIr::builtin("string")],
            },
            self_type: None,
            slots: artifact::SlotLayout::default(),
            may_suspend: true,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![artifact::StmtRefIr { statement: 0 }],
                }],
                statements: vec![artifact::StmtIr::Return {
                    value: Some(artifact::ExprRefIr { expression: 0 }),
                }],
                expressions: vec![artifact::ExprIr::Call {
                    call: artifact::CallIr {
                        target: artifact::CallTargetIr::ServiceCall {
                            service_call_ref_index: artifact::ServiceCallRefIndex::new(0),
                        },
                        site: instruction_site(),
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                }],
            },
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("activation stream caller file identity");
        file
    }

    fn service_contract() -> artifact::ServiceContract {
        let operation = artifact::ContractOperationId::new(OPERATION_ID);
        artifact::ServiceContract {
            schema_version: artifact::SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: SERVICE_ID.to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: artifact::ServiceProtocolIdentity::new(
                "protocol:f445h-e4r-activation-stream",
            ),
            operations: BTreeMap::from([(
                operation.clone(),
                artifact::BoundaryOperationDescriptor {
                    operation_id: operation.clone(),
                    stable_key: "stream".to_string(),
                    contract: artifact::BoundaryOperationContract {
                        parameters: Vec::new(),
                        return_value: artifact::BoundaryReturn {
                            ty: artifact::ContractTypeRef::builtin("void"),
                            value_plan: detached_plan(
                                artifact::BoundaryValueOwner::Provider,
                                artifact::BoundaryValueLifetime::Call,
                            ),
                        },
                        stream: artifact::BoundaryStreamContract::ServerStream {
                            item_type: artifact::ContractTypeRef::builtin("string"),
                            item_value_plan: detached_plan(
                                artifact::BoundaryValueOwner::Provider,
                                artifact::BoundaryValueLifetime::Stream,
                            ),
                        },
                        callbacks: artifact::BoundaryCallbackContract::None,
                        effect_guarantee: artifact::BoundaryEffectGuarantee {
                            detached_parameters: true,
                            detached_return: true,
                            detached_error: true,
                            no_caller_reachable_mutation: true,
                            no_caller_value_escape: true,
                            no_same_heap_identity: true,
                        },
                    },
                },
            )]),
            package_type_requirements: Vec::new(),
            diagnostic_text: artifact::ContractDiagnosticText {
                service: "activation server stream fixture".to_string(),
                operations: BTreeMap::from([(operation, "stream".to_string())]),
                types: BTreeMap::new(),
            },
        }
    }

    fn detached_plan(
        owner: artifact::BoundaryValueOwner,
        lifetime: artifact::BoundaryValueLifetime,
    ) -> artifact::BoundaryValuePlan {
        artifact::BoundaryValuePlan::Linkable {
            carrier: artifact::BoundaryValueCarrier::DetachedValueGraph,
            encoding: artifact::BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime,
        }
    }

    fn contract_ref(contract: &artifact::ServiceContract) -> artifact::ServiceContractRef {
        artifact::ServiceContractRef {
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            service_protocol_identity: contract.service_protocol_identity.clone(),
        }
    }

    fn contract_requirement(
        contract: &artifact::ServiceContractRef,
    ) -> artifact::ContractRequirement {
        artifact::ContractRequirement {
            alias: "stream".to_string(),
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        }
    }

    fn file_ref(file: &artifact::FileIrUnit) -> artifact::FileIrRef {
        artifact::FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }
    }

    fn package_ref(package: &artifact::PackageArtifact) -> artifact::PackageArtifactRef {
        artifact::PackageArtifactRef {
            package_id: package.package_id.clone(),
            package_version: package.package_version.clone(),
            package_build_id: package.package_build_id.clone(),
            package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
        }
    }

    fn private_package(package_id: &str, file: &artifact::FileIrUnit) -> artifact::PackageArtifact {
        artifact::PackageArtifact {
            schema_version: artifact::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: artifact::PackageBuildId::new("unassigned"),
            files: vec![file_ref(file)],
            static_resources: Vec::new(),
            package_local_abi: artifact::PackageLocalAbi {
                local_abi_identity: artifact::PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: artifact::PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        package_id,
                        &BTreeMap::new(),
                    )
                    .expect("activation stream schema index"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: artifact::PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: artifact::PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
        }
    }

    fn activation_bindings() -> ActivationOwnedBindings {
        ActivationOwnedBindings {
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            policy: artifact::DeploymentPolicy {
                timeout_ms: Some(1_000),
                resources: artifact::ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_048_576,
                },
                activation: artifact::ActivationPolicy {
                    max_concurrency: 1,
                    idle_timeout_ms: None,
                },
                principal: "test".to_string(),
            },
        }
    }

    fn activation_identity(
        assembly_identity: artifact::AssemblyIdentity,
        service_id: &str,
        revision: &str,
    ) -> ActivationIdentity {
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 1,
            runtime_replica_id: "replica:f445h-e4r-activation-stream".to_string(),
            deployment: artifact::ServiceDeploymentRef {
                service_id: service_id.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: artifact::DeploymentRevision::new(revision),
                deployment_artifact_identity: artifact::DeploymentArtifactIdentity::new(format!(
                    "deployment:f445h-e4r:{revision}"
                )),
            },
        }
    }

    fn instruction_site() -> artifact::InstructionSourceSite {
        artifact::InstructionSourceSite::Synthetic {
            reason: artifact::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        }
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_once<F: std::future::Future>(future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = std::task::Context::from_waker(&waker);
    future.poll(&mut context)
}

#[tokio::test]
async fn f445h_e4r_stream_activation_unary_ready_keeps_actor_segment() {
    let actor = ActorFrameFixture::new();
    let (frame, mut heap) = actor.frame().await;
    let authority = ActorExecutorAuthority::new();
    let mut competitor = Box::pin(actor.store.acquire_execution(&authority, &actor.handle));
    assert!(poll_once(competitor.as_mut()).is_pending());

    let (fixture, interpreter, projection, call) = unary_fixture();
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller");
    let mut env = Env::new();
    let context = fixture
        .execution_context(&interpreter, fixture.caller_eval_target())
        .with_actor_execution_frame(frame.clone());
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut heap,
        &mut env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("activation evaluator");
    let mut evaluation = Box::pin(eval.eval_program_call(&call));
    assert!(
        matches!(
            poll_once(evaluation.as_mut()),
            Poll::Ready(Err(RuntimeError::UserException(_)))
        ),
        "a first-poll Ready provider outcome must finalize without releasing the Actor segment"
    );
    drop(evaluation);
    drop(eval);
    assert!(
        poll_once(competitor.as_mut()).is_pending(),
        "the queued Actor execution must remain blocked across a Ready activation call"
    );
    drop(competitor);
    frame.finish(heap).expect("finish Ready Actor segment");
}

#[tokio::test]
async fn f445h_e4r_stream_activation_public_instance_receiver_executes_after_synchronous_setup() {
    let actor = ActorFrameFixture::new();
    let (frame, mut heap) = actor.frame().await;
    let authority = ActorExecutorAuthority::new();
    let mut competitor = Box::pin(actor.store.acquire_execution(&authority, &actor.handle));
    assert!(poll_once(competitor.as_mut()).is_pending());

    let fixture = server_stream_fixture::fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let projection = RuntimeAssemblyExecutionProjection::from_image(Arc::clone(
        fixture.target.execution_image(),
    ));
    let caller = projection
        .resolve_executable(&fixture.caller_addr)
        .expect("linked activation stream caller");
    let call = caller
        .executable
        .body
        .expressions
        .iter()
        .find_map(|expression| match expression {
            LinkedExprIr::Call { call }
                if matches!(
                    call.target,
                    LinkedCallTarget::ActivationRelativeService { .. }
                ) =>
            {
                Some(call.clone())
            }
            _ => None,
        })
        .expect("activation-relative server stream call");
    let mut env = Env::new();
    let context = server_stream_fixture::execution_context(&interpreter, fixture.target)
        .with_actor_execution_frame(frame.clone());
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut heap,
        &mut env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("activation stream evaluator");
    let mut evaluation = Box::pin(eval.eval_program_call(&call));
    let Poll::Ready(Ok(stream)) = poll_once(evaluation.as_mut()) else {
        panic!("serverStream setup must complete on its first poll");
    };
    drop(evaluation);
    let stream = crate::runtime_ops::runtime_to_wire(stream.value(), &*eval.heap)
        .expect("activation stream wire handle");
    drop(eval);
    assert!(
        poll_once(competitor.as_mut()).is_pending(),
        "synchronous serverStream setup must keep the Actor segment"
    );
    drop(competitor);
    frame
        .finish(heap)
        .expect("finish activation stream Actor segment");

    let receiver_item = interpreter
        .stream_runtime
        .next(&stream)
        .await
        .expect("activation stream receiver item");
    assert!(
        matches!(
            &receiver_item,
            StreamPoll::Item(value) if value == &serde_json::json!("receiver-stream-item")
        ),
        "public instance stream must emit its receiver const, got {receiver_item:?}"
    );
    assert!(matches!(
        interpreter
            .stream_runtime
            .next(&stream)
            .await
            .expect("activation stream terminal"),
        StreamPoll::End
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        while crate::assembly_execution::provider_stream_tasks_active_for_test() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider stream task converges after End");
}

#[tokio::test]
async fn f445h_e4r_stream_activation_unary_pending_releases_then_reacquires_before_finalize() {
    let actor = ActorFrameFixture::new();
    let (frame, mut heap) = actor.frame().await;
    let authority = ActorExecutorAuthority::new();
    let mut competitor = Box::pin(actor.store.acquire_execution(&authority, &actor.handle));
    assert!(poll_once(competitor.as_mut()).is_pending());

    let (fixture, interpreter, projection, call) = unary_fixture();
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller");
    let caller_target = fixture.caller_eval_target();
    let mut gate = EvalContext::install_activation_relative_wait_gate_for_test(
        caller_target.request_activation().generation(),
    );
    let mut env = Env::new();
    let context = fixture
        .execution_context(&interpreter, caller_target)
        .with_actor_execution_frame(frame.clone());
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut heap,
        &mut env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("activation evaluator");
    let mut evaluation = Box::pin(eval.eval_program_call(&call));
    assert!(poll_once(evaluation.as_mut()).is_pending());
    assert!(
        gate.has_started(),
        "the explicit gate is inside the owned provider wait"
    );
    let Poll::Ready(Ok(competing_lease)) = poll_once(competitor.as_mut()) else {
        panic!("the first real Pending must release the Actor segment");
    };
    drop(competing_lease);
    drop(competitor);
    gate.release();
    assert!(matches!(
        evaluation.await,
        Err(RuntimeError::UserException(_))
    ));
    drop(eval);

    let after_authority = ActorExecutorAuthority::new();
    let mut after = Box::pin(
        actor
            .store
            .acquire_execution(&after_authority, &actor.handle),
    );
    assert!(
        poll_once(after.as_mut()).is_pending(),
        "provider finalize returns only after the same Actor frame reacquires"
    );
    drop(after);
    frame.finish(heap).expect("finish resumed Actor segment");
}

#[tokio::test]
async fn f445h_e4r_stream_activation_unary_actual_evaluator_imports_provider_failure_once() {
    let (fixture, interpreter, projection, call) = unary_fixture();
    let caller = projection
        .resolve_executable(fixture.caller_addr())
        .expect("linked caller");
    let mut heap = RequestHeap::default();
    let mut env = Env::new();
    let context = fixture.execution_context(&interpreter, fixture.caller_eval_target());
    let mut eval = EvalContext::new(
        &interpreter,
        context,
        &mut heap,
        &mut env,
        &caller.addr,
        caller.file.as_ref(),
        caller.executable,
    )
    .expect("activation evaluator");
    assert!(matches!(
        eval.eval_program_call(&call).await,
        Err(RuntimeError::UserException(_))
    ));
}

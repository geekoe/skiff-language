use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ActivationTemplate, ActorAbiInput, ActorDeclarationIr, ActorFieldEncodingIr, ActorFieldIr,
    ActorImplementationIdentity, ActorMethodIdentity, ActorPublicMethodIr, AssemblyIdentity,
    BoundaryCallableProjection, BoundaryCallbackContract, BoundaryEffectGuarantee,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryOperationDescriptor,
    BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier, BoundaryValueEncoding,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, CallIr, CallTargetIr,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CanonicalPackageLinkPlan, ContractDiagnosticText, ContractTypeRef, DeploymentArtifactIdentity,
    DeploymentDiagnosticText, DeploymentOperationBinding, DeploymentRevision, ExecutableBody,
    ExecutableExport, ExecutableIr, ExecutableKind, ExecutableSignatureIr, ExprIr, ExprRefIr,
    FileIrRef, FileIrUnit, InstructionSourceSite, LiteralIr, NativeTarget, OperationCallableKind,
    OperationTargetRef, PackageActorImplementation, PackageArtifact, PackageArtifactRef,
    PackageBinding, PackageBuildId, PackageCallableId, PackageCallableLinkFact,
    PackageCallableParameter, PackageCallableRef, PackageCallableSignature, PackageCodeSlot,
    PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRefIr, PackageRequirement, PackageRequirementKey, PackageRuntimeRequirements,
    PackageSchemaIndex, PackageSchemaIndexRef, PackageTypeRef, ParamIr, ParamModeIr,
    PublicationResourceRef, RuntimeAssembly, ServiceBindingTemplate, ServiceContract,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentRef, ServiceProtocolIdentity,
    ServiceSymbolRef, SlotLayout, SyntheticInstructionSiteReason, TypeDeclIr, TypeDeclarationIr,
    TypeDescriptorIr, TypeExport, TypeRefIr, ACTOR_RUNTIME_ABI_VERSION_V1,
    PACKAGE_ARTIFACT_SCHEMA_VERSION, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
    SERVICE_CONTRACT_SCHEMA_VERSION, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_runtime_linked_program::{FileAddr, LinkedCallTarget, LinkedExprIr, UnitAddr};
use skiff_runtime_loader::{RuntimeAssemblyContentResolver, RuntimeAssemblyLoader};

use crate::assembly::link_runtime_assembly;

const PROVIDER_ID: &str = "example.actor-provider";
const CONSUMER_ID: &str = "example.actor-consumer";
const ACTOR_SYMBOL: &str = "ThreadActor";
const ACTOR_MODULE: &str = "thread_actor";
const METHOD_IDENTITY: &str = "actor-method:read";
const METHOD_NAME: &str = "read";

fn test_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn actor_type_ref() -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: ACTOR_MODULE.to_string(),
            symbol: ACTOR_SYMBOL.to_string(),
        },
    }
}

fn actor_record() -> TypeDescriptorIr {
    TypeDescriptorIr::Record {
        fields: BTreeMap::from([
            ("id".to_string(), TypeRefIr::builtin("string")),
            ("label".to_string(), TypeRefIr::builtin("string")),
        ]),
    }
}

fn no_effects() -> CallableMayEffects {
    CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: false,
        pending_effect_categories: Vec::new(),
        inout_path_effects: Vec::new(),
    }
}

fn semantic_facts() -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: no_effects(),
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        resolved_call_targets: BTreeMap::new(),
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

fn actor_abi() -> ActorAbiInput {
    ActorAbiInput {
        actor_name: ACTOR_SYMBOL.to_string(),
        actor_id_type: TypeRefIr::builtin("string"),
        key_field: "id".to_string(),
        fields: vec![
            ActorFieldIr {
                name: "id".to_string(),
                ty: TypeRefIr::builtin("string"),
                encoding: ActorFieldEncodingIr::CanonicalValueV1,
            },
            ActorFieldIr {
                name: "label".to_string(),
                ty: TypeRefIr::builtin("string"),
                encoding: ActorFieldEncodingIr::CanonicalValueV1,
            },
        ],
        create: None,
        public_methods: vec![ActorPublicMethodIr {
            method_identity: ActorMethodIdentity::new(METHOD_IDENTITY),
            name: METHOD_NAME.to_string(),
            parameters: Vec::new(),
            return_type: TypeRefIr::builtin("string"),
            may_suspend: false,
        }],
        actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
    }
}

fn provider_file_ref(file: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: ACTOR_MODULE.to_string(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

fn provider_package_with_actor(
    include_actor: bool,
) -> (FileIrUnit, PackageArtifact, Option<ActorAbiInput>) {
    let mut file = FileIrUnit::empty(ACTOR_MODULE, "source:thread_actor");
    file.type_table.push(TypeDeclIr {
        name: ACTOR_SYMBOL.to_string(),
        descriptor: actor_record(),
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file.declarations.types.insert(
        ACTOR_SYMBOL.to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}"),
            source_span: None,
        },
    );
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::ImplMethod,
        symbol: format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "self".to_string(),
            slot: 0,
            ty: actor_type_ref(),
            mode: ParamModeIr::Value,
        }],
        return_type: TypeRefIr::builtin("string"),
        self_type: Some(actor_type_ref()),
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    });
    let abi = include_actor.then(actor_abi);
    let actor_abi_identity = abi
        .as_ref()
        .map(|abi| skiff_artifact_identity::actor_abi_identity(abi).unwrap());
    let actor_implementation_identity = ActorImplementationIdentity::new("actor-impl:thread-actor");
    if let Some(abi) = abi.as_ref() {
        file.actor_declarations.push(ActorDeclarationIr {
            actor_abi_identity: actor_abi_identity.clone().unwrap(),
            actor_implementation_identity: actor_implementation_identity.clone(),
            abi: abi.clone(),
            method_implementations: BTreeMap::from([(
                ActorMethodIdentity::new(METHOD_IDENTITY),
                0,
            )]),
            create_implementation: None,
        });
    }
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    let file_ref = provider_file_ref(&file);
    let read_callable = PackageCallableId::new(format!(
        "pkg-callable:{PROVIDER_ID}:top-level:{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"
    ));
    let package_actor_abi =
        actor_abi_identity
            .clone()
            .zip(abi.clone())
            .map(
                |(actor_abi_identity, abi)| skiff_artifact_model::PackageActorAbi {
                    actor_abi_identity,
                    abi,
                },
            );
    let public_type = |local_type_id: &str| PackageLocalAbiSymbol::Type {
        local_type_id: local_type_id.to_string(),
        descriptor: actor_record(),
        is_alias: false,
        is_interface: false,
        type_params: Vec::new(),
        interface_methods: Vec::new(),
        actor: package_actor_abi.clone(),
    };
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: PROVIDER_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                ACTOR_SYMBOL.to_string(),
                public_type(&format!("type:{PROVIDER_ID}:public:{ACTOR_SYMBOL}")),
            )]),
            implementation_symbols: BTreeMap::from([
                (
                    format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}"),
                    public_type(&format!(
                        "type:{PROVIDER_ID}:top-level:{ACTOR_MODULE}.{ACTOR_SYMBOL}"
                    )),
                ),
                (
                    format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: read_callable.clone(),
                        signature: PackageCallableSignature {
                            type_params: Vec::new(),
                            parameters: vec![PackageCallableParameter {
                                name: "self".to_string(),
                                ty: PackageTypeRef::Local {
                                    local_type: actor_type_ref(),
                                },
                                mode: ParamModeIr::Value,
                            }],
                            return_type: PackageTypeRef::Local {
                                local_type: TypeRefIr::builtin("string"),
                            },
                            may_suspend: false,
                        },
                    },
                ),
            ]),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: PROVIDER_ID.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                PROVIDER_ID,
                &BTreeMap::new(),
            )
            .expect("empty provider schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            types: BTreeMap::from([
                (
                    ACTOR_SYMBOL.to_string(),
                    TypeExport {
                        file: file_ref.clone(),
                        type_index: 0,
                        symbol: ACTOR_SYMBOL.to_string(),
                        is_interface: false,
                        descriptor: Some(actor_record()),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: package_actor_abi.clone(),
                    },
                ),
                (
                    format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}"),
                    TypeExport {
                        file: file_ref.clone(),
                        type_index: 0,
                        symbol: format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}"),
                        is_interface: false,
                        descriptor: Some(actor_record()),
                        type_params: Vec::new(),
                        interface_methods: Vec::new(),
                        actor: package_actor_abi,
                    },
                ),
            ]),
            functions: BTreeMap::new(),
            impl_methods: BTreeMap::from([(
                format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"),
                ExecutableExport {
                    file: file_ref.clone(),
                    executable_index: 0,
                    symbol: format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"),
                    signature: ExecutableSignatureIr {
                        params: vec![ParamIr {
                            name: "self".to_string(),
                            slot: 0,
                            ty: actor_type_ref(),
                            mode: ParamModeIr::Value,
                        }],
                        return_type: TypeRefIr::builtin("string"),
                        self_type: None,
                        may_suspend: false,
                    },
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::from([(
            read_callable.clone(),
            PackageCallableLinkFact {
                callable_id: read_callable.clone(),
                target: OperationTargetRef {
                    file_ref,
                    executable_index: 0,
                    callable_abi_id: read_callable.to_string(),
                    callable_kind: OperationCallableKind::ImplMethod,
                },
            },
        )]),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: abi
            .is_some()
            .then(|| PackageActorImplementation {
                actor: ServiceSymbolRef {
                    module_path: ACTOR_MODULE.to_string(),
                    symbol: ACTOR_SYMBOL.to_string(),
                },
                actor_implementation_identity: actor_implementation_identity.clone(),
                methods: BTreeMap::from([(
                    ActorMethodIdentity::new(METHOD_IDENTITY),
                    read_callable.clone(),
                )]),
                create: None,
            })
            .into_iter()
            .collect(),
        local_interface_conformances: Vec::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([(read_callable.clone(), semantic_facts())]),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
        bytecode: None,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    (file, package, abi)
}

fn provider_package() -> (FileIrUnit, PackageArtifact, ActorAbiInput) {
    let (file, package, abi) = provider_package_with_actor(true);
    (file, package, abi.expect("actor fixture carries actor ABI"))
}

fn consumer_file(read_callable: &PackageCallableId) -> FileIrUnit {
    let mut file = FileIrUnit::empty("consumer_main", "source:consumer_main");
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "consumer_main.entry".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("bool"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        expression_types: Vec::new(),
        statement_spans: Vec::new(),
        source_span: None,
    });
    file.external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: PackageRefIr::Dependency {
                dependency_ref: "subjectImpl".to_string(),
            },
            package_callable_id: read_callable.clone(),
        });
    file.executables[0].body.expressions.push(ExprIr::Literal {
        value: LiteralIr::String {
            value: "thread-1".to_string(),
        },
    });
    file.executables[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::Native {
                target: NativeTarget {
                    namespace: "std.actor".to_string(),
                    symbol: "get".to_string(),
                    binding_key: Some("std.actor.get".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            concrete_receiver: None,
            site: test_site(),
            args: vec![ExprRefIr { expression: 0 }],
            inout_args: Vec::new(),
            type_args: BTreeMap::from([
                ("T0".to_string(), actor_type_ref()),
                ("T1".to_string(), TypeRefIr::builtin("string")),
            ]),
            metadata: BTreeMap::new(),
        },
    });
    file.executables[0].body.expressions.push(ExprIr::Call {
        call: CallIr {
            target: CallTargetIr::PackageCallable {
                package_ref: PackageRefIr::Dependency {
                    dependency_ref: "subjectImpl".to_string(),
                },
                package_callable_id: read_callable.clone(),
            },
            concrete_receiver: Some(actor_type_ref()),
            site: test_site(),
            args: vec![ExprRefIr { expression: 1 }],
            inout_args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        },
    });
    file.executables[0].expression_types = vec![
        TypeRefIr::builtin("string"),
        actor_type_ref(),
        TypeRefIr::builtin("string"),
    ];
    skiff_artifact_identity::assign_file_ir_identity(&mut file).unwrap();
    file
}

fn consumer_package(file: &FileIrUnit, provider: &PackageArtifact) -> PackageArtifact {
    let file_ref = FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    };
    let entry_callable = PackageCallableId::new(format!("pkg-callable:{CONSUMER_ID}:entry"));
    let provider_ref = skiff_artifact_identity::package_artifact_ref(provider).unwrap();
    let mut package = PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: CONSUMER_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref.clone()],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::from([(
                "entry".to_string(),
                PackageLocalAbiSymbol::Callable {
                    callable_id: entry_callable.clone(),
                    signature: PackageCallableSignature {
                        type_params: Vec::new(),
                        parameters: Vec::new(),
                        return_type: PackageTypeRef::Local {
                            local_type: TypeRefIr::builtin("bool"),
                        },
                        may_suspend: false,
                    },
                },
            )]),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: CONSUMER_ID.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                CONSUMER_ID,
                &BTreeMap::new(),
            )
            .expect("empty consumer schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks {
            functions: BTreeMap::from([(
                "entry".to_string(),
                ExecutableExport {
                    file: file_ref.clone(),
                    executable_index: 0,
                    symbol: "consumer_main.entry".to_string(),
                    signature: ExecutableSignatureIr {
                        params: Vec::new(),
                        return_type: TypeRefIr::builtin("bool"),
                        self_type: None,
                        may_suspend: false,
                    },
                },
            )]),
            ..PackageImplementationLinks::default()
        },
        callable_links: BTreeMap::from([(
            entry_callable.clone(),
            PackageCallableLinkFact {
                callable_id: entry_callable.clone(),
                target: OperationTargetRef {
                    file_ref: file_ref.clone(),
                    executable_index: 0,
                    callable_abi_id: entry_callable.to_string(),
                    callable_kind: OperationCallableKind::PublicFunction,
                },
            },
        )]),
        synthetic_callback_owners: Vec::new(),
        bytecode_schema_records: BTreeMap::new(),
        actor_implementations: Vec::new(),
        local_interface_conformances: Vec::new(),
        package_requirements: vec![PackageRequirement {
            alias: "subjectImpl".to_string(),
            package_id: PROVIDER_ID.to_string(),
            exact_version: "1.0.0".to_string(),
            expected_local_abi: provider_ref.package_local_abi_identity.clone(),
            expected_package_build: Some(provider_ref.package_build_id.clone()),
        }],
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements { config: Vec::new() },
        callable_semantic_facts: BTreeMap::from([(entry_callable.clone(), semantic_facts())]),
        boundary_projections: BTreeMap::from([(
            entry_callable.clone(),
            BoundaryCallableProjection::Available {
                operation_contract: operation_contract(),
                implementation_requirements: BoundaryImplementationRequirements {
                    config: Vec::new(),
                    state: Vec::new(),
                    native_capabilities: Vec::new(),
                    complete_may_effects: no_effects(),
                    provenance: CallableProvenanceSummary::Analyzed {
                        return_origins: Vec::new(),
                        direct_return_origins: Vec::new(),
                        throw_origins: Vec::new(),
                        escape_lanes: Vec::new(),
                    },
                },
            },
        )]),
        service_call_refs: Vec::new(),
        bytecode: None,
    };
    skiff_artifact_identity::assign_package_artifact_identities(&mut package).unwrap();
    package
}

struct CrossPackageResolver {
    packages: BTreeMap<PackageArtifactRef, Arc<PackageArtifact>>,
    schema_indexes: Vec<(PackageSchemaIndexRef, Arc<PackageSchemaIndex>)>,
    files: BTreeMap<(PackageBuildId, String), Arc<FileIrUnit>>,
    contracts: BTreeMap<ServiceContractRef, Arc<ServiceContract>>,
    deployments: BTreeMap<ServiceDeploymentRef, Arc<ServiceDeployment>>,
}

impl RuntimeAssemblyContentResolver for CrossPackageResolver {
    fn resolve_deployment(
        &self,
        reference: &skiff_artifact_model::ServiceDeploymentRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::ServiceDeployment>> {
        self.deployments
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing service deployment"))
    }

    fn resolve_contract(
        &self,
        reference: &skiff_artifact_model::ServiceContractRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::ServiceContract>> {
        self.contracts
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing service contract"))
    }

    fn resolve_package_schema_type(
        &self,
        _reference: &skiff_artifact_model::PackageSchemaTypeRecordRef,
    ) -> anyhow::Result<Arc<skiff_artifact_model::PackageSchemaTypeRecord>> {
        anyhow::bail!("cross-package actor fixture has no schema records")
    }

    fn resolve_package_schema_index(
        &self,
        reference: &PackageSchemaIndexRef,
    ) -> anyhow::Result<Arc<PackageSchemaIndex>> {
        self.schema_indexes
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, index)| Arc::clone(index))
            .ok_or_else(|| anyhow::anyhow!("missing package schema index"))
    }

    fn resolve_package(
        &self,
        reference: &PackageArtifactRef,
    ) -> anyhow::Result<Arc<PackageArtifact>> {
        self.packages
            .get(reference)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing package"))
    }

    fn resolve_file_ir(
        &self,
        package: &PackageArtifactRef,
        reference: &FileIrRef,
    ) -> anyhow::Result<Arc<FileIrUnit>> {
        self.files
            .get(&(
                package.package_build_id.clone(),
                reference.file_ir_identity.clone(),
            ))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing File IR"))
    }

    fn resolve_static_resource(
        &self,
        _package: &PackageArtifactRef,
        _reference: &PublicationResourceRef,
    ) -> anyhow::Result<Arc<[u8]>> {
        anyhow::bail!("cross-package actor fixture has no static resources")
    }
}

fn fixture_inputs(
    provider: PackageArtifact,
    provider_file: FileIrUnit,
    consumer: PackageArtifact,
    consumer_file: FileIrUnit,
) -> (RuntimeAssembly, CrossPackageResolver) {
    let provider_ref = skiff_artifact_identity::package_artifact_ref(&provider).unwrap();
    let consumer_ref = skiff_artifact_identity::package_artifact_ref(&consumer).unwrap();
    let entry_callable = PackageCallableId::new(format!("pkg-callable:{CONSUMER_ID}:entry"));
    let service_id = "example.actor-consumer-service";
    let contract_version = "1.0.0";
    let operation_id =
        skiff_artifact_identity::contract_operation_id(service_id, contract_version, "entry")
            .expect("fixture contract operation id");
    let descriptor = BoundaryOperationDescriptor {
        operation_id: operation_id.clone(),
        stable_key: "entry".to_string(),
        contract: operation_contract(),
    };
    let mut contract = ServiceContract {
        schema_version: SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
        service_id: service_id.to_string(),
        contract_version: contract_version.to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        operations: BTreeMap::from([(operation_id.clone(), descriptor)]),
        public_instances: BTreeMap::new(),
        package_type_requirements: Vec::new(),
        diagnostic_text: ContractDiagnosticText {
            service: "Actor consumer".to_string(),
            operations: BTreeMap::from([(operation_id.clone(), "entry".to_string())]),
            types: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_contract_identities(&mut contract).unwrap();
    let contract_ref = ServiceContractRef {
        service_id: contract.service_id.clone(),
        contract_version: contract.contract_version.clone(),
        service_protocol_identity: contract.service_protocol_identity.clone(),
    };
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: contract_ref.clone(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: consumer_ref.clone(),
        operation_bindings: vec![DeploymentOperationBinding {
            contract_operation_id: operation_id,
            package_callable_id: entry_callable,
        }],
        package_bindings: vec![PackageBinding {
            key: PackageRequirementKey {
                caller_package_build_id: consumer.package_build_id.clone(),
                package_requirement_alias: "subjectImpl".to_string(),
            },
            package: provider_ref.clone(),
        }],
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "Actor consumer".to_string(),
            notes: BTreeMap::new(),
        },
    };
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment).unwrap();
    let deployment_ref = skiff_artifact_identity::service_deployment_ref(&deployment);

    let mut assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("unassigned"),
        roots: vec![deployment_ref.clone()],
        resolved_deployments: vec![deployment_ref.clone()],
        resolved_contracts: vec![contract_ref.clone()],
        resolved_packages: vec![consumer_ref.clone(), provider_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: consumer_ref.clone(),
                },
                PackageCodeSlot {
                    package: provider_ref.clone(),
                },
            ],
            package_links: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: consumer.package_build_id.clone(),
                    package_requirement_alias: "subjectImpl".to_string(),
                },
                package: provider_ref.clone(),
            }],
        },
        service_binding_templates: vec![ServiceBindingTemplate {
            activation: deployment_ref.clone(),
            bindings: Vec::new(),
        }],
        activation_templates: vec![ActivationTemplate {
            deployment: deployment_ref,
            implementation_package_build_id: consumer.package_build_id.clone(),
        }],
        gateway_ingress: Vec::new(),
    };
    skiff_artifact_identity::assign_runtime_assembly_identity(&mut assembly).unwrap();

    let resolver = CrossPackageResolver {
        packages: BTreeMap::from([
            (consumer_ref.clone(), Arc::new(consumer.clone())),
            (provider_ref.clone(), Arc::new(provider.clone())),
        ]),
        schema_indexes: vec![
            (
                consumer.package_schema_index.clone(),
                Arc::new(PackageSchemaIndex {
                    package_id: CONSUMER_ID.to_string(),
                    package_schema_index_identity: consumer
                        .package_schema_index
                        .package_schema_index_identity
                        .clone(),
                    types: BTreeMap::new(),
                }),
            ),
            (
                provider.package_schema_index.clone(),
                Arc::new(PackageSchemaIndex {
                    package_id: PROVIDER_ID.to_string(),
                    package_schema_index_identity: provider
                        .package_schema_index
                        .package_schema_index_identity
                        .clone(),
                    types: BTreeMap::new(),
                }),
            ),
        ],
        files: BTreeMap::from([
            (
                (
                    consumer.package_build_id.clone(),
                    consumer_file.file_ir_identity.clone(),
                ),
                Arc::new(consumer_file),
            ),
            (
                (
                    provider.package_build_id.clone(),
                    provider_file.file_ir_identity.clone(),
                ),
                Arc::new(provider_file),
            ),
        ]),
        contracts: BTreeMap::from([(contract_ref, Arc::new(contract))]),
        deployments: BTreeMap::from([(
            skiff_artifact_identity::service_deployment_ref(&deployment),
            Arc::new(deployment),
        )]),
    };
    (assembly, resolver)
}

fn link_cross_package_actor_fixture() -> (crate::assembly::AssemblyLinkedCandidate, PackageArtifact)
{
    let (provider_file, provider, _) = provider_package();
    let read_callable = PackageCallableId::new(format!(
        "pkg-callable:{PROVIDER_ID}:top-level:{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"
    ));
    let consumer_file = consumer_file(&read_callable);
    let consumer = consumer_package(&consumer_file, &provider);
    let (assembly, resolver) =
        fixture_inputs(provider.clone(), provider_file, consumer, consumer_file);
    let hydrated = RuntimeAssemblyLoader::new(&resolver)
        .load(assembly)
        .expect("cross-package actor fixture should hydrate");
    let candidate = link_runtime_assembly(hydrated)
        .expect("cross-package actor fixture should link through the assembly execution linker");
    (candidate, provider)
}

#[test]
fn cross_package_actor_registry_get_and_method_call_link_through_provider_artifact() {
    let (candidate, provider) = link_cross_package_actor_fixture();
    let image = candidate.execution_image();
    let consumer = &image.execution_packages()[0];
    let file = &consumer.files()[0];
    let expressions = &file.executables[0].body.expressions;

    let registry = expressions
        .iter()
        .find_map(|expression| {
            let LinkedExprIr::Call { call } = expression else {
                return None;
            };
            matches!(
                &call.target,
                LinkedCallTarget::Native { target }
                    if target.binding_key.as_deref() == Some("std.actor.get")
            )
            .then_some(call)
        })
        .expect("consumer std.actor.get call should link");
    let metadata = registry
        .actor_metadata
        .as_ref()
        .expect("cross-package std.actor.get must pin the provider declaration owner");
    assert_eq!(metadata.declaration_owner.unit, UnitAddr::Package(1));
    assert_eq!(
        metadata.declaration_owner.file,
        FileAddr::LoadedFileIndex(0)
    );
    assert_eq!(metadata.declaration_owner.actor_symbol, ACTOR_SYMBOL);

    let dispatch = expressions
        .iter()
        .find_map(|expression| {
            let LinkedExprIr::Call { call } = expression else {
                return None;
            };
            match &call.target {
                LinkedCallTarget::ActorDispatch { plan } => Some(plan.clone()),
                _ => None,
            }
        })
        .expect("cross-package actor method call must link as routed ActorDispatch");
    assert_eq!(dispatch.declaration_owner.unit, UnitAddr::Package(1));
    assert_eq!(
        dispatch.declaration_owner.file,
        FileAddr::LoadedFileIndex(0)
    );
    assert_eq!(dispatch.declaration_owner.actor_symbol, ACTOR_SYMBOL);
    assert_eq!(dispatch.method_identity.as_str(), METHOD_IDENTITY);

    let provider_declaration = image.execution_packages()[1]
        .files()
        .iter()
        .flat_map(|file| &file.actor_declarations)
        .find(|declaration| declaration.actor_name == ACTOR_SYMBOL)
        .expect("provider linked actor declaration");
    assert_eq!(
        provider_declaration.implementation_owner.as_ref(),
        Some(&dispatch.declaration_owner)
    );
    assert_eq!(
        dispatch.actor_abi_identity,
        provider_declaration.actor_abi_identity
    );
    assert_eq!(
        dispatch.actor_implementation_identity,
        provider_declaration.actor_implementation_identity
    );

    let provider_export = provider
        .implementation_links
        .types
        .get(&format!("{ACTOR_MODULE}.{ACTOR_SYMBOL}"))
        .expect("provider artifact actor export");
    assert_eq!(
        provider_export
            .actor
            .as_ref()
            .map(|actor| &actor.actor_abi_identity),
        Some(&provider_declaration.actor_abi_identity),
        "runtime linker must consume the PackageArtifact actor metadata as its fact source"
    );
}

#[test]
fn cross_package_actor_reference_fails_closed_without_provider_actor_declaration() {
    let (provider_file, provider, _) = provider_package_with_actor(false);
    let read_callable = PackageCallableId::new(format!(
        "pkg-callable:{PROVIDER_ID}:top-level:{ACTOR_MODULE}.{ACTOR_SYMBOL}.{METHOD_NAME}"
    ));
    let mut consumer_file = consumer_file(&read_callable);
    consumer_file.executables[0].return_type = actor_type_ref();
    skiff_artifact_identity::assign_file_ir_identity(&mut consumer_file).unwrap();
    let consumer = consumer_package(&consumer_file, &provider);
    let (assembly, resolver) = fixture_inputs(provider, provider_file, consumer, consumer_file);
    let error = RuntimeAssemblyLoader::new(&resolver)
        .load(assembly)
        .and_then(link_runtime_assembly)
        .expect_err("provider without an actor declaration must fail link");
    assert!(
        format!("{error:#}").contains("type symbol thread_actor.ThreadActor is unresolved"),
        "unexpected error: {error:#}"
    );
}

#[path = "test_runtime.rs"]
pub(crate) mod test_runtime;

use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::*;
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
    RequestActivationContext, RuntimeActivation,
};
use skiff_runtime_linked_program::{
    AssemblyExecutionImage, AssemblyPackageExecutionCode, HydratedPackageCode, LinkedCallTarget,
    LinkedExecutableBody, LinkedExprIr, LinkedStmtIr, PublicationResourceTable, RuntimeTypeContext,
    SharedPackageLinkedImage,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapHandle, HeapNode, RuntimeValue},
};

use crate::{
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

const DEPENDENCY_ALIAS: &str = "mutableDependency";

/// Executes a real linked caller and package callee through the canonical dispatcher. The
/// callee's index assignment can only be observed here when package-direct keeps the exact
/// request heap and handle; a manual handle clone would not exercise this path.
#[tokio::test]
async fn package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation() {
    let fixture = package_direct_fixture();
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let caller_handle = heap
        .alloc_array(vec![RuntimeValue::String("caller".to_string())])
        .expect("caller mutable array should allocate");

    let result = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut heap,
            &fixture.caller_addr,
            vec![RuntimeValue::Heap(caller_handle)],
        )
        .await
        .expect("canonical caller and package-direct callee should execute");

    let RuntimeValue::Heap(returned_handle) = result else {
        panic!("package callee should return the caller heap handle")
    };
    assert_eq!(
        returned_handle, caller_handle,
        "package-direct must preserve handle identity"
    );
    assert_array_item(&heap, caller_handle, "package-callee");
}

#[tokio::test]
async fn object_materialization_interpreter_heap_shape_distinguishes_construct_and_map_literal() {
    let (object, object_heap) = execute_materialization_expression(ExprIr::Construct {
        type_ref: TypeRefIr::Record {
            fields: BTreeMap::from([("value".to_string(), TypeRefIr::native("string"))]),
        },
        fields: BTreeMap::from([("value".to_string(), ExprRefIr { expression: 0 })]),
    })
    .await;
    let RuntimeValue::Heap(object_handle) = object else {
        panic!("Construct should return a heap value")
    };
    assert!(matches!(
        object_heap
            .get(object_handle)
            .expect("Construct heap handle should resolve"),
        HeapNode::Object(_)
    ));

    let (map, map_heap) = execute_materialization_expression(ExprIr::MapLiteral {
        entries: BTreeMap::from([("value".to_string(), ExprRefIr { expression: 0 })]),
    })
    .await;
    let RuntimeValue::Heap(map_handle) = map else {
        panic!("MapLiteral should return a heap value")
    };
    assert!(matches!(
        map_heap
            .get(map_handle)
            .expect("MapLiteral heap handle should resolve"),
        HeapNode::Map(_)
    ));
}

#[test]
fn ordinary_in_process_keeps_lane_specific_type_arguments_out_of_shared_planner() {
    let descriptor = BoundaryOperationDescriptor {
        operation_id: ContractOperationId::new("operation:ordinary-lane-validation"),
        stable_key: "ordinaryLaneValidation".to_string(),
        contract: ordinary_array_contract(),
    };
    let mut call = skiff_runtime_linked_program::CallIr {
        target: LinkedCallTarget::Builtin {
            op: "ordinary-lane-validation".to_string(),
        },
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    };
    call.type_args.insert(
        "T".to_string(),
        skiff_runtime_linked_program::LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        },
    );

    let error = super::validate_ordinary_operation(&descriptor, &call)
        .expect_err("ordinary lane must reject package-local type arguments");
    assert!(matches!(
        error,
        crate::error::RuntimeError::InvalidArtifact(_)
    ));
}

struct PackageDirectFixture {
    eval_target: RuntimeAssemblyEvalTarget,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
}

async fn execute_materialization_expression(expression: ExprIr) -> (RuntimeValue, RequestHeap) {
    let fixture = materialization_fixture(expression);
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.eval_target);
    let mut heap = RequestHeap::default();
    let value = interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("materialization expression should execute");
    (value, heap)
}

fn materialization_fixture(expression: ExprIr) -> PackageDirectFixture {
    let mut file = FileIrUnit::empty("object_materialization.heap_shape", "source:heap-shape");
    file.executables.push(ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "heapShape".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::native("Json"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "materialized".to_string(),
                    },
                },
                expression,
            ],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("materialization File IR should receive a canonical identity");
    let mut package = private_package("example.object-materialization", &file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("materialization package should receive canonical identities");
    let package_ref = package_ref(&package);
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:object-materialization-heap-shape"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![package_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![PackageCodeSlot {
                package: package_ref.clone(),
            }],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    let shared = Arc::new(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            [HydratedPackageCode::new(
                Arc::new(package),
                vec![Arc::new(file.clone())],
                PublicationResourceTable::default(),
            )],
        )
        .expect("materialization package should hydrate"),
    );
    let linked_file = skiff_runtime_linker::linked_file_unit_from_artifact(&file)
        .expect("materialization File IR should link");
    let code = Arc::new(
        AssemblyPackageExecutionCode::try_new(&shared.code_slots()[0], vec![Arc::new(linked_file)])
            .expect("materialization execution slot should match the canonical source"),
    );
    let image = Arc::new(
        AssemblyExecutionImage::try_new(shared, vec![code], RuntimeTypeContext::default())
            .expect("materialization execution image should build"),
    );
    let caller_addr = skiff_runtime_linked_program::ExecutableAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        executable: 0,
    };
    let activation = activation_context(assembly.assembly_identity, package_ref.package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("materialization request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("materialization image and activation should form an eval target");
    PackageDirectFixture {
        eval_target,
        caller_addr,
    }
}

fn package_direct_fixture() -> PackageDirectFixture {
    let array_type = array_type();
    let callable_id = PackageCallableId::new("callable:package-direct-mutate");

    let mut callee_file = FileIrUnit::empty("package_direct.callee", "source:callee");
    callee_file
        .executables
        .push(callee_executable(array_type.clone()));
    skiff_artifact_identity::assign_file_ir_identity(&mut callee_file)
        .expect("callee File IR should receive a canonical identity");
    let mut callee_package = callable_package(
        "example.package-direct-callee",
        &callee_file,
        callable_id.clone(),
        array_type.clone(),
    );
    skiff_artifact_identity::assign_package_artifact_identities(&mut callee_package)
        .expect("callee package should receive canonical identities");
    let callee_ref = package_ref(&callee_package);

    let dependency_ref = PackageRefIr::Dependency {
        dependency_ref: DEPENDENCY_ALIAS.to_string(),
    };
    let mut caller_file = FileIrUnit::empty("package_direct.caller", "source:caller");
    caller_file
        .external_refs
        .package_callables
        .push(PackageCallableRef {
            package_ref: dependency_ref.clone(),
            package_callable_id: callable_id.clone(),
        });
    caller_file.executables.push(caller_executable(
        array_type,
        dependency_ref.clone(),
        callable_id.clone(),
    ));
    skiff_artifact_identity::assign_file_ir_identity(&mut caller_file)
        .expect("caller File IR should receive a canonical identity");
    let mut caller_package = private_package("example.package-direct-caller", &caller_file);
    caller_package
        .package_requirements
        .push(PackageRequirement {
            alias: DEPENDENCY_ALIAS.to_string(),
            package_id: callee_ref.package_id.clone(),
            exact_version: callee_ref.package_version.clone(),
            expected_local_abi: callee_ref.package_local_abi_identity.clone(),
        });
    skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
        .expect("caller package should receive canonical identities");
    let caller_ref = package_ref(&caller_package);

    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new("assembly:package-direct-same-heap"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![caller_ref.clone(), callee_ref.clone()],
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: vec![
                PackageCodeSlot {
                    package: caller_ref.clone(),
                },
                PackageCodeSlot {
                    package: callee_ref.clone(),
                },
            ],
            package_links: vec![PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller_ref.package_build_id.clone(),
                    package_requirement_alias: DEPENDENCY_ALIAS.to_string(),
                },
                package: callee_ref,
            }],
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        global_ingress: Vec::new(),
    };
    let shared = Arc::new(
        SharedPackageLinkedImage::from_runtime_assembly(
            &assembly,
            [
                HydratedPackageCode::new(
                    Arc::new(caller_package),
                    vec![Arc::new(caller_file.clone())],
                    PublicationResourceTable::default(),
                ),
                HydratedPackageCode::new(
                    Arc::new(callee_package),
                    vec![Arc::new(callee_file.clone())],
                    PublicationResourceTable::default(),
                ),
            ],
        )
        .expect("canonical package graph should hydrate"),
    );
    let direct_call = shared
        .resolve_package_direct_call(&caller_ref.package_build_id, &dependency_ref, &callable_id)
        .expect("canonical package dependency should resolve to an exact executable");

    let mut caller_conversion = caller_file.clone();
    caller_conversion.external_refs.package_callables.clear();
    caller_conversion.executables[0].body = ExecutableBody::default();
    let mut linked_caller =
        skiff_runtime_linker::linked_file_unit_from_artifact(&caller_conversion)
            .expect("caller shell should convert before installing the resolved canonical call");
    linked_caller.executables[0].body = linked_caller_body(direct_call);
    let linked_callee = skiff_runtime_linker::linked_file_unit_from_artifact(&callee_file)
        .expect("callee should link as ordinary package code");

    let caller_code = Arc::new(
        AssemblyPackageExecutionCode::try_new(
            &shared.code_slots()[0],
            vec![Arc::new(linked_caller)],
        )
        .expect("caller execution slot should match the canonical source"),
    );
    let callee_code = Arc::new(
        AssemblyPackageExecutionCode::try_new(
            &shared.code_slots()[1],
            vec![Arc::new(linked_callee)],
        )
        .expect("callee execution slot should match the canonical source"),
    );
    let image = Arc::new(
        AssemblyExecutionImage::try_new(
            shared,
            vec![caller_code, callee_code],
            RuntimeTypeContext::default(),
        )
        .expect("canonical execution image should own both package slots"),
    );
    let caller_addr = skiff_runtime_linked_program::ExecutableAddr {
        unit: skiff_runtime_linked_program::UnitAddr::Package(0),
        file: skiff_runtime_linked_program::FileAddr::LoadedFileIndex(0),
        executable: 0,
    };

    let activation = activation_context(assembly.assembly_identity, caller_ref.package_build_id);
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let request = RequestActivationContext::begin(activation)
        .expect("package-direct request generation should begin");
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("canonical image and activation should form an eval target");
    PackageDirectFixture {
        eval_target,
        caller_addr,
    }
}

fn caller_executable(
    array_type: TypeRefIr,
    package_ref: PackageRefIr,
    package_callable_id: PackageCallableId,
) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "callPackageMutator".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type,
        self_type: None,
        slots: parameter_slots(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            }],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Call {
                    call: skiff_artifact_model::CallIr {
                        target: CallTargetIr::PackageCallable {
                            package_ref,
                            package_callable_id,
                        },
                        args: vec![ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        },
        source_span: None,
    }
}

fn callee_executable(array_type: TypeRefIr) -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "mutate".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "value".to_string(),
            slot: 0,
            ty: array_type.clone(),
        }],
        return_type: array_type,
        self_type: None,
        slots: parameter_slots(),
        may_suspend: false,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Assign {
                    target: AssignTargetIr::Index {
                        object: ExprRefIr { expression: 0 },
                        index: ExprRefIr { expression: 1 },
                    },
                    value: ExprRefIr { expression: 2 },
                },
                StmtIr::Return {
                    value: Some(ExprRefIr { expression: 3 }),
                },
            ],
            expressions: vec![
                ExprIr::LoadSlot { slot: 0 },
                ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(0),
                    },
                },
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "package-callee".to_string(),
                    },
                },
                ExprIr::LoadSlot { slot: 0 },
            ],
        },
        source_span: None,
    }
}

fn linked_caller_body(
    direct_call: skiff_runtime_linked_program::LinkedPackageDirectCall,
) -> LinkedExecutableBody {
    LinkedExecutableBody {
        blocks: vec![skiff_runtime_linked_program::BlockIr {
            label: "entry".to_string(),
            statements: vec![skiff_runtime_linked_program::StmtRefIr { statement: 0 }],
        }],
        statements: vec![LinkedStmtIr::Return {
            value: Some(skiff_runtime_linked_program::ExprRefIr { expression: 1 }),
        }],
        expressions: vec![
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Call {
                call: skiff_runtime_linked_program::CallIr {
                    target: LinkedCallTarget::PackageDirect { call: direct_call },
                    args: vec![skiff_runtime_linked_program::ExprRefIr { expression: 0 }],
                    type_args: BTreeMap::new(),
                    metadata: BTreeMap::new(),
                },
            },
        ],
    }
}

fn parameter_slots() -> SlotLayout {
    SlotLayout {
        slots: vec![SlotIr {
            index: 0,
            name: "value".to_string(),
            kind: SlotKind::Param,
        }],
        frame_size: 1,
    }
}

fn callable_package(
    package_id: &str,
    file: &FileIrUnit,
    callable_id: PackageCallableId,
    array_type: TypeRefIr,
) -> PackageArtifact {
    let file_ref = file_ref(file);
    let contract = ordinary_array_contract();
    let effects = no_effects();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: Vec::new(),
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    };
    let mut package = private_package(package_id, file);
    package.package_local_abi.public_symbols.insert(
        "mutate".to_string(),
        PackageLocalAbiSymbol::Callable {
            callable_id: callable_id.clone(),
            signature: PackageCallableSignature {
                parameters: vec![PackageCallableParameter {
                    name: "value".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: array_type.clone(),
                    },
                }],
                return_type: PackageTypeRef::Local {
                    local_type: array_type,
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
            effects: CallableEffectSummary::Analyzed {
                effects: effects.clone(),
            },
            provenance: provenance.clone(),
            resolved_call_targets: BTreeMap::new(),
        },
    );
    package.boundary_projections.insert(
        callable_id,
        BoundaryCallableProjection::Available {
            operation_contract: contract,
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
    package
}

fn private_package(package_id: &str, file: &FileIrUnit) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: package_id.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![file_ref(file)],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
        },
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        callable_semantic_facts: BTreeMap::new(),
        boundary_projections: BTreeMap::new(),
        service_call_refs: Vec::new(),
    }
}

fn ordinary_array_contract() -> BoundaryOperationContract {
    let array = ContractTypeRef::Builtin {
        name: "Array".to_string(),
        arguments: vec![ContractTypeRef::builtin("string")],
    };
    BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "value".to_string(),
            ty: array.clone(),
            value_plan: detached_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: array,
            value_plan: detached_plan(BoundaryValueOwner::Provider),
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

fn detached_plan(owner: BoundaryValueOwner) -> BoundaryValuePlan {
    BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
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

fn array_type() -> TypeRefIr {
    TypeRefIr::Native {
        name: "Array".to_string(),
        args: vec![TypeRefIr::native("string")],
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

fn package_ref(package: &PackageArtifact) -> PackageArtifactRef {
    PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn activation_context(
    assembly_identity: AssemblyIdentity,
    package_build_id: PackageBuildId,
) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 1,
            runtime_replica_id: "replica:package-direct-test".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: "example.package-direct-caller".to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("package-direct-test-r1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    "deployment:package-direct-test",
                ),
            },
        },
        package_build_id,
        ActivationOwnedBindings {
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            policy: DeploymentPolicy {
                timeout_ms: 1_000,
                resources: ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_048_576,
                },
                activation: ActivationPolicy {
                    max_concurrency: 1,
                    idle_timeout_ms: None,
                },
                principal: "test".to_string(),
            },
        },
        Vec::new(),
    )
    .expect("test activation should build")
}

struct TestResolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for TestResolver {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == activation_id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == activation_id)
            .then(|| Arc::clone(&self.activation))
    }

    fn contract(&self, _contract: &ServiceContractRef) -> Option<Arc<ServiceContract>> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &ContractOperationId,
    ) -> Option<OperationTargetRef> {
        None
    }
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    target: RuntimeAssemblyEvalTarget,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    let runtime_activation = Arc::new(RuntimeActivation {
        service: skiff_runtime_linked_program::ServiceMeta {
            id: "example.package-direct-caller".to_string(),
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
    });
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
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
        runtime_activation,
        actor: actor.clone(),
        spawn: actor,
        outbound: test_runtime::outbound_context(),
        request_heap_limits: RequestHeapLimits::default(),
    })
    .with_runtime_assembly_target(target)
}

fn assert_array_item(heap: &RequestHeap, handle: HeapHandle, expected: &str) {
    let HeapNode::Array(items) = heap.get(handle).expect("array handle should resolve") else {
        panic!("heap value should remain an array")
    };
    assert_eq!(items, &[RuntimeValue::String(expected.to_string())]);
}

use super::*;

use skiff_artifact_model as artifact;
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings, CallbackLifetime,
    RequestActivationContext,
};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_model::runtime_value::{
    InterfaceCarrier, InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable,
    InterfaceMethodTarget, InterfaceMethodType, InterfaceReceiverCallAbi, InterfaceValue,
};
use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

use crate::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

const CALLBACK_PACKAGE_ID: &str = "example.f445h.callback-owner";
const CALLBACK_INTERFACE_ABI: &str = "interface:f445h-e4r:callback";
const CALLBACK_METHOD_ABI: &str = "method:f445h-e4r:callback";
const CALLBACK_SCHEMA_ID: &str = "schema:f445h-e4r:callback";
const CALLBACK_STABLE_KEY: &str = "api.Callback";

struct CallbackFixture {
    evaluator: EvaluatorFixture,
    target: RuntimeAssemblyEvalTarget,
    carrier: skiff_runtime_model::runtime_value::CallbackCapabilityCarrier,
    caller_addr: ExecutableAddr,
}

fn callback_caller() -> EvaluatorFixture {
    let interface = LinkedInterfaceInstantiationRef {
        interface_abi_id: CALLBACK_INTERFACE_ABI.to_string(),
        canonical_type_args: Vec::new(),
    };
    EvaluatorFixture::new(
        vec![
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::InterfaceMethod {
                        interface,
                        method_abi_id: CALLBACK_METHOD_ABI.to_string(),
                        slot: 0,
                    },
                    vec![0],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 1 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "receiver".to_string(),
                kind: "parameter".to_string(),
            }],
            frame_size: 1,
        },
    )
}

fn callback_owner_file(delay_ms: u64) -> artifact::FileIrUnit {
    let mut file = artifact::FileIrUnit::empty("callback.owner", "source:f445h-e4r-callback-owner");
    file.executables.push(artifact::ExecutableIr {
        kind: artifact::ExecutableKind::Function,
        symbol: "callerAddressAnchor".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: artifact::TypeRefIr::builtin("void"),
        self_type: None,
        slots: artifact::SlotLayout::default(),
        may_suspend: false,
        body: artifact::ExecutableBody {
            blocks: vec![artifact::BlockIr {
                label: "entry".to_string(),
                statements: vec![artifact::StmtRefIr { statement: 0 }],
            }],
            statements: vec![artifact::StmtIr::Return { value: None }],
            expressions: Vec::new(),
        },
        source_span: None,
    });
    file.executables.push(artifact::ExecutableIr {
        kind: artifact::ExecutableKind::ImplMethod,
        symbol: "invoke".to_string(),
        type_params: Vec::new(),
        params: vec![artifact::ParamIr {
            name: "self".to_string(),
            slot: 0,
            ty: artifact::TypeRefIr::builtin("string"),
        }],
        return_type: artifact::TypeRefIr::builtin("string"),
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
                artifact::StmtIr::Expr {
                    value: artifact::ExprRefIr { expression: 1 },
                },
                artifact::StmtIr::Return {
                    value: Some(artifact::ExprRefIr { expression: 2 }),
                },
            ],
            expressions: vec![
                artifact::ExprIr::Literal {
                    value: artifact::LiteralIr::Number {
                        value: serde_json::Number::from(delay_ms),
                    },
                },
                artifact::ExprIr::Call {
                    call: artifact::CallIr {
                        target: artifact::CallTargetIr::Native {
                            target: artifact::NativeTarget {
                                namespace: "std.time".to_string(),
                                symbol: "sleep".to_string(),
                                binding_key: Some("std.time.sleep".to_string()),
                                metadata: BTreeMap::new(),
                            },
                        },
                        site: site(),
                        args: vec![artifact::ExprRefIr { expression: 0 }],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
                artifact::ExprIr::Literal {
                    value: artifact::LiteralIr::String {
                        value: "callback-complete".to_string(),
                    },
                },
            ],
        },
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("callback owner file identity");
    file
}

pub(super) fn file_ref(file: &artifact::FileIrUnit) -> artifact::FileIrRef {
    artifact::FileIrRef {
        file_ir_identity: file.file_ir_identity.clone(),
        module_path: file.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(file.source_ast_hash.clone()),
    }
}

pub(super) fn private_package(
    package_id: &str,
    file: &artifact::FileIrUnit,
) -> artifact::PackageArtifact {
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
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                package_id,
                &BTreeMap::new(),
            )
            .expect("empty callback Package schema index"),
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

fn std_duration_package() -> (artifact::PackageArtifact, artifact::FileIrUnit) {
    let descriptor = artifact::TypeDescriptorIr::Representation {
        representation: artifact::TypeRefIr::builtin("integer"),
    };
    let mut file = artifact::FileIrUnit::empty("std.time", "source:f445h-e4r-std-duration");
    file.declarations.types.insert(
        "Duration".to_string(),
        artifact::TypeDeclarationIr {
            type_index: 0,
            symbol: "std.time.Duration".to_string(),
            source_span: None,
        },
    );
    file.type_table.push(artifact::TypeDeclIr {
        name: "Duration".to_string(),
        descriptor: descriptor.clone(),
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("std Duration file identity");

    let mut package = private_package("skiff.run/std", &file);
    package.package_local_abi.public_symbols.insert(
        "std.time.Duration".to_string(),
        artifact::PackageLocalAbiSymbol::Type {
            local_type_id: "type:skiff.run/std:top-level:std.time.Duration".to_string(),
            descriptor: descriptor.clone(),
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    );
    package.implementation_links.types.insert(
        "std.time.Duration".to_string(),
        artifact::TypeExport {
            file: file_ref(&file),
            type_index: 0,
            symbol: "std.time.Duration".to_string(),
            is_interface: false,
            descriptor: Some(descriptor),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        },
    );
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("std Duration package identities");
    (package, file)
}

pub(super) fn package_ref(package: &artifact::PackageArtifact) -> artifact::PackageArtifactRef {
    artifact::PackageArtifactRef {
        package_id: package.package_id.clone(),
        package_version: package.package_version.clone(),
        package_build_id: package.package_build_id.clone(),
        package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
    }
}

fn activation(
    assembly_identity: artifact::AssemblyIdentity,
    package_build_id: artifact::PackageBuildId,
) -> Arc<ActivationContext> {
    ActivationContext::new(
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 1,
            runtime_replica_id: "replica:f445h-e4r-callback".to_string(),
            deployment: artifact::ServiceDeploymentRef {
                service_id: CALLBACK_PACKAGE_ID.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: artifact::DeploymentRevision::new("f445h-e4r-callback-r1"),
                deployment_artifact_identity: artifact::DeploymentArtifactIdentity::new(
                    "deployment:f445h-e4r-callback",
                ),
            },
        },
        package_build_id,
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
                    idle_timeout_ms: None,
                },
                principal: "test".to_string(),
            },
        },
        Vec::new(),
    )
    .expect("callback activation")
}

struct Resolver {
    activation: Arc<ActivationContext>,
}

impl RuntimeAssemblyEvalResolver for Resolver {
    fn activation(&self, id: &ActivationId) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id() == id).then(|| Arc::clone(&self.activation))
    }

    fn activation_by_opaque_id(&self, id: &str) -> Option<Arc<ActivationContext>> {
        (self.activation.activation_id().as_str() == id).then(|| Arc::clone(&self.activation))
    }

    fn contract(
        &self,
        _contract: &artifact::ServiceContractRef,
    ) -> Option<Arc<artifact::ServiceContract>> {
        None
    }

    fn admitted_schema_records(
        &self,
        _contract: &artifact::ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &artifact::ContractOperationId,
    ) -> Option<artifact::OperationTargetRef> {
        None
    }
}

fn callback_schema() -> (
    artifact::PackageSchemaTypeRef,
    BTreeMap<String, artifact::BoundaryCallbackOperation>,
    PackageSchemaRecords,
) {
    let reference = artifact::PackageSchemaTypeRef {
        package_id: CALLBACK_PACKAGE_ID.to_string(),
        stable_schema_key: CALLBACK_STABLE_KEY.to_string(),
        package_schema_type_id: artifact::PackageSchemaTypeId::new(CALLBACK_SCHEMA_ID),
    };
    let operations = BTreeMap::from([(
        "invoke".to_string(),
        artifact::BoundaryCallbackOperation {
            parameters: Vec::new(),
            return_type: artifact::ContractTypeRef::builtin("string"),
        },
    )]);
    let records = BTreeMap::from([(
        reference.package_schema_type_id.clone(),
        Arc::new(artifact::PackageSchemaTypeRecord {
            package_id: reference.package_id.clone(),
            stable_schema_key: reference.stable_schema_key.clone(),
            package_schema_type_id: reference.package_schema_type_id.clone(),
            canonical_descriptor: artifact::PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: artifact::ContractTypeDescriptor::CallbackInterface {
                    operations: operations.clone(),
                },
            },
        }),
    )]);
    (reference, operations, records)
}

fn fixture(delay_ms: u64) -> CallbackFixture {
    let owner_file = callback_owner_file(delay_ms);
    let mut owner_package = private_package(CALLBACK_PACKAGE_ID, &owner_file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut owner_package)
        .expect("callback owner package identities");
    let owner_ref = package_ref(&owner_package);
    let (std_package, std_file) = std_duration_package();
    let std_ref = package_ref(&std_package);
    let assembly = artifact::RuntimeAssembly {
        schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: artifact::AssemblyIdentity::new("assembly:f445h-e4r-callback"),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: vec![owner_ref.clone(), std_ref.clone()],
        package_link_plan: artifact::CanonicalPackageLinkPlan {
            code_slots: vec![
                artifact::PackageCodeSlot {
                    package: owner_ref.clone(),
                },
                artifact::PackageCodeSlot { package: std_ref },
            ],
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let image = crate::test_support::link_package_fixture(
        assembly.clone(),
        vec![
            (owner_package, vec![owner_file]),
            (std_package, vec![std_file]),
        ],
    );
    let activation = activation(
        assembly.assembly_identity,
        owner_ref.package_build_id.clone(),
    );
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(Resolver {
        activation: Arc::clone(&activation),
    });
    let request =
        RequestActivationContext::begin(Arc::clone(&activation)).expect("callback request");
    let target =
        RuntimeAssemblyEvalTarget::new(image, request, resolver).expect("callback eval target");

    let (schema_type, operations, schema) = callback_schema();
    let callback_addr = ExecutableAddr::package(0, 0, 1);
    let local_interface = InterfaceValue::new(
        CALLBACK_INTERFACE_ABI.to_string(),
        InterfaceCarrier::Local {
            concrete_type: "callback.owner.State".to_string(),
            method_table: InterfaceMethodTable::new(
                "table:f445h-e4r-callback".to_string(),
                CALLBACK_INTERFACE_ABI.to_string(),
                vec![InterfaceMethodSlot::from_admitted_metadata(
                    0,
                    "invoke".to_string(),
                    CALLBACK_METHOD_ABI.to_string(),
                    InterfaceMethodSignature::new(
                        vec![InterfaceMethodType::builtin("Self")],
                        InterfaceMethodType::builtin("string"),
                    ),
                    InterfaceMethodTarget::LocalExecutable {
                        executable: callback_addr,
                        receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                    },
                )],
            ),
            payload: RuntimeValue::String("callback-owner".to_string()),
        },
    );
    let adapter = InProcessCallbackAdapter::from_local_interface(
        schema_type.clone(),
        &local_interface,
        &operations,
        &schema,
        &RequestHeap::default(),
    )
    .expect("callback adapter");
    let contract = serde_json::to_string(&schema_type).expect("callback contract identity");
    let carrier = activation
        .callback_capabilities()
        .register(
            &activation,
            target.request_activation(),
            contract,
            "callback:f445h-e4r",
            CallbackLifetime::Request,
            Arc::new(adapter),
        )
        .expect("register callback");

    CallbackFixture {
        evaluator: callback_caller(),
        target,
        carrier,
        caller_addr: ExecutableAddr::package(0, 0, 0),
    }
}

fn caller_env(fixture: &CallbackFixture, heap: &mut RequestHeap) -> Env {
    let interface = heap
        .alloc_interface(InterfaceValue::new(
            CALLBACK_INTERFACE_ABI.to_string(),
            InterfaceCarrier::CallbackCapability(fixture.carrier.clone()),
        ))
        .expect("callback receiver");
    let mut env = Env::for_program_executable(
        fixture.evaluator.executable(),
        Some(fixture.evaluator.file.module_path.clone()),
        1,
    )
    .expect("callback caller env");
    env.declare_binding("receiver", Some(0), RuntimeValue::Heap(interface))
        .expect("callback receiver binding");
    env
}

#[tokio::test]
async fn f445h_e4r_spine_callback_ready_keeps_actor_segment() {
    let fixture = fixture(0);
    let (frame, mut heap) = fixture.evaluator.actor_frame().await;
    let mut env = caller_env(&fixture, &mut heap);
    let context = default_program_context(&fixture.evaluator.interpreter)
        .with_runtime_assembly_target(fixture.target.clone());
    let mut eval = fixture.evaluator.eval_context_with(
        context,
        frame.clone(),
        &mut heap,
        &mut env,
        &fixture.caller_addr,
    );
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(
        first_poll(execution.as_mut()),
        Poll::Ready(Ok(crate::env::Flow::Return(_)))
    ));
    drop(execution);
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "first-Ready callback must remain in the current Actor segment"
    );
    frame.finish(heap).expect("finish Ready callback frame");
}

#[tokio::test]
async fn f445h_e4r_spine_callback_pending_reacquires_before_finalize() {
    let fixture = fixture(20);
    let (frame, mut heap) = fixture.evaluator.actor_frame().await;
    let mut env = caller_env(&fixture, &mut heap);
    let context = default_program_context(&fixture.evaluator.interpreter)
        .with_runtime_assembly_target(fixture.target.clone());
    let mut eval = fixture.evaluator.eval_context_with(
        context,
        frame.clone(),
        &mut heap,
        &mut env,
        &fixture.caller_addr,
    );
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(
        !frame.has_execution_lease(),
        "first-Pending callback must release the Actor segment"
    );
    tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("callback completes")
        .expect("callback finalizes");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "callback completion must reacquire before caller-heap finalize"
    );
    frame.finish(heap).expect("finish Pending callback frame");
}

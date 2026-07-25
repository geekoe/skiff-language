use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::{
    ActivationPolicy, AssemblyIdentity, BlockIr, CanonicalPackageLinkPlan,
    DeploymentArtifactIdentity, DeploymentPolicy, DeploymentRevision, ExecutableBody, ExecutableIr,
    ExecutableKind, ExprIr, ExprRefIr, FileIrRef, FileIrUnit, MetadataValue, PackageArtifact,
    PackageArtifactRef, PackageBuildId, PackageCodeSlot, PackageImplementationLinks,
    PackageLocalAbi, PackageLocalAbiIdentity, PackageRuntimeRequirements, PackageSchemaIndexRef,
    ResourcePolicy, RuntimeAssembly, ServiceContract, ServiceContractRef, ServiceDeploymentRef,
    SlotLayout, StmtIr, StmtRefIr, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_activation::{
    ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
    RequestActivationContext, RuntimeActivation,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorRemoveControlRequest, ActorReplaceControlRequest,
    CapabilityError, CapabilityFuture, OwnedActorCapabilityContext, SpawnSubmitControlRequest,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::ActorRef,
};

use crate::{
    assembly_execution::ordinary::tests::test_runtime,
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    Interpreter, RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget,
};

const ASSEMBLY_ID: &str = "assembly:canonical-spawn-eval";
const PACKAGE_ID: &str = "example.canonical-spawn";
const TARGET_SYMBOL: &str = "spawn.fixture.run";

#[derive(Clone)]
struct RecordingActor {
    activation_identity: ActivationIdentityControl,
    submissions: Arc<Mutex<Vec<(SpawnSubmitControlRequest, Vec<u8>)>>>,
}

impl ActorCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "replica:canonical-spawn"
    }

    fn service_id(&self) -> &str {
        PACKAGE_ID
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:canonical-spawn"
    }

    fn request_target(&self) -> &str {
        "operation:submit"
    }

    fn request_build_id(&self) -> &str {
        "canonical-assembly-build"
    }

    fn spawn_service_protocol_identity(&self) -> &str {
        "protocol:canonical-spawn"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "protocol:canonical-spawn"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        Some("protocol:canonical-spawn")
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        Some(&self.activation_identity)
    }

    fn trace_id(&self) -> Option<&str> {
        Some("trace:canonical-spawn")
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor getOrCreate is not under test",
            ))
        })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor replace is not under test",
            ))
        })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("actor find is not under test")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor remove is not under test",
            ))
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        request: SpawnSubmitControlRequest,
        args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()> {
        let submissions = Arc::clone(&self.submissions);
        Box::pin(async move {
            submissions
                .lock()
                .expect("spawn recorder lock should remain available")
                .push((request, args_payload));
            Ok(())
        })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: skiff_runtime_capability_context::ActorInvocationRequest,
    ) -> CapabilityFuture<'a, skiff_runtime_capability_context::ActorInvocationOutcome> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "Actor invocation is not under test",
            ))
        })
    }
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

    fn admitted_schema_records(
        &self,
        _contract: &ServiceContractRef,
    ) -> Option<crate::AdmittedPackageSchemaRecords> {
        None
    }

    fn operation_target(
        &self,
        _activation_id: &ActivationId,
        _operation: &skiff_artifact_model::ContractOperationId,
    ) -> Option<skiff_artifact_model::OperationTargetRef> {
        None
    }
}

struct CanonicalSpawnFixture {
    eval_target: RuntimeAssemblyEvalTarget,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
    actor: ActorCapabilityContext<'static>,
    activation_identity: ActivationIdentityControl,
    submissions: Arc<Mutex<Vec<(SpawnSubmitControlRequest, Vec<u8>)>>>,
}

#[tokio::test]
async fn canonical_spawn_uses_admitted_projection_and_submits_exact_function_target() {
    let fixture = canonical_spawn_fixture(Some(TARGET_SYMBOL));
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.actor, Some(fixture.eval_target));
    let mut heap = RequestHeap::default();

    interpreter
        .execute_runtime_assembly_addr(context, &mut heap, &fixture.caller_addr, Vec::new())
        .await
        .expect("canonical spawn should submit from the admitted in-memory execution image");

    let submissions = fixture
        .submissions
        .lock()
        .expect("spawn submissions should be readable");
    let [(request, payload)] = submissions.as_slice() else {
        panic!("canonical spawn should submit exactly once");
    };
    assert_eq!(request.target_kind, "function");
    assert_eq!(request.target, format!("function:{TARGET_SYMBOL}"));
    assert_eq!(request.activation_identity, fixture.activation_identity);
    assert_eq!(&payload[..4], b"SKRE");
}

#[tokio::test]
async fn canonical_spawn_missing_metadata_fails_before_actor_capability() {
    let fixture = canonical_spawn_fixture(None);
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.actor, Some(fixture.eval_target));

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut RequestHeap::default(),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("canonical spawn without compiler metadata must fail closed");

    assert!(error
        .to_string()
        .contains("missing compiler spawnSubmit metadata"));
    assert!(fixture
        .submissions
        .lock()
        .expect("spawn submissions should be readable")
        .is_empty());
}

#[tokio::test]
async fn canonical_spawn_missing_execution_projection_fails_before_actor_capability() {
    let fixture = canonical_spawn_fixture(Some(TARGET_SYMBOL));
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.actor, None);

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut RequestHeap::default(),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("canonical execution without an admitted projection must fail closed");

    assert!(error
        .to_string()
        .contains("program execution context has no runtime assembly target"));
    assert!(fixture
        .submissions
        .lock()
        .expect("spawn submissions should be readable")
        .is_empty());
}

#[tokio::test]
async fn canonical_spawn_rejects_metadata_target_not_matching_linked_symbol() {
    let fixture = canonical_spawn_fixture(Some("spawn.fixture.other"));
    let interpreter = Interpreter::for_runtime_assembly(test_runtime::runtime_factory());
    let context = execution_context(&interpreter, fixture.actor, Some(fixture.eval_target));

    let error = interpreter
        .execute_runtime_assembly_addr(
            context,
            &mut RequestHeap::default(),
            &fixture.caller_addr,
            Vec::new(),
        )
        .await
        .expect_err("canonical spawn metadata cannot redirect the linked executable");

    assert!(error
        .to_string()
        .contains("does not match linked executable"));
    assert!(fixture
        .submissions
        .lock()
        .expect("spawn submissions should be readable")
        .is_empty());
}

fn canonical_spawn_fixture(metadata_symbol: Option<&str>) -> CanonicalSpawnFixture {
    let mut file = FileIrUnit::empty("spawn.fixture", "source:canonical-spawn");
    file.executables = vec![caller_executable(metadata_symbol), target_executable()];
    skiff_artifact_identity::assign_file_ir_identity(&mut file)
        .expect("canonical spawn File IR should receive an identity");
    let mut package = private_package(&file);
    skiff_artifact_identity::assign_package_artifact_identities(&mut package)
        .expect("canonical spawn package should receive identities");
    let package_ref = package_ref(&package);
    let assembly_identity = AssemblyIdentity::new(ASSEMBLY_ID);
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: assembly_identity.clone(),
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
    let image =
        crate::test_support::link_package_fixture(assembly.clone(), vec![(package, vec![file])]);
    let activation = activation_context(
        assembly_identity.clone(),
        package_ref.package_build_id.clone(),
    );
    let request = RequestActivationContext::begin(Arc::clone(&activation))
        .expect("canonical spawn request generation should begin");
    let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(TestResolver {
        activation: Arc::clone(&activation),
    });
    let eval_target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
        .expect("canonical spawn eval target should use admitted facts");
    let activation_identity = ActivationIdentityControl {
        assembly_identity,
        generation: activation.identity().assembly_generation,
        runtime_replica_id: activation.identity().runtime_replica_id.clone(),
        deployment_revision: activation.identity().deployment.deployment_revision.clone(),
    };
    let submissions = Arc::new(Mutex::new(Vec::new()));
    let actor = ActorCapabilityContext::new(RecordingActor {
        activation_identity: activation_identity.clone(),
        submissions: Arc::clone(&submissions),
    });
    CanonicalSpawnFixture {
        eval_target,
        caller_addr: skiff_runtime_linked_program::ExecutableAddr::package(0, 0, 0),
        actor,
        activation_identity,
        submissions,
    }
}

fn caller_executable(metadata_symbol: Option<&str>) -> ExecutableIr {
    let mut metadata = BTreeMap::new();
    if let Some(symbol) = metadata_symbol {
        metadata.insert(
            "spawnSubmit".to_string(),
            MetadataValue::Object(BTreeMap::from([
                (
                    "targetKind".to_string(),
                    MetadataValue::String("function".to_string()),
                ),
                (
                    "target".to_string(),
                    MetadataValue::String(format!("function:{symbol}")),
                ),
            ])),
        );
    }
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: "spawn.fixture.submit".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: true,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                StmtIr::Spawn {
                    call: ExprRefIr { expression: 0 },
                },
                StmtIr::Return { value: None },
            ],
            expressions: vec![ExprIr::Call {
                call: skiff_artifact_model::CallIr {
                    target: skiff_artifact_model::CallTargetIr::LocalExecutable {
                        executable_index: 1,
                    },
                    args: Vec::new(),
                    type_args: BTreeMap::new(),
                    metadata,
                },
            }],
        },
        source_span: None,
    }
}

fn target_executable() -> ExecutableIr {
    ExecutableIr {
        kind: ExecutableKind::Function,
        symbol: TARGET_SYMBOL.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: skiff_artifact_model::TypeRefIr::builtin("null"),
        self_type: None,
        slots: SlotLayout::default(),
        may_suspend: false,
        body: ExecutableBody::default(),
        source_span: None,
    }
}

fn private_package(file: &FileIrUnit) -> PackageArtifact {
    PackageArtifact {
        schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
        package_id: PACKAGE_ID.to_string(),
        package_version: "1.0.0".to_string(),
        package_build_id: PackageBuildId::new("unassigned"),
        files: vec![FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }],
        static_resources: Vec::new(),
        package_local_abi: PackageLocalAbi {
            local_abi_identity: PackageLocalAbiIdentity::new("unassigned"),
            public_symbols: BTreeMap::new(),
            implementation_symbols: BTreeMap::new(),
        },
        package_schema_index: PackageSchemaIndexRef {
            package_id: PACKAGE_ID.to_string(),
            package_schema_index_identity: skiff_artifact_identity::package_schema_index_identity(
                PACKAGE_ID,
                &BTreeMap::new(),
            )
            .expect("empty Package schema index is canonical"),
        },
        package_schema_type_records: BTreeMap::new(),
        implementation_links: PackageImplementationLinks::default(),
        callable_links: BTreeMap::new(),
        package_requirements: Vec::new(),
        contract_requirements: Vec::new(),
        service_requirements: Vec::new(),
        runtime_requirements: PackageRuntimeRequirements {
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
            assembly_generation: 7,
            runtime_replica_id: "replica:canonical-spawn".to_string(),
            deployment: ServiceDeploymentRef {
                service_id: PACKAGE_ID.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("canonical-spawn-r1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new(
                    "deployment:canonical-spawn",
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
                timeout_ms: Some(1_000),
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
    .expect("canonical spawn activation should build")
}

fn execution_context<'a>(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    target: Option<RuntimeAssemblyEvalTarget>,
) -> ProgramExecutionContext<'a> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let runtime_activation = Arc::new(RuntimeActivation {
        service: skiff_runtime_linked_program::ServiceMeta {
            id: PACKAGE_ID.to_string(),
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
    let context = ProgramExecutionContext::new(ProgramExecutionInput {
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
    });
    match target {
        Some(target) => context.with_runtime_assembly_target(target),
        None => context,
    }
}

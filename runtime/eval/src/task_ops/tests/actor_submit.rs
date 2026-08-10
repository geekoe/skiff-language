use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use base64::Engine as _;
use serde_json::json;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, ActorMethodIdentity,
    InstructionSourceSite, LiteralIr, MetadataValue, SyntheticInstructionSiteReason,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorInvocationRequest, ActorRemoveControlRequest,
    ActorReplaceControlRequest, CapabilityError, CapabilityFuture, OwnedActorCapabilityContext,
    OwnedExecutionControl, OwnedRequestCapabilityContext, RequestCapabilityApi,
    RequestCapabilityContext, TaskCancelControlRequest, TaskCancelControlResponse,
    TaskStatusControlRequest, TaskStatusControlResponse, TaskSubmitControlRequest,
    TaskSubmitResponseControl,
};
use skiff_runtime_linked_program::{
    BlockIr, CallIr, ExecutableAddr, ExecutableKind, ExprRefIr, FileAddr, FileDeclarations,
    FileLinkTargets, LinkOverlay, LinkedActorCreateMethod, LinkedActorDeclaration,
    LinkedActorDeclarationOwner, LinkedActorField, LinkedActorMethodDispatchPlan,
    LinkedActorMethodImplementation, LinkedActorPublicMethod, LinkedCallTarget, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedFunctionTypeParamIr, LinkedStmtIr,
    RuntimeTypeContext, SourceMapDto, StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
};

use crate::{
    actor_executor::ActorExecutionFrame,
    actor_executor_test_runtime as test_runtime,
    actor_instance::{
        ActorActivationRequest, ActorExecutorAuthority, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceHandle, ActorInstanceStore, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::TimeCapabilityContext,
    env::Env,
    heap_access::HeapAccess,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    test_support::runtime_execution_package_fixture_with_identity,
    EvalRuntimeProgram, Interpreter,
};

const FILE_ID: &str = "file:actor-task-submit";
const SERVICE_ID: &str = "example.com/actor-task-submit";
const ACTOR_TYPE_ID: &str = "svc.main.Counter";
const ACTOR_ABI: &str = "skiff-actor-abi-v1:sha256:actor-task-submit";
const ACTOR_IMPLEMENTATION: &str = "skiff-actor-implementation-v1:sha256:actor-task-submit";
const ACTOR_METHOD: &str = "skiff-actor-method-v1:sha256:actor-task-run";

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new(ACTOR_ABI)
}

fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(ACTOR_IMPLEMENTATION)
}

fn method_identity() -> ActorMethodIdentity {
    ActorMethodIdentity::new(ACTOR_METHOD)
}

fn actor_owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        actor_symbol: "Counter".to_string(),
    }
}

fn actor_ref_for(actor_id_bytes: &[u8], epoch: u64) -> ActorRef {
    ActorRef::new(
        SERVICE_ID,
        ACTOR_TYPE_ID,
        "builtin:string",
        ACTOR_BOOTSTRAP_ENCODING_V1,
        actor_id_bytes.to_vec(),
        format!("sha256:{}", hex::encode(Sha256::digest(actor_id_bytes))),
        Some(epoch),
    )
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn string_type() -> skiff_runtime_linked_program::LinkedTypeRef {
    skiff_runtime_linked_program::LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

fn integer_type() -> skiff_runtime_linked_program::LinkedTypeRef {
    skiff_runtime_linked_program::LinkedTypeRef::Native {
        name: "integer".to_string(),
        args: Vec::new(),
    }
}

fn null_type() -> skiff_runtime_linked_program::LinkedTypeRef {
    skiff_runtime_linked_program::LinkedTypeRef::Native {
        name: "null".to_string(),
        args: Vec::new(),
    }
}

fn linked_fixture(
    create_param_type: skiff_runtime_linked_program::LinkedTypeRef,
    has_create: bool,
) -> Arc<LinkedFileUnit> {
    let mut declarations = FileDeclarations::default();
    let _ = declarations.types.insert(
        "Counter".to_string(),
        skiff_runtime_linked_program::linked::TypeDeclarationIr {
            type_index: 0,
            symbol: "Counter".to_string(),
            source_span: None,
        },
    );
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: FILE_ID.to_string(),
        source_ast_hash: "source:actor-task-submit".to_string(),
        module_path: "svc.main".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations,
        link_targets: FileLinkTargets::default(),
        actor_declarations: vec![LinkedActorDeclaration {
            actor_type: skiff_runtime_linked_program::ServiceSymbolRef {
                module_path: "svc.main".to_string(),
                symbol: "Counter".to_string(),
            },
            implementation_owner: Some(actor_owner()),
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            actor_name: "Counter".to_string(),
            actor_id_type: string_type(),
            key_field: "id".to_string(),
            fields: vec![LinkedActorField {
                name: "id".to_string(),
                ty: string_type(),
                encoding: ActorFieldEncodingIr::CanonicalValueV1,
            }],
            create: has_create.then(|| LinkedActorCreateMethod {
                method_identity: ActorMethodIdentity::new("skiff-actor-method-v1:sha256:create"),
                parameters: vec![LinkedFunctionTypeParamIr {
                    name: "accountId".to_string(),
                    ty: create_param_type,
                }],
                implementation: LinkedActorMethodImplementation::LocalExecutable {
                    executable_index: 1,
                },
            }),
            public_methods: vec![LinkedActorPublicMethod {
                method_identity: method_identity(),
                name: "run".to_string(),
                parameters: Vec::new(),
                return_type: null_type(),
                may_suspend: false,
                implementation: LinkedActorMethodImplementation::LocalExecutable {
                    executable_index: 2,
                },
            }],
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }],
        types: vec![skiff_runtime_linked_program::anonymous_type_decl(
            "Counter",
            skiff_runtime_linked_program::LinkedTypeDescriptor::Record {
                fields: BTreeMap::new(),
            },
        )],
        constants: Vec::new(),
        executables: vec![
            caller_executable(),
            empty_executable("Counter.create", 1),
            empty_executable("Counter.run", 2),
        ],
        external_refs: skiff_runtime_linked_program::ExternalRefTable::default(),
    })
}

fn caller_executable() -> LinkedExecutable {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "dispatchSubmit".to_string(),
        MetadataValue::Object(BTreeMap::from([
            (
                "targetKind".to_string(),
                MetadataValue::String("actorMethod".to_string()),
            ),
            (
                "target".to_string(),
                MetadataValue::String(format!("actorMethod:Counter:{ACTOR_METHOD}")),
            ),
        ])),
    );
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "Counter.submit".to_string(),
        type_params: Vec::new(),
        params: vec![skiff_runtime_linked_program::ParamIr {
            name: "self".to_string(),
            slot: 0,
            ty: null_type(),
            mode: skiff_runtime_linked_program::ParamModeIr::Value,
        }],
        return_type: Some(null_type()),
        self_type: None,
        slots: skiff_runtime_linked_program::SlotLayoutIr {
            slots: vec![skiff_runtime_linked_program::SlotIr {
                index: 0,
                name: "self".to_string(),
                kind: "selfValue".to_string(),
                writable_local: false,
            }],
            frame_size: 1,
        },
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                LinkedStmtIr::Dispatch {
                    call: ExprRefIr { expression: 0 },
                },
                LinkedStmtIr::Return { value: None },
            ],
            expressions: vec![
                LinkedExprIr::Call {
                    call: CallIr {
                        target: LinkedCallTarget::ActorDispatch {
                            plan: LinkedActorMethodDispatchPlan {
                                declaration_owner: actor_owner(),
                                actor_abi_identity: actor_abi(),
                                actor_implementation_identity: actor_implementation(),
                                method_identity: method_identity(),
                            },
                        },
                        concrete_receiver: None,
                        site: synthetic_site(),
                        args: vec![ExprRefIr { expression: 1 }],
                        inout_args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata,
                        actor_metadata: None,
                    },
                },
                LinkedExprIr::LoadSlot { slot: 0 },
            ],
        },
    }
}

fn empty_executable(symbol: &str, executable_index: u32) -> LinkedExecutable {
    let _ = executable_index;
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(null_type()),
        self_type: None,
        slots: skiff_runtime_linked_program::SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody::default(),
    }
}

#[derive(Clone)]
struct RecordingTaskActor {
    activation_identity: ActivationIdentityControl,
    submissions: Arc<Mutex<Vec<(TaskSubmitControlRequest, Vec<u8>)>>>,
    replies: Arc<Mutex<VecDeque<Result<TaskSubmitResponseControl, CapabilityError>>>>,
    task_seq: Arc<AtomicU64>,
}

impl RecordingTaskActor {
    fn new() -> Self {
        Self {
            activation_identity: ActivationIdentityControl {
                assembly_identity: AssemblyIdentity::new(format!(
                    "skiff-runtime-assembly-v3:sha256:{}",
                    "a".repeat(64)
                )),
                generation: 1,
                runtime_replica_id: "replica:actor-task-submit".to_string(),
                deployment_revision: DeploymentRevision::new("rev-1"),
            },
            submissions: Arc::new(Mutex::new(Vec::new())),
            replies: Arc::new(Mutex::new(VecDeque::new())),
            task_seq: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl ActorCapabilityApi for RecordingTaskActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("actor find is not under test")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "actor remove is not under test",
            ))
        })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, skiff_runtime_capability_context::ActorInvocationOutcome> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "Actor invocation is not under test",
            ))
        })
    }
}

impl RequestCapabilityApi for RecordingTaskActor {
    fn owned(&self) -> OwnedRequestCapabilityContext {
        RequestCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> RequestCapabilityContext<'_> {
        RequestCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "replica:actor-task-submit"
    }

    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:actor-task-submit"
    }

    fn request_target(&self) -> &str {
        "operation:submit"
    }

    fn request_build_id(&self) -> &str {
        "build:actor-task-submit"
    }

    fn task_service_protocol_identity(&self) -> &str {
        "protocol:actor-task-submit"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "protocol:actor-task-submit"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        Some("protocol:actor-task-submit")
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        Some(&self.activation_identity)
    }

    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn submit_task<'a>(
        &'a self,
        request: TaskSubmitControlRequest,
        args_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl> {
        let submissions = Arc::clone(&self.submissions);
        let replies = Arc::clone(&self.replies);
        let task_seq = Arc::clone(&self.task_seq);
        Box::pin(async move {
            submissions
                .lock()
                .expect("task submissions lock")
                .push((request.clone(), args_payload));
            let reply = replies
                .lock()
                .expect("task replies lock")
                .pop_front()
                .unwrap_or_else(|| {
                    let seq = task_seq.fetch_add(1, Ordering::Relaxed) + 1;
                    let task_id = request
                        .task_id
                        .clone()
                        .unwrap_or_else(|| format!("task-{seq}"));
                    Ok(TaskSubmitResponseControl {
                        task_ref: format!("skiff-task-v1:{}:{}", SERVICE_ID, task_id),
                        task_id,
                        request_id: format!("request-{seq}"),
                    })
                });
            reply
        })
    }

    fn status_task<'a>(
        &'a self,
        _request: TaskStatusControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskStatusControlResponse> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "task status is not under test",
            ))
        })
    }

    fn cancel_task<'a>(
        &'a self,
        _request: TaskCancelControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskCancelControlResponse> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "task cancel is not under test",
            ))
        })
    }
}

struct ActorSubmitFixture {
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
    caller_addr: ExecutableAddr,
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
    actor_ref: ActorRef,
    frame: Option<ActorExecutionFrame>,
    recording: RecordingTaskActor,
}

impl ActorSubmitFixture {
    async fn new(
        create_param_type: skiff_runtime_linked_program::LinkedTypeRef,
        with_frame: bool,
    ) -> Self {
        Self::new_with_create(create_param_type, with_frame, true).await
    }

    /// Create-less keyed actor fixture: no `create` declaration and the
    /// canonical empty-array activation bootstrap (`[]`), which is the shape
    /// produced by `std.actor.get` on a create-less actor (E2a regression).
    async fn new_without_create(with_frame: bool) -> Self {
        Self::new_with_create(string_type(), with_frame, false).await
    }

    async fn new_with_create(
        create_param_type: skiff_runtime_linked_program::LinkedTypeRef,
        with_frame: bool,
        has_create: bool,
    ) -> Self {
        let file = linked_fixture(create_param_type, has_create);
        let package = runtime_execution_package_fixture_with_identity(
            "example.com/actor-task-submit-package",
            "1.0.0",
            "build:actor-task-submit",
            "abi:actor-task-submit",
            0,
            vec![Arc::clone(&file)],
            skiff_runtime_linked_program::PublicationResourceTable::default(),
        );
        let program = Arc::new(EvalRuntimeProgram::new(
            SERVICE_ID,
            vec![Arc::clone(&file)],
            vec![package],
            skiff_runtime_linked_program::PublicationResourceTable::default(),
            HashMap::new(),
            LinkOverlay::default(),
            RuntimeTypeContext::default(),
        ));
        let interpreter =
            Interpreter::with_program(Arc::clone(&program), test_runtime::runtime_factory());
        let caller_addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
            executable: 0,
        };
        let actor_id_bytes = br#""actor-1""#.to_vec();
        let actor_ref = actor_ref_for(&actor_id_bytes, 1);
        let mut store = ActorInstanceStore::new();
        let handle = store
            .activate(ActorActivationRequest {
                fence: ActorInstanceFence {
                    incarnation: ActorIncarnationKey {
                        logical_key: ActorLogicalKey {
                            service_id: SERVICE_ID.to_string(),
                            actor_type_identity: ACTOR_TYPE_ID.to_string(),
                            actor_id_type_identity: "builtin:string".to_string(),
                            actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                            actor_id_hash: format!(
                                "sha256:{}",
                                hex::encode(Sha256::digest(&actor_id_bytes))
                            ),
                            canonical_actor_id_key_bytes: actor_id_bytes,
                        },
                        epoch: 1,
                    },
                    actor_abi_identity: actor_abi(),
                    actor_implementation_identity: actor_implementation(),
                    declaration_owner: actor_owner(),
                },
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: if has_create {
                    br#"["account-1"]"#
                } else {
                    br#"[]"#
                },
                program: program.projection().type_view(),
            })
            .expect("actor task submit fixture activation");
        store
            .with_fields_for_executor(&ActorExecutorAuthority::new(), &handle, |fields, _| {
                fields[0].value = RuntimeValue::String("actor-1".to_string());
                fields[0].assigned = true;
            })
            .expect("actor task submit fixture fields");
        store
            .mark_admitted(&ActorExecutorAuthority::new(), &handle)
            .expect("actor task submit fixture admission");

        let frame = if with_frame {
            let authority = ActorExecutorAuthority::new();
            let segment = store
                .acquire_segment(&authority, &handle)
                .await
                .expect("actor task submit fixture segment");
            Some(ActorExecutionFrame::new(
                store.clone(),
                handle.clone(),
                segment,
                false,
            ))
        } else {
            None
        };
        let recording = RecordingTaskActor::new();
        Self {
            interpreter,
            file,
            caller_addr,
            store,
            handle,
            actor_ref,
            frame,
            recording,
        }
    }

    fn context(&self) -> ProgramExecutionContext<'static> {
        let recording = self.recording.clone();
        let execution = test_runtime::execution_control();
        let effects = test_runtime::effects_context();
        let actor = ActorCapabilityContext::new(recording.clone());
        let request = RequestCapabilityContext::new(recording);
        let mut context = ProgramExecutionContext::new(ProgramExecutionInput {
            execution: execution.clone(),
            config: test_runtime::config_context(),
            db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
            file: test_runtime::file_context(),
            file_source_stream: test_runtime::file_source_stream_context(
                self.interpreter.stream_runtime.clone(),
            ),
            time: TimeCapabilityContext::new(execution),
            websocket: test_runtime::websocket_context(),
            effects: effects.clone(),
            http_client: effects.http_client_context(
                self.interpreter.http_options.clone(),
                self.interpreter.stream_runtime.clone(),
                self.interpreter.test_effect_double_context(),
            ),
            test_effect_doubles: self.interpreter.test_effect_double_context(),
            actor,
            request,
            request_heap_limits: RequestHeapLimits::default(),
        });
        if let Some(frame) = self.frame.clone() {
            context = context.with_actor_execution_frame(frame);
        }
        context
    }

    /// External-context view: no actor execution frame, but the Runtime's
    /// actor instance store is installed exactly like ordinary host request
    /// contexts (F0b).
    fn context_with_actor_store(&self) -> ProgramExecutionContext<'static> {
        self.context()
            .with_actor_instance_store(Arc::new(self.store.clone()))
    }

    async fn run_submit(&self) -> Result<RuntimeValue, crate::error::RuntimeError> {
        let context = self.context();
        let mut heap = HeapAccess::private(RequestHeap::default());
        self.interpreter
            .call_program_executable_with_self_direct(
                context,
                &mut heap,
                &Env::new(),
                &self.caller_addr,
                &self.caller_addr,
                &Default::default(),
                RuntimeValue::ActorRef(self.actor_ref.clone()),
                Vec::new(),
            )
            .await
    }
}

#[tokio::test]
async fn actor_method_submit_external_context_freezes_snapshot_and_submits_once() {
    let fixture = ActorSubmitFixture::new(string_type(), false).await;
    let context = fixture.context_with_actor_store();
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = fixture
        .interpreter
        .call_program_executable_with_self_direct(
            context,
            &mut heap,
            &Env::new(),
            &fixture.caller_addr,
            &fixture.caller_addr,
            &Default::default(),
            RuntimeValue::ActorRef(fixture.actor_ref.clone()),
            Vec::new(),
        )
        .await
        .expect("external-context actor method dispatch should submit");
    assert_eq!(value, RuntimeValue::Null);

    let submissions = fixture
        .recording
        .submissions
        .lock()
        .expect("task submissions lock");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("external-context actor method dispatch should submit exactly once");
    };
    assert_eq!(request.target_kind, "actorMethod");
    let actor_method = request
        .actor_method
        .as_ref()
        .expect("actor method target metadata");
    assert_eq!(actor_method.actor_ref.epoch(), Some(1));
    let key = base64::engine::general_purpose::STANDARD
        .decode(&actor_method.activation.key)
        .expect("snapshot key base64");
    let key_value: serde_json::Value =
        serde_json::from_slice(&key).expect("snapshot key canonical JSON");
    assert_eq!(key_value["serviceId"], SERVICE_ID);
    assert_eq!(key_value["actorTypeIdentity"], ACTOR_TYPE_ID);
    assert_eq!(
        actor_method.activation.create_input,
        base64::engine::general_purpose::STANDARD.encode(br#"["account-1"]"#)
    );
    let plan = &actor_method.activation.expected_type_plan;
    assert_eq!(plan["label"], "record");
    assert_eq!(plan["node"]["kind"], "record");
    let fields = plan["node"]["fields"].as_array().expect("plan fields");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "accountId");
    assert_eq!(fields[0]["ty"]["node"]["kind"], "string");
}

#[tokio::test]
async fn actor_method_submit_external_context_missing_incarnation_rejects_before_task() {
    let fixture = ActorSubmitFixture::new(string_type(), false).await;
    let context = fixture.context_with_actor_store();
    let mut heap = HeapAccess::private(RequestHeap::default());
    let unknown_ref = actor_ref_for(br#""actor-unknown""#, 1);
    let error = fixture
        .interpreter
        .call_program_executable_with_self_direct(
            context,
            &mut heap,
            &Env::new(),
            &fixture.caller_addr,
            &fixture.caller_addr,
            &Default::default(),
            RuntimeValue::ActorRef(unknown_ref),
            Vec::new(),
        )
        .await
        .expect_err("external actor handle without a local incarnation must fail closed");
    assert!(
        error
            .to_string()
            .contains("no authenticated actor registry entry"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .recording
            .submissions
            .lock()
            .expect("task submissions lock")
            .is_empty(),
        "definite rejection must not produce a task"
    );
}

#[tokio::test]
async fn actor_method_submit_freezes_snapshot_and_submits_once() {
    let fixture = ActorSubmitFixture::new(string_type(), true).await;
    let value = fixture
        .run_submit()
        .await
        .expect("actor method dispatch should submit");
    assert_eq!(value, RuntimeValue::Null);

    let submissions = fixture
        .recording
        .submissions
        .lock()
        .expect("task submissions lock");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("actor method dispatch should submit exactly once");
    };
    assert_eq!(request.target_kind, "actorMethod");
    assert_eq!(
        request.target,
        format!("actorMethod:Counter:{ACTOR_METHOD}")
    );
    let actor_method = request
        .actor_method
        .as_ref()
        .expect("actor method target metadata");
    assert_eq!(actor_method.actor_ref.epoch(), Some(1));
    let key = base64::engine::general_purpose::STANDARD
        .decode(&actor_method.activation.key)
        .expect("snapshot key base64");
    let key_value: serde_json::Value =
        serde_json::from_slice(&key).expect("snapshot key canonical JSON");
    assert_eq!(key_value["serviceId"], SERVICE_ID);
    assert_eq!(key_value["actorTypeIdentity"], ACTOR_TYPE_ID);
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(
                key_value["canonicalActorIdKeyBytesBase64"]
                    .as_str()
                    .unwrap()
            )
            .expect("actor id base64"),
        br#""actor-1""#
    );
    assert_eq!(
        actor_method.activation.create_input,
        base64::engine::general_purpose::STANDARD.encode(br#"["account-1"]"#)
    );
    let plan = &actor_method.activation.expected_type_plan;
    assert_eq!(plan["label"], "record");
    assert_eq!(plan["node"]["kind"], "record");
    let fields = plan["node"]["fields"].as_array().expect("plan fields");
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0]["name"], "accountId");
    assert_eq!(fields[0]["ty"]["node"]["kind"], "string");
}

#[tokio::test]
async fn actor_method_submit_keyed_actor_without_create_submits_once() {
    // Regression: a keyed actor without a create declaration is activated with
    // the canonical empty-array bootstrap `[]`. The E2a snapshot must not
    // treat those 2 bytes as a create input and reject before submission.
    let fixture = ActorSubmitFixture::new_without_create(true).await;
    let value = fixture
        .run_submit()
        .await
        .expect("create-less keyed actor dispatch should submit");
    assert_eq!(value, RuntimeValue::Null);

    let submissions = fixture
        .recording
        .submissions
        .lock()
        .expect("task submissions lock");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("create-less actor method dispatch should submit exactly once");
    };
    assert_eq!(request.target_kind, "actorMethod");
    let actor_method = request
        .actor_method
        .as_ref()
        .expect("actor method target metadata");
    assert_eq!(
        actor_method.activation.create_input,
        base64::engine::general_purpose::STANDARD.encode(br#"[]"#)
    );
    let plan = &actor_method.activation.expected_type_plan;
    assert_eq!(plan["label"], "record");
    assert_eq!(plan["node"]["kind"], "record");
    let fields = plan["node"]["fields"].as_array().expect("plan fields");
    assert!(
        fields.is_empty(),
        "create-less actor snapshot plan must have no create fields"
    );
}

#[tokio::test]
async fn actor_method_submit_keyed_actor_without_create_external_context_submits_once() {
    // Regression (chat shape): an external context with the Runtime's actor
    // instance store dispatches `actor.method(...)` on a create-less keyed
    // actor whose activation bootstrap is `[]`. The submission must succeed.
    let fixture = ActorSubmitFixture::new_without_create(false).await;
    let context = fixture.context_with_actor_store();
    let mut heap = HeapAccess::private(RequestHeap::default());
    let value = fixture
        .interpreter
        .call_program_executable_with_self_direct(
            context,
            &mut heap,
            &Env::new(),
            &fixture.caller_addr,
            &fixture.caller_addr,
            &Default::default(),
            RuntimeValue::ActorRef(fixture.actor_ref.clone()),
            Vec::new(),
        )
        .await
        .expect("external-context create-less actor dispatch should submit");
    assert_eq!(value, RuntimeValue::Null);

    let submissions = fixture
        .recording
        .submissions
        .lock()
        .expect("task submissions lock");
    let [(request, _payload)] = submissions.as_slice() else {
        panic!("external-context create-less actor dispatch should submit exactly once");
    };
    assert_eq!(request.target_kind, "actorMethod");
    let actor_method = request
        .actor_method
        .as_ref()
        .expect("actor method target metadata");
    assert_eq!(
        actor_method.activation.create_input,
        base64::engine::general_purpose::STANDARD.encode(br#"[]"#)
    );
    let fields = actor_method.activation.expected_type_plan["node"]["fields"]
        .as_array()
        .expect("plan fields");
    assert!(
        fields.is_empty(),
        "create-less actor snapshot plan must have no create fields"
    );
}

#[tokio::test]
async fn actor_method_submit_without_frame_rejects_before_task() {
    let fixture = ActorSubmitFixture::new(string_type(), false).await;
    let error = fixture
        .run_submit()
        .await
        .expect_err("actor method dispatch without a frame must fail closed");
    assert!(
        error
            .to_string()
            .contains("no authenticated actor registry entry"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .recording
            .submissions
            .lock()
            .expect("task submissions lock")
            .is_empty(),
        "definite rejection must not produce a task"
    );
}

#[tokio::test]
async fn actor_method_submit_unknown_receiver_rejects_before_task() {
    let fixture = ActorSubmitFixture::new(string_type(), true).await;
    let context = fixture.context();
    let mut heap = HeapAccess::private(RequestHeap::default());
    let unknown_ref = actor_ref_for(br#""actor-unknown""#, 1);
    let error = fixture
        .interpreter
        .call_program_executable_with_self_direct(
            context,
            &mut heap,
            &Env::new(),
            &fixture.caller_addr,
            &fixture.caller_addr,
            &Default::default(),
            RuntimeValue::ActorRef(unknown_ref),
            Vec::new(),
        )
        .await
        .expect_err("unknown actor receiver must fail closed");
    assert!(
        error
            .to_string()
            .contains("registry entry is not available"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .recording
            .submissions
            .lock()
            .expect("task submissions lock")
            .is_empty(),
        "definite rejection must not produce a task"
    );
}

#[tokio::test]
async fn actor_method_submit_unrecoverable_create_input_rejects_before_task() {
    // The create declaration expects `integer`, but the frozen activation
    // input is a string: the recoverable gate must reject before any
    // task.submit.request.
    let fixture = ActorSubmitFixture::new(integer_type(), true).await;
    let error = fixture
        .run_submit()
        .await
        .expect_err("unrecoverable create input must fail closed");
    assert!(
        error.to_string().contains("Actor create"),
        "unexpected error: {error}"
    );
    assert!(
        fixture
            .recording
            .submissions
            .lock()
            .expect("task submissions lock")
            .is_empty(),
        "definite rejection must not produce a task"
    );
}

#[tokio::test]
async fn actor_method_submit_receiver_argument_evaluated_once() {
    let fixture = ActorSubmitFixture::new(string_type(), true).await;
    let _ = fixture
        .run_submit()
        .await
        .expect("actor method dispatch should submit");
    assert_eq!(
        fixture
            .recording
            .submissions
            .lock()
            .expect("task submissions lock")
            .len(),
        1,
        "receiver must be evaluated exactly once"
    );
}

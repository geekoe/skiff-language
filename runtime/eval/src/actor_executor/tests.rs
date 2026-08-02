use std::{collections::BTreeMap, sync::Arc, time::Duration};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, ActorMethodIdentity,
    InstructionSourceSite, SyntheticInstructionSiteReason, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_runtime_linked_program::{
    linked::TypeDeclarationIr, AssignTargetIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr,
    ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay,
    LinkedActorCreateMethod, LinkedActorDeclaration, LinkedActorDeclarationOwner, LinkedActorField,
    LinkedActorMethodImplementation, LinkedActorPublicMethod, LinkedExecutable,
    LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedFunctionTypeParamIr, LinkedStmtIr,
    LinkedTypeDescriptor, LinkedTypeRef, ParamIr, PublicationResourceTable, RuntimeTypeContext,
    ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, TypeAddr,
    TypeDeclIr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    runtime_value::{
        CallbackCapabilityCarrier, HeapNode, InterfaceCarrier, InterfaceValue, RuntimeObject,
        RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
    },
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, RequestException,
    },
};

use super::*;
use crate::{
    actor_executor_test_runtime as test_runtime,
    actor_instance::{
        ActorActivationRequest, ActorFieldValue, ActorIncarnationKey, ActorInstanceFence,
        ActorInstanceSessionTracker, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram,
};
use sha2::{Digest, Sha256};
const FILE_ID: &str = "file:actor-executor";

struct Fixture {
    interpreter: Interpreter,
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
    method: ActorMethodIdentity,
}

fn integer() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "integer".to_string(),
        args: Vec::new(),
    }
}

fn string() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

fn array(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Array".to_string(),
        args: vec![item],
    }
}

fn payload_type() -> LinkedTypeRef {
    LinkedTypeRef::Address {
        addr: TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        },
    }
}

fn payload_descriptor() -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: BTreeMap::from([(
            "items".to_string(),
            array(LinkedTypeRef::Record {
                fields: BTreeMap::from([
                    ("name".to_string(), string()),
                    ("tags".to_string(), array(string())),
                ]),
            }),
        )]),
    }
}

fn owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        actor_symbol: "Counter".to_string(),
    }
}

fn abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:counter")
}

fn implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:counter")
}

fn method_identity() -> ActorMethodIdentity {
    ActorMethodIdentity::new("skiff-actor-method-v1:sha256:set")
}

fn actor_file(return_type: LinkedTypeRef, may_suspend: bool) -> Arc<LinkedFileUnit> {
    let method = method_identity();
    let mut declarations = FileDeclarations::default();
    declarations.types.insert(
        "Payload".to_string(),
        TypeDeclarationIr {
            type_index: 0,
            symbol: "Payload".to_string(),
            source_span: None,
        },
    );
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: FILE_ID.to_string(),
        source_ast_hash: "source:actor-executor".to_string(),
        module_path: "actors".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations,
        link_targets: FileLinkTargets::default(),
        actor_declarations: vec![LinkedActorDeclaration {
            actor_type: ServiceSymbolRef {
                module_path: "actors".to_string(),
                symbol: "Counter".to_string(),
            },
            implementation_owner: Some(owner()),
            actor_abi_identity: abi(),
            actor_implementation_identity: implementation(),
            actor_name: "Counter".to_string(),
            actor_id_type: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
            key_field: "id".to_string(),
            fields: vec![
                LinkedActorField {
                    name: "id".to_string(),
                    ty: LinkedTypeRef::Native {
                        name: "string".to_string(),
                        args: Vec::new(),
                    },
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                LinkedActorField {
                    name: "count".to_string(),
                    ty: integer(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                LinkedActorField {
                    name: "payload".to_string(),
                    ty: payload_type(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
                LinkedActorField {
                    name: "payload_alias".to_string(),
                    ty: payload_type(),
                    encoding: ActorFieldEncodingIr::CanonicalValueV1,
                },
            ],
            create: None,
            public_methods: vec![LinkedActorPublicMethod {
                method_identity: method,
                name: "set".to_string(),
                parameters: vec![LinkedFunctionTypeParamIr {
                    name: "value".to_string(),
                    ty: integer(),
                }],
                return_type,
                may_suspend,
                implementation: LinkedActorMethodImplementation::LocalExecutable {
                    executable_index: 0,
                },
            }],
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }],
        types: vec![TypeDeclIr {
            name: "Payload".to_string(),
            descriptor: payload_descriptor(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        }],
        constants: Vec::new(),
        executables: vec![LinkedExecutable {
            kind: ExecutableKind::ImplMethod,
            symbol: "Counter.set".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "value".to_string(),
                slot: 0,
                ty: integer(),
            }],
            return_type: Some(integer()),
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "value".to_string(),
                    kind: "parameter".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend,
            body: LinkedExecutableBody {
                blocks: vec![BlockIr {
                    label: "entry".to_string(),
                    statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
                }],
                statements: vec![
                    LinkedStmtIr::Assign {
                        target: AssignTargetIr::ActorSelfField {
                            field: "count".to_string(),
                            field_type: integer(),
                        },
                        value: ExprRefIr { expression: 0 },
                    },
                    LinkedStmtIr::Return {
                        value: Some(ExprRefIr { expression: 1 }),
                    },
                ],
                expressions: vec![
                    LinkedExprIr::LoadSlot { slot: 0 },
                    LinkedExprIr::ActorSelfField {
                        field: "count".to_string(),
                        field_type: integer(),
                    },
                ],
            },
        }],
        external_refs: ExternalRefTable::default(),
    })
}

fn actor_file_with_create() -> Arc<LinkedFileUnit> {
    let mut file = (*actor_file(integer(), false)).clone();
    file.actor_declarations[0].create = Some(LinkedActorCreateMethod {
        method_identity: ActorMethodIdentity::new("skiff-actor-method-v1:sha256:create"),
        parameters: Vec::new(),
        implementation: LinkedActorMethodImplementation::LocalExecutable {
            executable_index: 1,
        },
    });
    file.executables.push(LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "Counter.create".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: Vec::new(),
            frame_size: 0,
        },
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![LinkedStmtIr::Return { value: None }],
            expressions: Vec::new(),
        },
    });
    Arc::new(file)
}

fn counter_fence(actor_id: &str) -> ActorInstanceFence {
    let id_bytes = serde_json::to_vec(actor_id).unwrap();
    ActorInstanceFence {
        incarnation: ActorIncarnationKey {
            logical_key: ActorLogicalKey {
                service_id: "skiff.run/counter".to_string(),
                actor_type_identity: "actors.Counter".to_string(),
                actor_id_type_identity: "builtin:string".to_string(),
                actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                actor_id_hash: format!("sha256:{}", hex::encode(Sha256::digest(&id_bytes))),
                canonical_actor_id_key_bytes: id_bytes,
            },
            epoch: 1,
        },
        actor_abi_identity: abi(),
        actor_implementation_identity: implementation(),
        declaration_owner: owner(),
    }
}

fn interpreter_for(file: Arc<LinkedFileUnit>) -> (Interpreter, Arc<EvalRuntimeProgram>) {
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        },
        TypeDeclIr {
            name: "Payload".to_string(),
            descriptor: payload_descriptor(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        },
    );
    let program = Arc::new(EvalRuntimeProgram::new(
        "skiff.run/counter",
        vec![file],
        Vec::new(),
        PublicationResourceTable::default(),
        Default::default(),
        LinkOverlay::default(),
        types,
    ));
    (
        Interpreter::with_program(Arc::clone(&program), test_runtime::runtime_factory()),
        program,
    )
}

fn fixture_from_file(
    file: Arc<LinkedFileUnit>,
    initialize: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap),
) -> Fixture {
    fixture_from_file_with_admission(file, initialize, true)
}

fn fixture_from_file_with_admission(
    file: Arc<LinkedFileUnit>,
    initialize: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap),
    admitted: bool,
) -> Fixture {
    let (interpreter, program) = interpreter_for(file);
    let store = ActorInstanceStore::new();
    let id_bytes = br#""counter-1""#.to_vec();
    let fence = ActorInstanceFence {
        incarnation: ActorIncarnationKey {
            logical_key: ActorLogicalKey {
                service_id: "skiff.run/counter".to_string(),
                actor_type_identity: "actors.Counter".to_string(),
                actor_id_type_identity: "builtin:string".to_string(),
                actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                actor_id_hash: format!("sha256:{}", hex::encode(Sha256::digest(&id_bytes))),
                canonical_actor_id_key_bytes: id_bytes,
            },
            epoch: 1,
        },
        actor_abi_identity: abi(),
        actor_implementation_identity: implementation(),
        declaration_owner: owner(),
    };
    let payload = br#"[]"#;
    let handle = store
        .activate(ActorActivationRequest {
            fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: payload,
            program: program.projection().type_view(),
        })
        .unwrap();
    let authority = ActorExecutorAuthority::new();
    store
        .with_fields_for_executor(&authority, &handle, initialize)
        .unwrap();
    if admitted {
        store
            .mark_admitted(&authority, &handle)
            .expect("fixture instance must be admitted");
    }
    Fixture {
        interpreter,
        store,
        handle,
        method: method_identity(),
    }
}

fn fixture_with_admission(
    return_type: LinkedTypeRef,
    may_suspend: bool,
    admitted: bool,
) -> Fixture {
    fixture_from_file_with_admission(
        actor_file(return_type, may_suspend),
        |fields, _| {
            fields[1].value = RuntimeValue::Number(1.0);
            fields[1].assigned = true;
        },
        admitted,
    )
}

fn fixture(return_type: LinkedTypeRef, may_suspend: bool) -> Fixture {
    fixture_with_admission(return_type, may_suspend, true)
}

fn activation_fixture() -> Fixture {
    fixture_from_file_with_admission(
        actor_file(integer(), true),
        |fields, _| {
            fields[1].value = RuntimeValue::Number(1.0);
            fields[1].assigned = true;
        },
        false,
    )
}

fn heap_field_actor_file() -> Arc<LinkedFileUnit> {
    let item_array = array(integer());
    let mut file = (*actor_file(item_array.clone(), false)).clone();
    let declaration = &mut file.actor_declarations[0];
    declaration.fields[1].name = "items".to_string();
    declaration.fields[1].ty = item_array.clone();
    declaration.public_methods[0].parameters[0].ty = item_array.clone();

    let executable = &mut file.executables[0];
    executable.params[0].ty = item_array.clone();
    executable.body.statements[0] = LinkedStmtIr::Assign {
        target: AssignTargetIr::ActorSelfField {
            field: "items".to_string(),
            field_type: item_array.clone(),
        },
        value: ExprRefIr { expression: 0 },
    };
    executable.body.expressions[1] = LinkedExprIr::ActorSelfField {
        field: "items".to_string(),
        field_type: item_array,
    };
    Arc::new(file)
}

fn heap_field_fixture() -> Fixture {
    fixture_from_file(heap_field_actor_file(), |fields, heap| {
        let items = heap
            .alloc_array(vec![RuntimeValue::Number(1.0)])
            .expect("initial heap-backed Actor field should allocate");
        fields[1].value = RuntimeValue::Heap(items);
        fields[1].assigned = true;
    })
}

fn stream_field_fixture() -> Fixture {
    let mut file = (*actor_file(integer(), true)).clone();
    file.actor_declarations[0].fields[2].name = "stream".to_string();
    file.actor_declarations[0].fields[2].ty = array(LinkedTypeRef::Nullable {
        inner: Box::new(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![string()],
        }),
    });
    fixture_from_file(Arc::new(file), |fields, _| {
        fields[1].value = RuntimeValue::Number(1.0);
        fields[1].assigned = true;
    })
}

fn context_with_execution(
    interpreter: &Interpreter,
    execution: crate::capabilities::ExecutionControl<'static>,
) -> ProgramExecutionContext<'static> {
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
    let request = test_runtime::request_context();
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
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

fn context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    context_with_execution(interpreter, test_runtime::execution_control())
}

fn create_activation_fixture(
    actor_id: &str,
) -> (
    Arc<Interpreter>,
    ActorInstanceStore,
    ActorInstanceFence,
    String,
) {
    let (interpreter, _) = interpreter_for(actor_file_with_create());
    let fence = counter_fence(actor_id);
    let actor_id_hash = fence.incarnation.logical_key.actor_id_hash.clone();
    (
        Arc::new(interpreter),
        ActorInstanceStore::new(),
        fence,
        actor_id_hash,
    )
}

async fn retry_create(
    interpreter: &Interpreter,
    store: &ActorInstanceStore,
    fence: ActorInstanceFence,
) {
    ActorMethodExecutor::new(store)
        .activate(
            interpreter,
            &context(interpreter),
            fence,
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("same fence must rematerialize after partial create cleanup");
}

async fn assert_no_create_activation_retry_after_scope_terminal(
    actor_id: &str,
    execution: crate::capabilities::ExecutionControl<'static>,
    assert_error: impl FnOnce(&ActorMethodExecutorError),
) {
    const SESSION_ID: &str = "router-session-no-create-terminal";

    let (interpreter, _) = interpreter_for(actor_file(integer(), false));
    let store = Arc::new(ActorInstanceStore::new());
    let tracker = Arc::new(ActorInstanceSessionTracker::new(Arc::clone(&store)));
    tracker.open_session(SESSION_ID).unwrap();
    let session = tracker.session_lease(SESSION_ID).unwrap();
    let fence = counter_fence(actor_id);

    let error = ActorMethodExecutor::new(store.as_ref())
        .activate_for_session(
            &tracker,
            &session,
            &interpreter,
            &context_with_execution(&interpreter, execution),
            fence.clone(),
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect_err("a terminal execution scope must reject fresh Actor admission");
    assert_error(&error);
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);

    let admitted = ActorMethodExecutor::new(store.as_ref())
        .activate_for_session(
            &tracker,
            &session,
            &interpreter,
            &context(&interpreter),
            fence,
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("the same fence must retry after exact provisional cleanup");
    store
        .await_admission(&admitted)
        .await
        .expect("retry must atomically admit the replacement materialization");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    assert_eq!(tracker.discard_session(SESSION_ID), 1);
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);
}

#[tokio::test]
async fn no_create_fresh_materialization_rejects_pre_cancelled_scope_and_retries() {
    let execution = test_runtime::execution_control();
    execution.cancellation_token().cancel();

    assert_no_create_activation_retry_after_scope_terminal(
        "no-create-pre-cancelled",
        execution,
        |error| {
            assert!(matches!(
                error,
                ActorMethodExecutorError::Execution(RuntimeError::Cancelled)
            ));
        },
    )
    .await;
}

#[tokio::test]
async fn no_create_fresh_materialization_rejects_expired_scope_and_retries() {
    let execution = test_runtime::execution_control_with_deadline(Some(
        std::time::Instant::now()
            .checked_sub(Duration::from_secs(1))
            .expect("one second before now is representable"),
    ));

    assert_no_create_activation_retry_after_scope_terminal(
        "no-create-expired",
        execution,
        |error| {
            assert!(matches!(
                error,
                ActorMethodExecutorError::Execution(RuntimeError::ScopeTerminal(terminal))
                    if matches!(
                        terminal.terminal(),
                        skiff_runtime_capability_context::ExecutionScopeTerminal::InheritedDeadlineExceeded(_)
                    )
            ));
        },
    )
    .await;
}

#[tokio::test]
async fn pending_create_future_drop_exact_discards_and_allows_retry() {
    let (interpreter, store, fence, hash) = create_activation_fixture("drop-create");
    let gate = install_actor_create_test_gate(hash, false);
    let executor = ActorMethodExecutor::new(&store);
    let create_context = context(&interpreter);
    let mut activation = Box::pin(executor.activate(
        &interpreter,
        &create_context,
        fence.clone(),
        ACTOR_BOOTSTRAP_ENCODING_V1,
        b"[]",
    ));
    tokio::select! {
        result = &mut activation => panic!("create completed before gate: {result:?}"),
        _ = gate.wait_entered() => {}
    }
    assert_eq!(store.len(), 1);
    drop(activation);
    assert!(store.is_empty());
    retry_create(&interpreter, &store, fence).await;
}

#[tokio::test]
async fn pending_create_scope_cancel_and_deadline_exact_discard() {
    for actor_id in ["cancel-create", "deadline-create"] {
        let (interpreter, store, fence, hash) = create_activation_fixture(actor_id);
        let gate = install_actor_create_test_gate(hash, false);
        let deadline = (actor_id == "deadline-create")
            .then(|| std::time::Instant::now() + Duration::from_millis(100));
        let execution = test_runtime::execution_control_with_deadline(deadline);
        let cancellation = execution.cancellation_token();
        let executor = ActorMethodExecutor::new(&store);
        let create_context = context_with_execution(&interpreter, execution);
        let mut activation = Box::pin(executor.activate(
            &interpreter,
            &create_context,
            fence.clone(),
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        ));
        tokio::time::timeout(Duration::from_secs(1), async {
            tokio::select! {
                result = &mut activation => panic!("create completed before gate: {result:?}"),
                _ = gate.wait_entered() => {}
            }
        })
        .await
        .expect("create must reach the deterministic gate before its deadline");
        if deadline.is_none() {
            cancellation.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(1), activation)
            .await
            .expect("scope terminal must settle pending create")
            .unwrap_err();
        assert!(matches!(error, ActorMethodExecutorError::Execution(_)));
        assert!(store.is_empty());
        retry_create(&interpreter, &store, fence).await;
    }
}

#[tokio::test]
async fn pending_create_task_abort_and_panic_exact_discard() {
    for (actor_id, panic_after_enter) in [("abort-create", false), ("panic-create", true)] {
        let (interpreter, store, fence, hash) = create_activation_fixture(actor_id);
        let gate = install_actor_create_test_gate(hash, panic_after_enter);
        let task_store = store.clone();
        let task_interpreter = Arc::clone(&interpreter);
        let task_fence = fence.clone();
        let task = tokio::spawn(async move {
            ActorMethodExecutor::new(&task_store)
                .activate(
                    &task_interpreter,
                    &context(&task_interpreter),
                    task_fence,
                    ACTOR_BOOTSTRAP_ENCODING_V1,
                    b"[]",
                )
                .await
        });
        gate.wait_entered().await;
        if !panic_after_enter {
            task.abort();
        }
        let join = task
            .await
            .expect_err("abort/panic must terminate create task");
        assert_eq!(join.is_panic(), panic_after_enter);
        assert!(store.is_empty());
        retry_create(&interpreter, &store, fence).await;
    }
}

#[tokio::test]
async fn follower_never_adopts_half_created_instance_and_observes_exact_outcome() {
    let (interpreter, store, fence, hash) = create_activation_fixture("follower-create");
    let gate = install_actor_create_test_gate(hash, false);
    let leader_executor = ActorMethodExecutor::new(&store);
    let leader_context = context(&interpreter);
    let mut leader = Box::pin(leader_executor.activate(
        &interpreter,
        &leader_context,
        fence.clone(),
        ACTOR_BOOTSTRAP_ENCODING_V1,
        b"[]",
    ));
    tokio::select! {
        result = &mut leader => panic!("leader completed before create gate: {result:?}"),
        _ = gate.wait_entered() => {}
    }

    let follower_executor = ActorMethodExecutor::new(&store);
    let follower_context = context(&interpreter);
    let mut follower = Box::pin(follower_executor.activate(
        &interpreter,
        &follower_context,
        fence.clone(),
        ACTOR_BOOTSTRAP_ENCODING_V1,
        b"[]",
    ));
    tokio::select! {
        result = &mut follower => panic!("follower adopted a half-created instance: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(10)) => {}
    }
    drop(leader);
    let follower_error = tokio::time::timeout(Duration::from_secs(1), follower)
        .await
        .expect("exact discard must wake the admission follower")
        .unwrap_err();
    assert!(matches!(
        follower_error,
        ActorMethodExecutorError::Store(
            ActorInstanceStoreError::InstanceNotFound | ActorInstanceStoreError::InstanceReplaced
        )
    ));
    assert!(store.is_empty());

    let admitted = ActorMethodExecutor::new(&store)
        .activate(
            &interpreter,
            &context(&interpreter),
            fence.clone(),
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("retry rematerializes after leader drop");
    let reused = ActorMethodExecutor::new(&store)
        .activate(
            &interpreter,
            &context(&interpreter),
            fence,
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("post-admission follower reuses exact live instance");
    assert!(admitted.same_instance(&reused));
}

#[tokio::test]
async fn session_close_terminates_pending_create_owner_scope_and_allows_retry() {
    const SESSION_ID: &str = "router-session-pending-create";

    let (interpreter, raw_store, fence, hash) =
        create_activation_fixture("session-close-pending-create");
    let store = Arc::new(raw_store);
    let tracker = Arc::new(ActorInstanceSessionTracker::new(Arc::clone(&store)));
    tracker.open_session(SESSION_ID).unwrap();
    let session = tracker.session_lease(SESSION_ID).unwrap();
    let gate = install_actor_create_test_gate(hash, false);
    let task_store = Arc::clone(&store);
    let task_tracker = Arc::clone(&tracker);
    let task_interpreter = Arc::clone(&interpreter);
    let task_fence = fence.clone();
    let activation = tokio::spawn(async move {
        let context = context(&task_interpreter);
        let executor = ActorMethodExecutor::new(task_store.as_ref());
        tokio::select! {
            biased;
            _ = session.wait_closed() => None,
            result = executor.activate_for_session(
                &task_tracker,
                &session,
                &task_interpreter,
                &context,
                task_fence,
                ACTOR_BOOTSTRAP_ENCODING_V1,
                b"[]",
            ) => Some(result),
        }
    });
    tokio::time::timeout(Duration::from_secs(1), gate.wait_entered())
        .await
        .expect("session activation must reach the real create gate");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    assert_eq!(tracker.discard_session(SESSION_ID), 1);
    let terminal = tokio::time::timeout(Duration::from_secs(1), activation)
        .await
        .expect("session close must terminate the pending owner activation scope")
        .expect("owner activation task must not panic");
    assert!(terminal.is_none());
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);

    tracker.open_session(SESSION_ID).unwrap();
    let new_session = tracker.session_lease(SESSION_ID).unwrap();
    let new_handle = ActorMethodExecutor::new(store.as_ref())
        .activate_for_session(
            &tracker,
            &new_session,
            &interpreter,
            &context(&interpreter),
            fence,
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("same fence must rematerialize and admit after session-close cancellation");
    store
        .await_admission(&new_handle)
        .await
        .expect("replacement materialization must be admitted");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    assert_eq!(tracker.discard_session(SESSION_ID), 1);
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);
}

#[tokio::test]
async fn session_close_during_create_fences_delayed_guard_from_new_materialization() {
    const SESSION_ID: &str = "router-session-create-fence";

    let (interpreter, raw_store, fence, hash) = create_activation_fixture("session-close-create");
    let store = Arc::new(raw_store);
    let tracker = Arc::new(ActorInstanceSessionTracker::new(Arc::clone(&store)));
    tracker.open_session(SESSION_ID).unwrap();
    let old_session = tracker.session_lease(SESSION_ID).unwrap();
    let old_gate = install_actor_create_test_gate(hash, false);
    let executor = ActorMethodExecutor::new(store.as_ref());
    let old_context = context(&interpreter);
    let mut old_activation = Box::pin(executor.activate_for_session(
        &tracker,
        &old_session,
        &interpreter,
        &old_context,
        fence.clone(),
        ACTOR_BOOTSTRAP_ENCODING_V1,
        b"[]",
    ));
    tokio::time::timeout(Duration::from_secs(1), async {
        tokio::select! {
            result = &mut old_activation => panic!("old create completed before gate: {result:?}"),
            _ = old_gate.wait_entered() => {}
        }
    })
    .await
    .expect("old session activation must reach the real create gate");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    assert_eq!(tracker.discard_session(SESSION_ID), 1);
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);

    tracker.open_session(SESSION_ID).unwrap();
    let new_session = tracker.session_lease(SESSION_ID).unwrap();
    let new_handle = ActorMethodExecutor::new(store.as_ref())
        .activate_for_session(
            &tracker,
            &new_session,
            &interpreter,
            &context(&interpreter),
            fence,
            ACTOR_BOOTSTRAP_ENCODING_V1,
            b"[]",
        )
        .await
        .expect("same fence must rematerialize and admit for the new session generation");
    store
        .await_admission(&new_handle)
        .await
        .expect("new exact materialization must be admitted");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    old_gate.release();
    let old_error = tokio::time::timeout(Duration::from_secs(1), old_activation)
        .await
        .expect("closed-session activation must terminate after create resumes")
        .unwrap_err();
    assert!(
        matches!(
            &old_error,
            ActorMethodExecutorError::Execution(RuntimeError::ActorInstance(
                ActorInstanceStoreError::InstanceReplaced
            ))
        ),
        "unexpected closed-session activation error: {old_error:?}"
    );
    store
        .await_admission(&new_handle)
        .await
        .expect("delayed old guard cleanup must not remove the new Arc");
    assert_eq!(store.len(), 1);
    assert_eq!(tracker.tracked_owner_count_for_test(), 1);

    assert_eq!(tracker.discard_session(SESSION_ID), 1);
    assert!(store.is_empty());
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);
}

async fn execute(
    fixture: &Fixture,
    method: &ActorMethodIdentity,
    payload: &[u8],
) -> Result<Vec<u8>, ActorMethodExecutorError> {
    ActorMethodExecutor::new(&fixture.store)
        .execute(
            &fixture.interpreter,
            ActorMethodExecutionRequest {
                instance: &fixture.handle,
                method_identity: method,
                arguments_payload: payload,
                context: context(&fixture.interpreter),
            },
        )
        .await
}

async fn execution_frame(fixture: &Fixture) -> (ActorExecutionFrame, RequestHeap) {
    execution_frame_with_activation(fixture, false).await
}

async fn execution_frame_with_activation(
    fixture: &Fixture,
    activation: bool,
) -> (ActorExecutionFrame, RequestHeap) {
    let authority = ActorExecutorAuthority::new();
    let mut lease = if activation {
        fixture
            .store
            .acquire_execution_for_activation(&authority, &fixture.handle)
            .await
            .unwrap()
    } else {
        fixture
            .store
            .acquire_execution(&authority, &fixture.handle)
            .await
            .unwrap()
    };
    let heap = lease.take_heap();
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    let declaration = resolve_actor_declaration(
        program.type_view(),
        &fixture.handle.fence().declaration_owner,
    )
    .unwrap();
    let field_plans = actor_field_plans(declaration, program.type_view(), &addr).unwrap();
    (
        ActorExecutionFrame::new(
            fixture.store.clone(),
            fixture.handle.clone(),
            lease,
            field_plans,
            activation,
        ),
        heap,
    )
}

fn stored_heap_len(fixture: &Fixture) -> usize {
    fixture
        .store
        .with_fields_for_executor(
            &ActorExecutorAuthority::new(),
            &fixture.handle,
            |_fields, heap| heap.len(),
        )
        .unwrap()
}

async fn force_pending_cut(
    frame: &ActorExecutionFrame,
    heap: &mut RequestHeap,
    execution: &crate::capabilities::ExecutionControl<'_>,
) {
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    let waiting = frame.await_if_pending(heap, execution, receiver);
    let wake = async {
        tokio::task::yield_now().await;
        sender.send(()).expect("pending cut receiver stays live");
    };
    let (outcome, ()) = tokio::join!(waiting, wake);
    outcome
        .expect("pending Actor continuation should resume")
        .expect("pending cut sender should produce a value");
}

#[tokio::test]
async fn real_linked_actor_method_commits_self_field_across_calls() {
    let fixture = fixture(integer(), false);
    assert_eq!(
        execute(&fixture, &fixture.method, b"[8]").await.unwrap(),
        b"8"
    );
    assert_eq!(
        execute(&fixture, &fixture.method, b"[13]").await.unwrap(),
        b"13"
    );
}

#[tokio::test]
async fn scheduler_queue_observes_request_cancellation() {
    let fixture = fixture(integer(), false);
    let authority = ActorExecutorAuthority::new();
    let scheduler_owner = fixture
        .store
        .acquire_execution(&authority, &fixture.handle)
        .await
        .unwrap();
    let execution = test_runtime::execution_control();
    let cancellation = execution.cancellation_token();
    let executor = ActorMethodExecutor::new(&fixture.store);
    let mut waiting = Box::pin(executor.execute(
        &fixture.interpreter,
        ActorMethodExecutionRequest {
            instance: &fixture.handle,
            method_identity: &fixture.method,
            arguments_payload: b"[8]",
            context: context_with_execution(&fixture.interpreter, execution),
        },
    ));
    tokio::select! {
        biased;
        result = &mut waiting => panic!("queued invocation completed before cancellation: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(10)) => {}
    }

    cancellation.cancel();
    let error = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("cancellation must wake the scheduler queue")
        .unwrap_err();
    assert!(matches!(
        error,
        ActorMethodExecutorError::Execution(error) if error.is_cancelled()
    ));
    drop(scheduler_owner);
}

#[tokio::test]
async fn admission_wait_observes_effective_deadline() {
    let fixture = fixture_with_admission(integer(), false, false);
    let execution = test_runtime::execution_control_with_deadline(Some(
        std::time::Instant::now() + Duration::from_millis(20),
    ));

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        ActorMethodExecutor::new(&fixture.store).execute(
            &fixture.interpreter,
            ActorMethodExecutionRequest {
                instance: &fixture.handle,
                method_identity: &fixture.method,
                arguments_payload: b"[8]",
                context: context_with_execution(&fixture.interpreter, execution),
            },
        ),
    )
    .await
    .expect("effective deadline must wake the admission wait")
    .unwrap_err();
    assert!(matches!(
        error,
        ActorMethodExecutorError::Execution(RuntimeError::ScopeTerminal(_))
    ));
}

#[tokio::test]
async fn method_and_argument_errors_fail_closed_without_changing_live_field() {
    let fixture = fixture(integer(), false);
    let missing = ActorMethodIdentity::new("skiff-actor-method-v1:sha256:missing");
    assert!(matches!(
        execute(&fixture, &missing, b"[2]").await,
        Err(ActorMethodExecutorError::MethodMissing)
    ));
    assert!(matches!(
        execute(&fixture, &fixture.method, b"[]").await,
        Err(ActorMethodExecutorError::ArgumentCount { .. })
    ));
    assert!(matches!(
        execute(&fixture, &fixture.method, br#"["bad"]"#).await,
        Err(ActorMethodExecutorError::ArgumentDecode { .. })
    ));
    assert_eq!(
        execute(&fixture, &fixture.method, b"[3]").await.unwrap(),
        b"3"
    );
}

#[tokio::test]
async fn wrong_return_type_rolls_back_and_may_suspend_method_executes() {
    let wrong_return = fixture(
        LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        },
        false,
    );
    assert!(matches!(
        execute(&wrong_return, &wrong_return.method, b"[9]").await,
        Err(ActorMethodExecutorError::ReturnEncode(_))
    ));
    let suspended = fixture(integer(), true);
    assert_eq!(
        execute(&suspended, &suspended.method, b"[9]")
            .await
            .unwrap(),
        b"9"
    );
    assert_eq!(
        execute(&suspended, &suspended.method, b"[10]")
            .await
            .unwrap(),
        b"10"
    );
}

#[tokio::test]
async fn suspended_segment_commits_releases_and_resumes_with_latest_field() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let local_handle = heap
        .alloc_array(vec![RuntimeValue::String("continuation-local".to_string())])
        .unwrap();
    for index in 0..512 {
        heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "dead-local".to_string(),
            RuntimeValue::Number(index as f64),
        )])))
        .unwrap();
    }
    let program = fixture.interpreter.program_projection().unwrap();
    frame
        .write_field(
            "count",
            &integer(),
            program.type_view(),
            &ExecutableAddr {
                unit: UnitAddr::Service,
                file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
                executable: 0,
            },
            &RuntimeValue::Number(5.0),
            &mut heap,
        )
        .unwrap();

    frame.suspend(&heap).unwrap();
    assert_eq!(
        stored_heap_len(&fixture),
        0,
        "suspension must persist only primitive Actor fields, not invocation locals"
    );
    assert!(frame.suspension.lease.lock().unwrap().is_none());
    assert!(frame.read_field("count").is_err());

    assert_eq!(
        execute(&fixture, &fixture.method, b"[9]").await.unwrap(),
        b"9"
    );
    let execution = context(&fixture.interpreter).execution();
    frame.resume(&mut heap, &execution).await.unwrap();
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(9.0)
    );
    assert!(matches!(
        heap.get(local_handle).unwrap(),
        skiff_runtime_model::runtime_value::HeapNode::Array(items)
            if items == &[RuntimeValue::String("continuation-local".to_string())]
    ));
    frame.finish(heap).unwrap();
}

#[tokio::test]
async fn repeated_actor_invocations_do_not_accumulate_dead_local_heap_nodes() {
    let fixture = fixture(integer(), true);

    for round in 0..8 {
        let (frame, mut heap) = execution_frame(&fixture).await;
        assert_eq!(
            heap.len(),
            0,
            "invocation {round} inherited dead nodes from an earlier method"
        );
        for index in 0..256 {
            heap.alloc_array(vec![
                RuntimeValue::Number(round as f64),
                RuntimeValue::Number(index as f64),
            ])
            .unwrap();
        }
        frame.finish(heap).unwrap();
        assert_eq!(
            stored_heap_len(&fixture),
            0,
            "invocation {round} committed non-field local nodes"
        );
    }
}

#[tokio::test]
async fn primitive_actor_does_not_retain_temporary_heaps_across_pending_calls() {
    const ROUNDS: usize = 5;
    const PENDING_CUTS_PER_ROUND: usize = 3;
    const TEMP_STRUCTURES_PER_CUT: usize = 128;

    let fixture = fixture(integer(), true);
    let baseline = stored_heap_len(&fixture);
    let mut persistent_counts = Vec::with_capacity(ROUNDS);

    for round in 0..ROUNDS {
        let (frame, mut heap) = execution_frame(&fixture).await;
        let execution = context(&fixture.interpreter).execution();
        let continuation_local = heap
            .alloc_array(vec![RuntimeValue::String(format!("continuation-{round}"))])
            .expect("continuation local should allocate");
        for cut in 0..PENDING_CUTS_PER_ROUND {
            for item in 0..TEMP_STRUCTURES_PER_CUT {
                let leaf = heap
                    .alloc_array(vec![
                        RuntimeValue::Number(round as f64),
                        RuntimeValue::Number(cut as f64),
                        RuntimeValue::Number(item as f64),
                    ])
                    .expect("temporary leaf should allocate");
                let wrapper = heap
                    .alloc_object(RuntimeObject::unshaped(BTreeMap::from([
                        ("leaf".to_string(), RuntimeValue::Heap(leaf)),
                        (
                            "label".to_string(),
                            RuntimeValue::String(format!("{round}:{cut}:{item}")),
                        ),
                    ])))
                    .expect("temporary wrapper should allocate");
                heap.alloc_array(vec![RuntimeValue::Heap(wrapper)])
                    .expect("temporary root should allocate");
            }
            force_pending_cut(&frame, &mut heap, &execution).await;
            assert_eq!(
                heap.get(continuation_local).unwrap(),
                &HeapNode::Array(vec![RuntimeValue::String(format!("continuation-{round}"))]),
                "real Pending cut must keep the continuation heap and handles intact"
            );
        }
        frame.finish(heap).expect("Actor call should finish");
        persistent_counts.push(stored_heap_len(&fixture));
    }

    assert!(
        persistent_counts.iter().all(|count| *count == baseline),
        "primitive-only Actor retained request-local heaps: baseline={baseline}, persistent_counts={persistent_counts:?}"
    );
}

#[tokio::test]
async fn heap_backed_actor_field_survives_compacted_heap_across_calls() {
    let fixture = heap_field_fixture();
    let item_array = array(integer());
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };

    let (first_frame, mut first_heap) = execution_frame(&fixture).await;
    let replacement = first_heap
        .alloc_array(vec![
            RuntimeValue::Number(2.0),
            RuntimeValue::Number(3.0),
            RuntimeValue::Number(5.0),
        ])
        .expect("replacement Actor field should allocate");
    first_frame
        .write_field(
            "items",
            &item_array,
            program.type_view(),
            &addr,
            &RuntimeValue::Heap(replacement),
            &mut first_heap,
        )
        .expect("heap-backed Actor field should be writable");
    first_frame
        .finish(first_heap)
        .expect("first Actor call should commit");

    let (second_frame, second_heap) = execution_frame(&fixture).await;
    let RuntimeValue::Heap(items) = second_frame
        .read_field("items")
        .expect("heap-backed Actor field should survive into the next call")
    else {
        panic!("heap-backed Actor field must remain a heap value")
    };
    assert_eq!(
        second_heap.get(items).unwrap(),
        &HeapNode::Array(vec![
            RuntimeValue::Number(2.0),
            RuntimeValue::Number(3.0),
            RuntimeValue::Number(5.0),
        ])
    );
    second_frame
        .finish(second_heap)
        .expect("second Actor call should commit");
}

#[tokio::test]
async fn nominal_nested_collection_actor_field_survives_compacted_invocations() {
    const ITEM_COUNT: usize = 32;
    const PERSISTENT_GRAPH_NODES: usize = 2 + ITEM_COUNT * 2;
    const PENDING_CUTS: usize = 5;

    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let mut item_values = Vec::with_capacity(ITEM_COUNT);
    for index in 0..ITEM_COUNT {
        let tags = heap
            .alloc_array(vec![
                RuntimeValue::from("runtime"),
                RuntimeValue::from(format!("actor-{index}")),
            ])
            .unwrap();
        let item = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                (
                    "name".to_string(),
                    RuntimeValue::from(if index == 0 {
                        "first".to_string()
                    } else {
                        format!("item-{index}")
                    }),
                ),
                ("tags".to_string(), RuntimeValue::Heap(tags)),
            ])))
            .unwrap();
        item_values.push(RuntimeValue::Heap(item));
    }
    let items = heap.alloc_array(item_values).unwrap();
    let payload = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "items".to_string(),
            RuntimeValue::Heap(items),
        )])))
        .unwrap();
    let program = fixture.interpreter.program_projection().unwrap();
    frame
        .write_field(
            "payload",
            &payload_type(),
            program.type_view(),
            &ExecutableAddr {
                unit: UnitAddr::Service,
                file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
                executable: 0,
            },
            &RuntimeValue::Heap(payload),
            &mut heap,
        )
        .unwrap();
    let payload_value = frame.read_field("payload").unwrap();
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let alias = fields
        .iter_mut()
        .find(|field| field.name == "payload_alias")
        .unwrap();
    alias.value = payload_value;
    alias.assigned = true;
    drop(fields);
    for index in 0..128 {
        heap.alloc_array(vec![RuntimeValue::Number(index as f64)])
            .unwrap();
    }
    let continuation_local = heap
        .alloc_array(vec![RuntimeValue::from("continuation-local")])
        .unwrap();
    let execution = context(&fixture.interpreter).execution();
    let continuation_heap_before_pending = heap.len();
    for cut in 1..=PENDING_CUTS {
        force_pending_cut(&frame, &mut heap, &execution).await;
        assert_eq!(
            heap.len(),
            continuation_heap_before_pending + PERSISTENT_GRAPH_NODES * cut,
            "each real resume currently imports one fresh copy of the reachable Actor field graph"
        );
        assert_eq!(
            stored_heap_len(&fixture),
            PERSISTENT_GRAPH_NODES,
            "the persistent Actor snapshot itself must remain compact"
        );
    }
    let resumed_payload = frame.read_field("payload").unwrap();
    assert_eq!(
        frame.read_field("payload_alias").unwrap(),
        resumed_payload,
        "suspend/resume must import all field roots with one shared clone context"
    );
    assert_eq!(
        heap.get(continuation_local).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("continuation-local")])
    );
    frame.finish(heap).unwrap();
    assert_eq!(
        stored_heap_len(&fixture),
        PERSISTENT_GRAPH_NODES,
        "only the nominal payload graph is persistent"
    );

    for _ in 0..3 {
        let (frame, heap) = execution_frame(&fixture).await;
        let RuntimeValue::Heap(payload) = frame.read_field("payload").unwrap() else {
            panic!("payload field must remain heap-backed");
        };
        assert_eq!(
            frame.read_field("payload_alias").unwrap(),
            RuntimeValue::Heap(payload),
            "multi-root compaction must preserve aliases between Actor fields"
        );
        let HeapNode::Object(payload) = heap.get(payload).unwrap() else {
            panic!("payload must remain a nominal record object");
        };
        let items = payload.fields()["items"].as_heap_handle().unwrap();
        let HeapNode::Array(items) = heap.get(items).unwrap() else {
            panic!("payload.items must remain an array");
        };
        let item = items[0].as_heap_handle().unwrap();
        let HeapNode::Object(item) = heap.get(item).unwrap() else {
            panic!("payload item must remain a record object");
        };
        assert_eq!(item.fields()["name"], RuntimeValue::from("first"));
        let tags = item.fields()["tags"].as_heap_handle().unwrap();
        assert_eq!(
            heap.get(tags).unwrap(),
            &HeapNode::Array(vec![
                RuntimeValue::from("runtime"),
                RuntimeValue::from("actor-0")
            ])
        );
        frame.finish(heap).unwrap();
        assert_eq!(stored_heap_len(&fixture), PERSISTENT_GRAPH_NODES);
    }
}

#[tokio::test]
async fn failed_heap_field_compaction_does_not_publish_partial_actor_state() {
    let fixture = fixture(integer(), true);
    let (frame, heap) = execution_frame(&fixture).await;
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let count = fields
        .iter_mut()
        .find(|field| field.name == "count")
        .unwrap();
    count.value = RuntimeValue::Number(99.0);
    let payload = fields
        .iter_mut()
        .find(|field| field.name == "payload")
        .unwrap();
    payload.value = RuntimeValue::Heap(skiff_runtime_model::runtime_value::HeapHandle::new(
        u32::MAX,
        0,
    ));
    payload.assigned = true;
    drop(fields);

    let error = frame.finish(heap).unwrap_err();
    assert!(error.to_string().contains("heap handle"));
    assert!(!frame.has_execution_lease());
    assert!(frame.read_field("count").is_err());
    fixture
        .store
        .with_fields_for_executor(
            &ActorExecutorAuthority::new(),
            &fixture.handle,
            |fields, stored_heap| {
                assert_eq!(fields[1].value, RuntimeValue::Number(1.0));
                assert_eq!(stored_heap.len(), 0);
            },
        )
        .unwrap();
    let authority = ActorExecutorAuthority::new();
    let competing = fixture.store.acquire_execution(&authority, &fixture.handle);
    tokio::time::timeout(std::time::Duration::from_secs(1), competing)
        .await
        .expect("failed finish must release the Actor scheduler")
        .unwrap();
}

#[tokio::test]
async fn deeply_nested_actor_field_fails_at_heap_depth_limit_without_recursion() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let mut value = RuntimeValue::from("leaf");
    for _ in 0..=heap.limits().max_clone_depth {
        value = RuntimeValue::Heap(heap.alloc_array(vec![value]).unwrap());
    }
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let payload = fields
        .iter_mut()
        .find(|field| field.name == "payload")
        .unwrap();
    payload.value = value;
    payload.assigned = true;
    drop(fields);

    let error = frame.finish(heap).unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::ResourceLimitExceeded { reason, .. }
            if reason == "max persistent Actor graph depth"
    ));
    assert_eq!(stored_heap_len(&fixture), 0);
}

#[tokio::test]
async fn partial_create_unassigned_heap_value_is_not_persisted_and_can_resume_assignment() {
    let fixture = activation_fixture();
    let (frame, mut heap) = execution_frame_with_activation(&fixture, true).await;
    let dead = heap
        .alloc_array(vec![RuntimeValue::from("unassigned-local")])
        .unwrap();
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let payload = fields
        .iter_mut()
        .find(|field| field.name == "payload")
        .unwrap();
    assert!(!payload.assigned);
    payload.value = RuntimeValue::Heap(dead);
    drop(fields);

    frame.suspend(&heap).unwrap();
    fixture
        .store
        .with_fields_for_executor(
            &ActorExecutorAuthority::new(),
            &fixture.handle,
            |fields, stored_heap| {
                let payload = fields.iter().find(|field| field.name == "payload").unwrap();
                assert!(!payload.assigned);
                assert_eq!(payload.value, RuntimeValue::Null);
                assert_eq!(stored_heap.len(), 0);
            },
        )
        .unwrap();

    let execution = context(&fixture.interpreter).execution();
    frame.resume(&mut heap, &execution).await.unwrap();
    assert!(frame.read_field("payload").is_err());
    assert_eq!(
        heap.get(dead).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("unassigned-local")])
    );
    let items = heap.alloc_array(Vec::new()).unwrap();
    let payload = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "items".to_string(),
            RuntimeValue::Heap(items),
        )])))
        .unwrap();
    let program = fixture.interpreter.program_projection().unwrap();
    frame
        .write_field(
            "payload",
            &payload_type(),
            program.type_view(),
            &ExecutableAddr {
                unit: UnitAddr::Service,
                file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
                executable: 0,
            },
            &RuntimeValue::Heap(payload),
            &mut heap,
        )
        .unwrap();
    frame.finish(heap).unwrap();
    assert_eq!(stored_heap_len(&fixture), 2);
}

#[tokio::test]
async fn resume_limit_failure_rolls_back_continuation_heap_and_releases_scheduler() {
    let fixture = fixture(integer(), true);
    let (writer, mut writer_heap) = execution_frame(&fixture).await;
    let first_items = writer_heap.alloc_array(Vec::new()).unwrap();
    let first = writer_heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "items".to_string(),
            RuntimeValue::Heap(first_items),
        )])))
        .unwrap();
    let second_items = writer_heap.alloc_array(Vec::new()).unwrap();
    let second = writer_heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "items".to_string(),
            RuntimeValue::Heap(second_items),
        )])))
        .unwrap();
    let fields = writer
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    for (name, value) in [
        ("payload", RuntimeValue::Heap(first)),
        ("payload_alias", RuntimeValue::Heap(second)),
    ] {
        let field = fields.iter_mut().find(|field| field.name == name).unwrap();
        field.value = value;
        field.assigned = true;
    }
    drop(fields);
    writer.finish(writer_heap).unwrap();
    assert_eq!(stored_heap_len(&fixture), 4);

    let (frame, source_heap) = execution_frame(&fixture).await;
    frame.suspend(&source_heap).unwrap();
    let mut continuation_heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 3,
        ..RequestHeapLimits::default()
    });
    let sentinel = continuation_heap
        .alloc_array(vec![RuntimeValue::from("existing-local")])
        .unwrap();
    let before_len = continuation_heap.len();
    let before_stats = continuation_heap.stats();
    let execution = context(&fixture.interpreter).execution();

    let error = frame
        .resume(&mut continuation_heap, &execution)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("max heap nodes"));
    assert_eq!(continuation_heap.len(), before_len);
    assert_eq!(continuation_heap.stats(), before_stats);
    assert_eq!(
        continuation_heap.get(sentinel).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("existing-local")])
    );
    assert!(frame.read_field("payload").is_err());
    assert!(!frame.has_execution_lease());

    let authority = ActorExecutorAuthority::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        fixture.store.acquire_execution(&authority, &fixture.handle),
    )
    .await
    .expect("failed resume must release its temporary scheduler lease")
    .unwrap();
}

#[tokio::test]
async fn request_scoped_callback_capability_cannot_enter_actor_snapshot() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let capability = heap
        .alloc_interface(InterfaceValue::new(
            "contract:reader".to_string(),
            InterfaceCarrier::CallbackCapability(CallbackCapabilityCarrier::new(
                "runtime-a",
                "activation-a",
                7,
                "contract:reader",
                "capability-1",
            )),
        ))
        .unwrap();
    let nested = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "callback".to_string(),
            RuntimeValue::Heap(capability),
        )])))
        .unwrap();
    let root = heap.alloc_array(vec![RuntimeValue::Heap(nested)]).unwrap();
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let payload = fields
        .iter_mut()
        .find(|field| field.name == "payload")
        .unwrap();
    payload.value = RuntimeValue::Heap(root);
    payload.assigned = true;
    drop(fields);

    let error = frame.finish(heap).unwrap_err();
    assert!(error.to_string().contains("callback capability"));
    assert_eq!(stored_heap_len(&fixture), 0);
}

#[tokio::test]
async fn request_local_exception_cannot_enter_actor_snapshot() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    };
    let exception = RequestException::local(
        RuntimeValueCarrier::identified(
            RuntimeValue::from("private"),
            CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
                LocalExecutionTypeIdentity {
                    addr: TypeAddr {
                        unit: UnitAddr::Service,
                        file: FileAddr::LoadedFileIndex(0),
                        type_index: 0,
                    },
                    type_arguments: Vec::new(),
                },
            )),
        ),
        site.clone(),
        vec![ExceptionStackFrame::Local { site }],
        ErrorCorrelation {
            trace_id: "trace-actor-snapshot".to_string(),
            error_id: "error-actor-snapshot".to_string(),
        },
    )
    .unwrap();
    let exception = heap.alloc_exception(exception).unwrap();
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let payload = fields
        .iter_mut()
        .find(|field| field.name == "payload")
        .unwrap();
    payload.value = RuntimeValue::Heap(exception);
    payload.assigned = true;
    drop(fields);

    let error = frame.finish(heap).unwrap_err();
    assert!(error.to_string().contains("request-local exception"));
    assert_eq!(stored_heap_len(&fixture), 0);
}

#[tokio::test]
async fn request_scoped_stream_type_cannot_enter_actor_snapshot() {
    let fixture = stream_field_fixture();
    let (frame, heap) = execution_frame(&fixture).await;
    let fields = frame
        .suspension
        .lease
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .fields();
    let mut fields = fields.lock().unwrap();
    let stream = fields
        .iter_mut()
        .find(|field| field.name == "stream")
        .unwrap();
    stream.value = RuntimeValue::from("request-scoped-stream");
    stream.assigned = true;
    drop(fields);

    let error = frame.finish(heap).unwrap_err();
    assert!(error.to_string().contains("request-scoped Stream"));
    assert_eq!(stored_heap_len(&fixture), 0);
}

#[tokio::test]
async fn buffered_stream_next_does_not_create_a_scheduler_cut_point() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();

    let item = frame
        .await_if_pending(&mut heap, &execution, async {
            RuntimeValue::String("buffered".to_string())
        })
        .await
        .unwrap();

    assert_eq!(item, RuntimeValue::String("buffered".to_string()));
    assert!(frame.suspension.lease.lock().unwrap().is_some());
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(1.0)
    );
    frame.finish(heap).unwrap();
}

#[tokio::test]
async fn pending_stream_next_releases_scheduler_until_item_arrives() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();
    let (sender, receiver) = tokio::sync::oneshot::channel();

    let waiting = async {
        frame
            .await_if_pending(&mut heap, &execution, receiver)
            .await
            .unwrap()
            .unwrap()
    };
    let concurrent_method = async {
        assert_eq!(
            execute(&fixture, &fixture.method, b"[17]").await.unwrap(),
            b"17"
        );
        sender.send("ready").unwrap();
    };
    let (item, ()) = tokio::join!(waiting, concurrent_method);

    assert_eq!(item, "ready");
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(17.0)
    );
    frame.finish(heap).unwrap();
}

#[tokio::test]
async fn stale_epoch_resume_fails_without_reinstalling_execution_lease() {
    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    frame.suspend(&heap).unwrap();

    let mut newer_fence = fixture.handle.fence().clone();
    newer_fence.incarnation.epoch = 2;
    let program = fixture.interpreter.program_projection().unwrap();
    fixture
        .store
        .activate(ActorActivationRequest {
            fence: newer_fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: br#"[]"#,
            program: program.type_view(),
        })
        .unwrap();

    let execution = context(&fixture.interpreter).execution();
    let error = frame.resume(&mut heap, &execution).await.unwrap_err();
    assert!(error.to_string().contains("stale Actor epoch"));
    assert!(frame.suspension.lease.lock().unwrap().is_none());
}

#[tokio::test]
async fn cancelled_resume_does_not_reinstall_execution_lease() {
    use std::sync::atomic::Ordering;

    let fixture = fixture(integer(), true);
    let (frame, mut heap) = execution_frame(&fixture).await;
    frame.suspend(&heap).unwrap();
    let execution = context(&fixture.interpreter).execution();
    execution.cancel_flag().store(true, Ordering::Release);

    let error = frame.resume(&mut heap, &execution).await.unwrap_err();
    assert!(error.is_cancelled());
    assert!(frame.suspension.lease.lock().unwrap().is_none());
}

#[test]
fn suspension_probe_matrix_matches_only_real_async_native_paths() {
    for target in [
        "std.time.sleep",
        "std.file.read",
        "std.actor.get",
        "std.http.client.request",
        "std.http.client.stream",
        "std.http.client.sse",
        "std.http.stream.emitResponse",
    ] {
        assert!(
            crate::eval_context::native_call_suspends(target),
            "{target}"
        );
    }
    for target in [
        "core.date.now",
        "std.http.request.header",
        "std.websocket.sendTextToConnection",
    ] {
        assert!(
            !crate::eval_context::native_call_suspends(target),
            "{target}"
        );
    }
}

#[test]
fn connection_send_stays_inside_the_current_synchronous_segment() {
    for target in [
        "std.websocket.sendTextToConnection",
        "std.websocket.sendBinaryToConnection",
        "std.websocket.sendTextToBusinessIdentity",
        "std.websocket.sendBinaryToBusinessIdentity",
    ] {
        let semantics = skiff_artifact_model::native_callable_semantics(target)
            .expect("connection send has exact callable semantics");
        assert!(!semantics.effects.may_suspend, "{target}");
        assert!(
            !crate::eval_context::native_call_suspends(target),
            "{target}"
        );
    }
}

#[tokio::test]
async fn executor_rechecks_abi_implementation_epoch_and_rejects_ordinary_context() {
    let base = fixture(integer(), false);
    let mut wrong_abi_file = (*actor_file(integer(), false)).clone();
    wrong_abi_file.actor_declarations[0].actor_abi_identity =
        ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:wrong");
    let (wrong_abi_interpreter, _) = interpreter_for(Arc::new(wrong_abi_file));
    let error = ActorMethodExecutor::new(&base.store)
        .execute(
            &wrong_abi_interpreter,
            ActorMethodExecutionRequest {
                instance: &base.handle,
                method_identity: &base.method,
                arguments_payload: b"[2]",
                context: context(&wrong_abi_interpreter),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ActorMethodExecutorError::Store(ActorInstanceStoreError::ActorAbiMismatch)
    ));

    let mut wrong_impl_file = (*actor_file(integer(), false)).clone();
    wrong_impl_file.actor_declarations[0].actor_implementation_identity =
        ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:wrong");
    let (wrong_impl_interpreter, _) = interpreter_for(Arc::new(wrong_impl_file));
    let error = ActorMethodExecutor::new(&base.store)
        .execute(
            &wrong_impl_interpreter,
            ActorMethodExecutionRequest {
                instance: &base.handle,
                method_identity: &base.method,
                arguments_payload: b"[2]",
                context: context(&wrong_impl_interpreter),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ActorMethodExecutorError::Store(ActorInstanceStoreError::ActorImplementationMismatch)
    ));

    let program = base.interpreter.program_projection().unwrap();
    let mut newer_fence = base.handle.fence().clone();
    newer_fence.incarnation.epoch = 2;
    base.store
        .activate(ActorActivationRequest {
            fence: newer_fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: br#"[]"#,
            program: program.type_view(),
        })
        .unwrap();
    assert!(matches!(
        execute(&base, &base.method, b"[3]").await,
        Err(ActorMethodExecutorError::Store(
            ActorInstanceStoreError::StaleEpoch { .. }
        ))
    ));

    let ordinary = fixture(integer(), false);
    let mut heap = skiff_runtime_model::request_heap::RequestHeap::default();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    let error = ordinary
        .interpreter
        .call_program_executable(
            context(&ordinary.interpreter),
            &mut heap,
            &crate::env::Env::new(),
            &addr,
            &addr,
            &Default::default(),
            vec![RuntimeValue::Number(4.0)],
        )
        .await
        .unwrap_err();
    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error.to_string().contains("current Actor execution token"));
}

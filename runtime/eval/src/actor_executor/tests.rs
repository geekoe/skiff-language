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
    ServiceSymbolRef, SlotIr, SlotLayoutIr, SourceMapDto, StmtRefIr, TypeAddr, TypeDeclIr,
    UnitAddr,
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
    fixture_from_file_with_limits(file, initialize, true, RequestHeapLimits::default())
}

fn fixture_from_file_with_admission(
    file: Arc<LinkedFileUnit>,
    initialize: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap),
    admitted: bool,
) -> Fixture {
    fixture_from_file_with_limits(file, initialize, admitted, RequestHeapLimits::default())
}

fn fixture_from_file_with_limits(
    file: Arc<LinkedFileUnit>,
    initialize: impl FnOnce(&mut [ActorFieldValue], &mut RequestHeap),
    admitted: bool,
    arena_limits: RequestHeapLimits,
) -> Fixture {
    let (interpreter, program) = interpreter_for(file);
    let mut store = ActorInstanceStore::new();
    store.arena_limits = arena_limits;
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

fn fixture_with_arena_limits(
    return_type: LinkedTypeRef,
    may_suspend: bool,
    arena_limits: RequestHeapLimits,
) -> Fixture {
    fixture_from_file_with_limits(
        actor_file(return_type, may_suspend),
        |fields, _| {
            fields[1].value = RuntimeValue::Number(1.0);
            fields[1].assigned = true;
        },
        true,
        arena_limits,
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

    assert_eq!(
        tracker.discard_session(SESSION_ID),
        0,
        "discard is deferred while the pending create segment is live"
    );
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);
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

    assert_eq!(
        tracker.discard_session(SESSION_ID),
        0,
        "the live create segment defers exact discard"
    );
    assert_eq!(tracker.tracked_owner_count_for_test(), 0);

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
    assert!(
        store.is_empty(),
        "the pending-discard instance must be reclaimed when the old segment ends"
    );

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

async fn execution_frame(fixture: &Fixture) -> (ActorExecutionFrame, HeapAccess) {
    execution_frame_with_activation(fixture, false).await
}

async fn execution_frame_with_activation(
    fixture: &Fixture,
    activation: bool,
) -> (ActorExecutionFrame, HeapAccess) {
    let authority = ActorExecutorAuthority::new();
    let mut segment = if activation {
        fixture
            .store
            .acquire_segment_for_activation(&authority, &fixture.handle)
            .await
            .unwrap()
    } else {
        fixture
            .store
            .acquire_segment(&authority, &fixture.handle)
            .await
            .unwrap()
    };
    let access = HeapAccess::with_guard(segment.arena().clone(), segment.take_guard());
    (
        ActorExecutionFrame::new(
            fixture.store.clone(),
            fixture.handle.clone(),
            segment,
            activation,
        ),
        access,
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
    access: &mut HeapAccess,
    execution: &crate::capabilities::ExecutionControl<'_>,
) {
    let (sender, receiver) = tokio::sync::oneshot::channel::<()>();
    let waiting = frame.await_if_pending(access, execution, receiver);
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
        .acquire_segment(&authority, &fixture.handle)
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
async fn wrong_return_type_leaves_partial_field_writes_and_may_suspend_method_executes() {
    // Design §3.4: a failed segment keeps already-executed field mutations.
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
    let fields = wrong_return
        .store
        .with_fields_for_executor(
            &ActorExecutorAuthority::new(),
            &wrong_return.handle,
            |fields, _| fields.to_vec(),
        )
        .unwrap();
    assert_eq!(fields[1].value, RuntimeValue::Number(9.0));
    assert!(fields[1].assigned);
    assert_eq!(
        wrong_return
            .store
            .segment_counters_for_test(&wrong_return.handle)
            .unwrap(),
        (0, 0),
        "the failed segment must still release its continuation counters"
    );

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
async fn real_suspension_is_zero_copy_and_keeps_continuation_handles() {
    let fixture = fixture(integer(), true);
    let arena_before = fixture.store.arena_ptr_for_test(&fixture.handle).unwrap();
    let epoch_before = fixture.store.arena_epoch_for_test(&fixture.handle).unwrap();
    let (frame, mut access) = execution_frame(&fixture).await;
    let nodes_before = access.len();
    let continuation_local = access
        .alloc_array(vec![RuntimeValue::String("continuation-local".to_string())])
        .unwrap();
    for index in 0..512 {
        access
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
                "dead-local".to_string(),
                RuntimeValue::Number(index as f64),
            )])))
            .unwrap();
    }
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    frame
        .write_field(
            "count",
            &integer(),
            program.type_view(),
            &addr,
            &RuntimeValue::Number(5.0),
            access.heap_mut(),
        )
        .unwrap();

    let execution = context(&fixture.interpreter).execution();
    force_pending_cut(&frame, &mut access, &execution).await;

    assert_eq!(
        fixture.store.arena_ptr_for_test(&fixture.handle).unwrap(),
        arena_before,
        "a real Pending cut must not clone or replace the shared arena"
    );
    assert_eq!(
        fixture.store.arena_epoch_for_test(&fixture.handle).unwrap(),
        epoch_before
    );
    assert_eq!(
        access.len(),
        nodes_before + 513,
        "continuation locals and the field graph stay in the same arena"
    );
    assert_eq!(
        access.get(continuation_local).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::String("continuation-local".to_string())])
    );
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(5.0)
    );
    assert_eq!(
        fixture
            .store
            .segment_counters_for_test(&fixture.handle)
            .unwrap(),
        (1, 0)
    );
    frame.finish().unwrap();
    assert_eq!(
        fixture
            .store
            .segment_counters_for_test(&fixture.handle)
            .unwrap(),
        (0, 0)
    );
}

#[tokio::test]
async fn repeated_invocations_reuse_the_arena_and_quiescence_compaction_drops_locals() {
    let fixture = fixture_with_arena_limits(
        integer(),
        true,
        RequestHeapLimits {
            max_nodes: 256,
            ..RequestHeapLimits::default()
        },
    );
    let arena_before = fixture.store.arena_ptr_for_test(&fixture.handle).unwrap();
    let epoch_before = fixture.store.arena_epoch_for_test(&fixture.handle).unwrap();
    let baseline = stored_heap_len(&fixture);

    for round in 0..4 {
        let (frame, mut access) = execution_frame(&fixture).await;
        for index in 0..48 {
            access
                .alloc_array(vec![
                    RuntimeValue::Number(round as f64),
                    RuntimeValue::Number(index as f64),
                ])
                .unwrap();
        }
        frame.finish().unwrap();
        drop(access);
        assert_eq!(
            fixture.store.arena_ptr_for_test(&fixture.handle).unwrap(),
            arena_before,
            "invocation {round} must reuse the shared arena"
        );
    }
    let grown = stored_heap_len(&fixture);
    assert!(
        grown > baseline,
        "dead invocation locals accumulate until compaction"
    );

    let compacted = fixture
        .store
        .compact_if_quiescent(&fixture.handle)
        .await
        .unwrap();
    assert!(compacted);
    assert_eq!(
        stored_heap_len(&fixture),
        baseline,
        "quiescence compaction must drop dead invocation locals"
    );
    assert_eq!(
        fixture.store.arena_epoch_for_test(&fixture.handle).unwrap(),
        epoch_before + 1,
        "compaction must bump the arena epoch"
    );
    assert_ne!(
        fixture.store.arena_ptr_for_test(&fixture.handle).unwrap(),
        arena_before
    );

    let (frame, mut access) = execution_frame(&fixture).await;
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(1.0),
        "field roots must survive compaction"
    );
    let stale = access
        .alloc_array(vec![RuntimeValue::from("new-epoch-local")])
        .unwrap();
    for index in 0..130 {
        access
            .alloc_array(vec![RuntimeValue::Number(index as f64)])
            .unwrap();
    }
    frame.finish().unwrap();
    drop(access);
    drop(frame);

    assert!(fixture
        .store
        .compact_if_quiescent(&fixture.handle)
        .await
        .unwrap());
    let (_frame, access) = execution_frame(&fixture).await;
    let error = access.get(stale).unwrap_err();
    assert!(
        error.to_string().contains("epoch does not match heap slot"),
        "stale handles from a compacted arena must fail closed: {error}"
    );
    frame_finish_and_drop(_frame, access).await;
}

#[tokio::test]
async fn heap_backed_actor_field_lives_in_one_arena_across_calls() {
    let fixture = heap_field_fixture();
    let arena_before = fixture.store.arena_ptr_for_test(&fixture.handle).unwrap();
    let item_array = array(integer());
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };

    let (first_frame, mut first_access) = execution_frame(&fixture).await;
    let replacement = first_access
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
            first_access.heap_mut(),
        )
        .expect("heap-backed Actor field should be writable");
    first_frame
        .finish()
        .expect("first Actor call should commit");
    drop(first_access);

    let (second_frame, second_access) = execution_frame(&fixture).await;
    assert_eq!(
        fixture.store.arena_ptr_for_test(&fixture.handle).unwrap(),
        arena_before,
        "calls must share one arena (zero-copy commit)"
    );
    let RuntimeValue::Heap(items) = second_frame
        .read_field("items")
        .expect("heap-backed Actor field should survive into the next call")
    else {
        panic!("heap-backed Actor field must remain a heap value")
    };
    assert_eq!(
        items, replacement,
        "field roots must keep handle identity across calls"
    );
    assert_eq!(
        second_access.get(items).unwrap(),
        &HeapNode::Array(vec![
            RuntimeValue::Number(2.0),
            RuntimeValue::Number(3.0),
            RuntimeValue::Number(5.0),
        ])
    );
    second_frame
        .finish()
        .expect("second Actor call should commit");
    drop(second_access);
}

#[tokio::test]
async fn nominal_aliased_field_graph_is_live_across_pending_cuts_and_compaction() {
    const ITEM_COUNT: usize = 32;
    const PENDING_CUTS: usize = 5;

    let fixture = fixture_with_arena_limits(
        integer(),
        true,
        RequestHeapLimits {
            max_nodes: 1024,
            ..RequestHeapLimits::default()
        },
    );
    let arena_before = fixture.store.arena_ptr_for_test(&fixture.handle).unwrap();
    let (frame, mut access) = execution_frame(&fixture).await;
    let mut item_values = Vec::with_capacity(ITEM_COUNT);
    for index in 0..ITEM_COUNT {
        let tags = access
            .alloc_array(vec![
                RuntimeValue::from("runtime"),
                RuntimeValue::from(format!("actor-{index}")),
            ])
            .unwrap();
        let item = access
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
    let items = access.alloc_array(item_values).unwrap();
    let payload = access
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "items".to_string(),
            RuntimeValue::Heap(items),
        )])))
        .unwrap();
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    frame
        .write_field(
            "payload",
            &payload_type(),
            program.type_view(),
            &addr,
            &RuntimeValue::Heap(payload),
            access.heap_mut(),
        )
        .unwrap();
    let payload_value = frame.read_field("payload").unwrap();
    assert!(
        fixture
            .store
            .set_field_root(&fixture.handle, "payload_alias", payload_value.clone())
            .unwrap(),
        "payload_alias root must exist"
    );
    for index in 0..600 {
        access
            .alloc_array(vec![RuntimeValue::Number(index as f64)])
            .unwrap();
    }
    let stale_local = access
        .alloc_array(vec![RuntimeValue::from("continuation-local")])
        .unwrap();
    let execution = context(&fixture.interpreter).execution();
    let nodes_before_pending = access.len();
    for _ in 0..PENDING_CUTS {
        force_pending_cut(&frame, &mut access, &execution).await;
        assert_eq!(
            access.len(),
            nodes_before_pending,
            "real Pending cuts must not clone or import the Actor field graph"
        );
        assert_eq!(
            fixture.store.arena_ptr_for_test(&fixture.handle).unwrap(),
            arena_before
        );
    }
    assert_eq!(
        frame.read_field("payload_alias").unwrap(),
        frame.read_field("payload").unwrap(),
        "aliases stay live across suspension"
    );
    assert_eq!(
        access.get(stale_local).unwrap(),
        &HeapNode::Array(vec![RuntimeValue::from("continuation-local")])
    );
    frame.finish().unwrap();
    drop(access);

    let compacted = fixture
        .store
        .compact_if_quiescent(&fixture.handle)
        .await
        .unwrap();
    assert!(compacted);
    let compacted_len = stored_heap_len(&fixture);
    assert!(
        compacted_len < nodes_before_pending,
        "compaction must drop dead invocation locals"
    );

    let (frame, access) = execution_frame(&fixture).await;
    let payload = frame.read_field("payload").unwrap();
    assert_eq!(
        frame.read_field("payload_alias").unwrap(),
        payload,
        "multi-root compaction must preserve aliases between Actor fields"
    );
    let RuntimeValue::Heap(payload) = payload else {
        panic!("payload field must remain heap-backed");
    };
    let HeapNode::Object(payload_node) = access.get(payload).unwrap() else {
        panic!("payload must remain a nominal record object");
    };
    let items = payload_node.fields()["items"].as_heap_handle().unwrap();
    let HeapNode::Array(items_node) = access.get(items).unwrap() else {
        panic!("payload.items must remain an array");
    };
    let item = items_node[0].as_heap_handle().unwrap();
    let HeapNode::Object(item_node) = access.get(item).unwrap() else {
        panic!("payload item must remain a record object");
    };
    assert_eq!(item_node.fields()["name"], RuntimeValue::from("first"));
    let tags = item_node.fields()["tags"].as_heap_handle().unwrap();
    assert_eq!(
        access.get(tags).unwrap(),
        &HeapNode::Array(vec![
            RuntimeValue::from("runtime"),
            RuntimeValue::from("actor-0")
        ])
    );
    let error = access.get(stale_local).unwrap_err();
    assert!(
        error.to_string().contains("epoch does not match heap slot"),
        "stale handles must fail closed after compaction: {error}"
    );
    frame.finish().unwrap();
    drop(access);
}

#[tokio::test]
async fn compaction_root_scan_rejects_request_scoped_callback_and_exception() {
    let fixture = fixture_with_arena_limits(
        integer(),
        true,
        RequestHeapLimits {
            max_nodes: 16,
            ..RequestHeapLimits::default()
        },
    );

    let (frame, mut access) = execution_frame(&fixture).await;
    let capability = access
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
    let nested = access
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "callback".to_string(),
            RuntimeValue::Heap(capability),
        )])))
        .unwrap();
    let root = access
        .alloc_array(vec![RuntimeValue::Heap(nested)])
        .unwrap();
    for index in 0..8 {
        access
            .alloc_array(vec![RuntimeValue::Number(index as f64)])
            .unwrap();
    }
    assert!(fixture
        .store
        .set_field_root(&fixture.handle, "payload", RuntimeValue::Heap(root))
        .unwrap());
    frame.finish().unwrap();
    drop(access);

    let error = fixture
        .store
        .compact_if_quiescent(&fixture.handle)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("callback capability"),
        "unexpected compaction error: {error}"
    );
    assert_eq!(fixture.store.len(), 1);

    // Request-local exceptions are rejected the same way at root collection.
    let fixture = fixture_with_arena_limits(
        integer(),
        true,
        RequestHeapLimits {
            max_nodes: 16,
            ..RequestHeapLimits::default()
        },
    );
    let (frame, mut access) = execution_frame(&fixture).await;
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
            trace_id: "trace-actor-compaction".to_string(),
            error_id: "error-actor-compaction".to_string(),
        },
    )
    .unwrap();
    let exception = access.alloc_exception(exception).unwrap();
    for index in 0..8 {
        access
            .alloc_array(vec![RuntimeValue::Number(index as f64)])
            .unwrap();
    }
    assert!(fixture
        .store
        .set_field_root(&fixture.handle, "payload", RuntimeValue::Heap(exception))
        .unwrap());
    frame.finish().unwrap();
    drop(access);

    let error = fixture
        .store
        .compact_if_quiescent(&fixture.handle)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("request-local exception"),
        "unexpected compaction error: {error}"
    );
}

#[tokio::test]
async fn request_scoped_stream_type_is_rejected_on_the_write_path() {
    let fixture = stream_field_fixture();
    let (frame, mut access) = execution_frame(&fixture).await;
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    let stream_type = array(LinkedTypeRef::Nullable {
        inner: Box::new(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![string()],
        }),
    });
    let error = frame
        .write_field(
            "stream",
            &stream_type,
            program.type_view(),
            &addr,
            &RuntimeValue::String("request-scoped-stream".to_string()),
            access.heap_mut(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("request-scoped Stream"));
    assert!(
        frame.read_field("stream").is_err(),
        "rejected write must not assign the field root"
    );
    frame.finish().unwrap();
}

#[tokio::test]
async fn arena_limits_error_path_reports_resource_limit_and_releases_segment() {
    let fixture = fixture_with_arena_limits(
        integer(),
        true,
        RequestHeapLimits {
            max_nodes: 2,
            ..RequestHeapLimits::default()
        },
    );
    let (frame, mut access) = execution_frame(&fixture).await;
    access.alloc_array(Vec::new()).unwrap();
    access.alloc_array(Vec::new()).unwrap();
    let error = access
        .alloc_array(Vec::new())
        .expect_err("the third allocation must exceed the per-instance arena limit");
    assert!(matches!(
        error,
        skiff_runtime_model::error::RuntimeModelError::ResourceLimitExceeded { reason, .. }
            if reason == "max heap nodes"
    ));
    assert_eq!(
        fixture
            .store
            .segment_counters_for_test(&fixture.handle)
            .unwrap(),
        (1, 0),
        "the segment is still live while the method handles the limit error"
    );
    frame.finish().unwrap();
    drop(access);
    assert_eq!(
        fixture
            .store
            .segment_counters_for_test(&fixture.handle)
            .unwrap(),
        (0, 0)
    );
}

#[tokio::test]
async fn partial_create_unassigned_roots_stay_null_and_resume_can_assign() {
    let fixture = activation_fixture();
    let (frame, mut access) = execution_frame_with_activation(&fixture, true).await;
    assert!(
        frame.read_field("payload").is_err(),
        "unassigned create roots are not readable"
    );
    let execution = context(&fixture.interpreter).execution();
    force_pending_cut(&frame, &mut access, &execution).await;
    assert!(
        frame.read_field("payload").is_err(),
        "unassigned roots remain unreadable across suspension"
    );
    let items = access.alloc_array(Vec::new()).unwrap();
    let payload = access
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
            access.heap_mut(),
        )
        .unwrap();
    assert_eq!(
        frame.read_field("payload").unwrap(),
        RuntimeValue::Heap(payload)
    );
    frame.finish().unwrap();
    drop(access);
    assert_eq!(stored_heap_len(&fixture), 2);
}

async fn frame_finish_and_drop(frame: ActorExecutionFrame, access: HeapAccess) {
    frame.finish().unwrap();
    drop(access);
}

#[tokio::test]
async fn buffered_stream_next_does_not_create_a_scheduler_cut_point() {
    let fixture = fixture(integer(), true);
    let (frame, mut access) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();

    let item = frame
        .await_if_pending(&mut access, &execution, async {
            RuntimeValue::String("buffered".to_string())
        })
        .await
        .unwrap();

    assert_eq!(item, RuntimeValue::String("buffered".to_string()));
    assert!(frame.has_execution_lease());
    assert!(!frame.is_suspended());
    assert_eq!(
        fixture
            .store
            .segment_counters_for_test(&fixture.handle)
            .unwrap(),
        (1, 0),
        "a Ready poll must keep the segment active"
    );
    assert_eq!(
        frame.read_field("count").unwrap(),
        RuntimeValue::Number(1.0)
    );
    frame.finish().unwrap();
}

#[tokio::test]
async fn pending_stream_next_releases_scheduler_until_item_arrives() {
    let fixture = fixture(integer(), true);
    let (frame, mut access) = execution_frame(&fixture).await;
    let execution = context(&fixture.interpreter).execution();
    let (sender, receiver) = tokio::sync::oneshot::channel();

    let waiting = async {
        frame
            .await_if_pending(&mut access, &execution, receiver)
            .await
            .unwrap()
            .unwrap()
    };
    let concurrent_method = async {
        tokio::task::yield_now().await;
        assert_eq!(
            fixture
                .store
                .segment_counters_for_test(&fixture.handle)
                .unwrap(),
            (0, 1),
            "the guard must be released and the continuation suspended at Pending"
        );
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
        RuntimeValue::Number(17.0),
        "aliases live: the resumed segment reads the concurrent method's write"
    );
    frame.finish().unwrap();
}

#[tokio::test]
async fn stale_epoch_resume_fails_without_reinstalling_execution_lease() {
    let fixture = fixture(integer(), true);
    let (frame, mut access) = execution_frame(&fixture).await;
    frame.suspend().unwrap();
    access.release();

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
    let error = frame.resume(&execution).unwrap_err();
    assert!(error.to_string().contains("stale Actor epoch"));
    assert!(frame.is_suspended());
    drop(frame);
    drop(access);
}

#[tokio::test]
async fn cancelled_resume_does_not_reinstall_execution_lease() {
    use std::sync::atomic::Ordering;

    let fixture = fixture(integer(), true);
    let (frame, mut access) = execution_frame(&fixture).await;
    frame.suspend().unwrap();
    access.release();
    let execution = context(&fixture.interpreter).execution();
    execution.cancel_flag().store(true, Ordering::Release);

    let error = frame.resume(&execution).unwrap_err();
    assert!(error.is_cancelled());
    assert!(frame.is_suspended());
    drop(frame);
    drop(access);
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
    let heap = skiff_runtime_model::request_heap::RequestHeap::default();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    let error = ordinary
        .interpreter
        .call_program_executable(
            context(&ordinary.interpreter),
            &mut HeapAccess::private(heap),
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

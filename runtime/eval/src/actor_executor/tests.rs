use std::{collections::BTreeMap, sync::Arc};

use skiff_artifact_model::{
    ActorAbiIdentity, ActorFieldEncodingIr, ActorImplementationIdentity, ActorMethodIdentity,
    ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_runtime_linked_program::{
    AssignTargetIr, BlockIr, ExecutableAddr, ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr,
    FileDeclarations, FileLinkTargets, LinkOverlay, LinkedActorDeclaration,
    LinkedActorDeclarationOwner, LinkedActorField, LinkedActorMethodImplementation,
    LinkedActorPublicMethod, LinkedExecutable, LinkedExecutableBody, LinkedExprIr, LinkedFileUnit,
    LinkedFunctionTypeParamIr, LinkedStmtIr, LinkedTypeRef, ParamIr, PublicationResourceTable,
    RuntimeTypeContext, ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, SourceMapDto,
    StmtRefIr, UnitAddr,
};
use skiff_runtime_model::{request_heap::RequestHeapLimits, runtime_value::RuntimeValue};

use super::*;
use crate::{
    actor_executor_test_runtime as test_runtime,
    actor_instance::{
        ActorActivationRequest, ActorIncarnationKey, ActorInstanceFence, ActorLogicalKey,
        ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::TimeCapabilityContext,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram,
};
use sha2::{Digest, Sha256};
mod actor_concurrent_continuation;

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
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: FILE_ID.to_string(),
        source_ast_hash: "source:actor-executor".to_string(),
        module_path: "actors".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
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
            fields: vec![LinkedActorField {
                name: "count".to_string(),
                ty: integer(),
                encoding: ActorFieldEncodingIr::CanonicalValueV1,
            }],
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
        types: Vec::new(),
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

fn interpreter_for(file: Arc<LinkedFileUnit>) -> (Interpreter, Arc<EvalRuntimeProgram>) {
    let program = Arc::new(EvalRuntimeProgram::new(
        "skiff.run/counter",
        vec![file],
        Vec::new(),
        PublicationResourceTable::default(),
        Default::default(),
        LinkOverlay::default(),
        RuntimeTypeContext::default(),
    ));
    (
        Interpreter::with_program(Arc::clone(&program), test_runtime::runtime_factory()),
        program,
    )
}

fn fixture(return_type: LinkedTypeRef, may_suspend: bool) -> Fixture {
    let file = actor_file(return_type, may_suspend);
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
    let payload = br#"{"count":1}"#;
    let handle = store
        .activate(ActorActivationRequest {
            fence,
            bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
            bootstrap_payload: payload,
            program: program.projection().type_view(),
        })
        .unwrap();
    Fixture {
        interpreter,
        store,
        handle,
        method: method_identity(),
    }
}

fn context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context();
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
        spawn: actor,
        request_heap_limits: RequestHeapLimits::default(),
    })
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
    let authority = ActorExecutorAuthority::new();
    let mut lease = fixture
        .store
        .acquire_execution(&authority, &fixture.handle)
        .await
        .unwrap();
    let heap = lease.take_heap();
    let program = fixture.interpreter.program_projection().unwrap();
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 0,
    };
    let plan = RuntimeTypePlan::from_linked(
        &integer(),
        &PlanContext::from_type_view(program.type_view(), &addr),
    )
    .unwrap();
    (
        ActorExecutionFrame::new(
            fixture.store.clone(),
            fixture.handle.clone(),
            lease,
            vec![("count".to_string(), plan)],
        ),
        heap,
    )
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
            bootstrap_payload: br#"{"count":20}"#,
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
        "std.actor.find",
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
            bootstrap_payload: br#"{"count":20}"#,
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

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use serde_json::json;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_boundary::stream::stream_id;
use skiff_runtime_boundary::type_descriptor::RuntimeTypePlanDescriptorExt;
use skiff_runtime_capability_context::{
    StreamRuntimeError, SupervisedStreamConsumptionChild, SupervisedStreamConsumptionLease,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, ExecutableKind, FileAddr, FileDeclarations, FileLinkTargets, LinkOverlay,
    LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedFileUnit, LinkedTypeDescriptor,
    LinkedTypeRef, ParamIr, PublicationResourceTable, RuntimeTypeContext, ServiceMeta, SlotIr,
    SlotLayoutIr, SourceMapDto, TypeAddr, TypeDeclIr, UnitAddr,
};
use skiff_runtime_model::service_error::{
    CatchIdentity, LocalExecutionTypeIdentity, NominalTypeIdentity,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use super::super::{
    assembly_execution::RuntimeExecutionProjection,
    capabilities::{StreamPoll, StreamRuntime},
    env::Env,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram, Interpreter,
};
use super::{PreparedNativeStreamProducer, StreamProducerCall};
use crate::error::{unwrap_diagnostic_source_context, RuntimeError};
use crate::{
    actor_executor_test_runtime as test_runtime, capabilities::TimeCapabilityContext,
    runtime_ops::runtime_to_wire,
};

#[test]
fn ordinary_stream_supervision_ignores_unrelated_stream_values() {
    let expected = json!({"$stream": "expected"});
    let unrelated = json!({"$stream": "unrelated"});
    let lease = SupervisedStreamConsumptionLease::from_cancel(&expected, |_| {});
    let mut env = Env::new();
    env.supervise_stream_consumer(expected.clone(), lease.child());

    assert!(env.stream_consumer_supervision_for(&expected).is_some());
    assert!(env.stream_consumer_supervision_for(&unrelated).is_none());
    lease.hard_cancel();
}

#[tokio::test]
async fn ordinary_executable_stream_consumer_preserves_late_producer_error() {
    let error = execute_program(vec![
        stream_consumer_route(),
        failing_stream_consumer(),
        emit_then_fail_producer(),
    ])
    .await
    .expect_err("late producer error should win over the ordinary consumer error");

    assert_producer_error(&error);
}

#[tokio::test]
async fn ordinary_executable_stream_consumer_keeps_own_error_after_producer_end() {
    let error = execute_program(vec![
        stream_consumer_route(),
        failing_stream_consumer(),
        emit_then_end_producer(),
    ])
    .await
    .expect_err("ordinary consumer error should survive a natural producer End");

    assert!(
        error.to_string().contains("slot 98"),
        "consumer error must not become an unknown Stream value: {error}"
    );
}

#[tokio::test]
async fn ordinary_executable_stream_consumer_observes_producer_error_directly() {
    let error = execute_program(vec![
        stream_consumer_route(),
        draining_stream_consumer(),
        emit_then_fail_producer(),
    ])
    .await
    .expect_err("the draining ordinary consumer should observe the producer error");

    assert_producer_error(&error);
}

#[tokio::test]
async fn ordinary_executable_stream_consumer_natural_end_completes() {
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        execute_program(vec![
            stream_consumer_route(),
            draining_stream_consumer(),
            emit_then_end_producer(),
        ]),
    )
    .await
    .expect("natural End must not leave the stream registry pending")
    .expect("natural End should complete the ordinary consumer");

    assert_eq!(result, RuntimeValue::Null);
}

#[tokio::test]
async fn tail_call_negative_stream_producing_argument_stays_ordinary_and_drains_owner() {
    let error = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        execute_program_at_depth(
            vec![
                stream_consumer_route(),
                draining_stream_consumer(),
                emit_then_end_producer(),
            ],
            31,
        ),
    )
    .await
    .expect("the prepared producer must drain before the ordinary continuation returns")
    .expect_err("the exact consumer call must retain its nested depth push");

    assert_program_depth_error(&error);
}

#[tokio::test]
async fn tail_call_negative_stream_producer_call_remains_deferred_at_depth_limit() {
    let (interpreter, file) =
        interpreter_with_executables(vec![stream_producer_route(1), emit_then_end_producer()]);
    let context = execution_context(&interpreter).with_program_call_depth_for_test(31);
    let route_addr = ExecutableAddr::service(0, 0);
    let mut heap = RequestHeap::default();
    let result = interpreter
        .call_program_executable(
            context,
            &mut heap,
            &Env::new(),
            &route_addr,
            &route_addr,
            &Default::default(),
            Vec::new(),
        )
        .await
        .expect("a tail-position producer call must return its deferred Stream handle");
    let stream_value =
        runtime_to_wire(&result, &heap).expect("returned Stream handle must remain wire encodable");
    assert!(
        stream_id(&stream_value).is_some(),
        "the deferred producer continuation must return a canonical Stream value"
    );

    let stream_runtime = interpreter.stream_runtime.clone();
    let consumed_stream_value = stream_value.clone();
    let values = interpreter
        .drive_deferred_stream_producer(
            execution_context(&interpreter),
            &route_addr,
            &stream_value,
            |supervision| {
                consume_deferred_stream(
                    stream_runtime.clone(),
                    consumed_stream_value.clone(),
                    supervision,
                )
            },
        )
        .await
        .expect("the parked producer must remain executable after its caller returned");

    assert_eq!(values, 2, "the deferred continuation must emit both items");
    assert_stream_closed(&stream_runtime, &stream_value).await;
    assert_eq!(
        file.executables[0].return_type,
        file.executables[1].return_type,
        "the negative case must exclude tail transfer because of stream semantics, not return-plan mismatch"
    );
}

#[test]
fn ordinary_executable_stream_supervision_crosses_nested_helper_call() {
    std::thread::Builder::new()
        .name("nested-stream-supervision".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("nested stream test runtime")
                .block_on(async {
                    let error = execute_program(vec![
                        stream_consumer_route_with(1, 3),
                        nested_stream_consumer(2),
                        failing_stream_consumer(),
                        emit_then_fail_producer(),
                    ])
                    .await
                    .expect_err("nested ordinary consumer should preserve the late producer error");

                    assert_producer_error(&error);
                });
        })
        .expect("nested stream test thread")
        .join()
        .expect("nested stream test thread should not panic");
}

#[tokio::test]
async fn prepared_stream_outer_cancellation_removes_registry_before_return() {
    let PreparedFixture {
        interpreter,
        context,
        caller_addr,
        prepared,
        stream_runtime,
        stream_value,
    } = prepared_fixture().await;
    let result = interpreter
        .exec_prepared_native_stream_producer_arg(context, &caller_addr, prepared, async {
            Err::<RuntimeValue, _>(RuntimeError::Cancelled)
        })
        .await;

    assert!(matches!(result, Err(RuntimeError::Cancelled)));
    assert_stream_closed(&stream_runtime, &stream_value).await;
}

#[tokio::test]
async fn prepared_stream_outer_drop_cleans_registry_without_late_result() {
    let PreparedFixture {
        interpreter,
        context,
        caller_addr,
        prepared,
        stream_runtime,
        stream_value,
    } = prepared_fixture().await;
    let mut execution = Box::pin(interpreter.exec_prepared_native_stream_producer_arg(
        context,
        &caller_addr,
        prepared,
        std::future::pending::<crate::error::Result<RuntimeValue>>(),
    ));

    tokio::select! {
        biased;
        result = &mut execution => panic!("pending consumer completed unexpectedly: {result:?}"),
        () = tokio::task::yield_now() => {}
    }
    drop(execution);

    assert_stream_closed(&stream_runtime, &stream_value).await;
    tokio::task::yield_now().await;
    assert_stream_closed(&stream_runtime, &stream_value).await;
}

#[tokio::test]
async fn deferred_stream_drive_natural_end_completes_without_leak() {
    let fixture = deferred_fixture(emit_then_end_producer()).await;
    let stream_runtime = fixture.stream_runtime.clone();
    let stream_value = fixture.stream_value.clone();
    let values = fixture
        .interpreter
        .drive_deferred_stream_producer(
            fixture.context,
            &fixture.caller_addr,
            &fixture.stream_value,
            |supervision| {
                consume_deferred_stream(stream_runtime.clone(), stream_value.clone(), supervision)
            },
        )
        .await
        .expect("deferred producer should reach natural End");

    assert_eq!(values, 2);
    assert_stream_closed(&stream_runtime, &stream_value).await;
}

#[tokio::test]
async fn deferred_stream_drive_cancellation_closes_without_leak() {
    let fixture = deferred_fixture(emit_then_end_producer()).await;
    let stream_runtime = fixture.stream_runtime.clone();
    let stream_value = fixture.stream_value.clone();
    let result = fixture
        .interpreter
        .drive_deferred_stream_producer(
            fixture.context,
            &fixture.caller_addr,
            &fixture.stream_value,
            |_| async { Err::<(), _>(RuntimeError::Cancelled) },
        )
        .await;

    assert!(matches!(result, Err(RuntimeError::Cancelled)));
    assert_stream_closed(&stream_runtime, &stream_value).await;
}

#[tokio::test]
async fn deferred_stream_drive_preserves_consumed_producer_error() {
    let fixture = deferred_fixture(emit_then_fail_producer()).await;
    let stream_runtime = fixture.stream_runtime.clone();
    let stream_value = fixture.stream_value.clone();
    let error = fixture
        .interpreter
        .drive_deferred_stream_producer(
            fixture.context,
            &fixture.caller_addr,
            &fixture.stream_value,
            |supervision| {
                consume_deferred_stream(stream_runtime.clone(), stream_value.clone(), supervision)
            },
        )
        .await
        .expect_err("deferred producer error should reach its consumer");

    assert!(
        !error.to_string().contains("unknown Stream value"),
        "consumed producer terminal must not trigger a second registry read: {error}"
    );
    assert_stream_closed(&stream_runtime, &stream_value).await;
}

async fn consume_deferred_stream(
    stream_runtime: StreamRuntime,
    stream_value: serde_json::Value,
    supervision: Option<SupervisedStreamConsumptionChild>,
) -> crate::error::Result<usize> {
    let supervision = supervision.expect("parked deferred producer must provide supervision");
    let mut cleanup = supervision.consumer_cleanup(&stream_value);
    let mut values = 0usize;
    loop {
        match stream_runtime.next(&stream_value).await {
            Ok(StreamPoll::Item(_) | StreamPoll::InternalItem(_)) => values += 1,
            Ok(StreamPoll::End) => {
                cleanup.reached_end();
                return Ok(values);
            }
            Err(error) => {
                if matches!(&error, StreamRuntimeError::Producer(_)) {
                    supervision.observe_producer_error(&stream_value);
                }
                return Err(RuntimeError::from(error));
            }
        }
    }
}

async fn execute_program(executables: Vec<LinkedExecutable>) -> crate::error::Result<RuntimeValue> {
    execute_program_at_depth(executables, 0).await
}

async fn execute_program_at_depth(
    executables: Vec<LinkedExecutable>,
    initial_depth: usize,
) -> crate::error::Result<RuntimeValue> {
    let (interpreter, _) = interpreter_with_executables(executables);
    let context = execution_context(&interpreter).with_program_call_depth_for_test(initial_depth);
    let route_addr = ExecutableAddr::service(0, 0);
    let mut heap = RequestHeap::default();

    interpreter
        .call_program_executable(
            context,
            &mut heap,
            &Env::new(),
            &route_addr,
            &route_addr,
            &Default::default(),
            Vec::new(),
        )
        .await
}

fn interpreter_with_executables(
    executables: Vec<LinkedExecutable>,
) -> (Interpreter, Arc<LinkedFileUnit>) {
    let file = Arc::new(file_with_executables(executables));
    let mut types = RuntimeTypeContext::default();
    types
        .descriptors
        .insert(producer_error_addr(), producer_error_decl());
    let program = Arc::new(EvalRuntimeProgram {
        service_id: "example.test/stream-supervision".to_string(),
        service_files: vec![file.clone()],
        packages: Vec::new(),
        service_resources: PublicationResourceTable::default(),
        spawn_routes: HashMap::new(),
        link_overlay: LinkOverlay::default(),
        types,
    });
    (
        Interpreter::with_program(program, test_runtime::runtime_factory()),
        file,
    )
}

fn assert_producer_error(error: &RuntimeError) {
    let RuntimeError::UserException(exception) = unwrap_diagnostic_source_context(error) else {
        panic!("the producer exception must be materialized into the consumer heap: {error:?}");
    };
    assert_eq!(
        exception.actual_payload_type(),
        Some(&producer_error_identity()),
        "the original typed producer error must remain catchable: {error}"
    );
}

fn assert_program_depth_error(error: &RuntimeError) {
    assert!(matches!(
        unwrap_diagnostic_source_context(error),
        RuntimeError::ResourceLimitExceeded {
            resource,
            limit: 32,
            current: 32,
            requested_delta: 1,
            ..
        } if resource == "programCallDepth"
    ));
}

struct PreparedFixture {
    interpreter: Interpreter,
    context: ProgramExecutionContext<'static>,
    caller_addr: ExecutableAddr,
    prepared: PreparedNativeStreamProducer,
    stream_runtime: StreamRuntime,
    stream_value: serde_json::Value,
}

struct DeferredFixture {
    interpreter: Interpreter,
    context: ProgramExecutionContext<'static>,
    caller_addr: ExecutableAddr,
    stream_runtime: StreamRuntime,
    stream_value: serde_json::Value,
}

async fn prepared_fixture() -> PreparedFixture {
    prepared_fixture_with(emit_then_end_producer()).await
}

async fn prepared_fixture_with(producer: LinkedExecutable) -> PreparedFixture {
    let (interpreter, file) = interpreter_with_executables(vec![
        executable(
            "caller",
            Vec::new(),
            SlotLayoutIr::default(),
            json!({
                "blocks": [{ "label": "entry", "statements": [] }],
                "statements": [],
                "expressions": []
            }),
        ),
        producer,
    ]);
    let context = execution_context(&interpreter);
    let stream_runtime = context.stream_runtime();
    let caller_addr = ExecutableAddr::service(0, 0);
    let producer_addr = ExecutableAddr::service(0, 1);
    let mut heap = RequestHeap::default();
    let mut env = Env::for_program_executable(&file.executables[0], None, 0).expect("caller env");
    let producer = StreamProducerCall {
        addr: producer_addr.clone(),
        receiver_const: None,
        producer_self: None,
        call: skiff_runtime_linked_program::CallIr {
            target: LinkedCallTarget::Executable {
                addr: producer_addr,
            },
            site: site(),
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
            actor_metadata: None,
        },
        item_type: RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "string",
            "args": []
        }))
        .expect("string stream item plan"),
    };
    let prepared = interpreter
        .prepare_native_stream_producer_arg(
            RuntimeExecutionProjection::for_context(&interpreter, &context)
                .expect("legacy execution projection"),
            context.clone(),
            &mut heap,
            &mut env,
            &caller_addr,
            &file,
            &file.executables[0],
            producer,
        )
        .await
        .expect("prepared producer");
    let stream_value = prepared.stream_value().clone();

    PreparedFixture {
        interpreter,
        context,
        caller_addr,
        prepared,
        stream_runtime,
        stream_value,
    }
}

async fn deferred_fixture(producer_executable: LinkedExecutable) -> DeferredFixture {
    let (interpreter, file) = interpreter_with_executables(vec![
        executable(
            "caller",
            Vec::new(),
            SlotLayoutIr::default(),
            json!({
                "blocks": [{ "label": "entry", "statements": [] }],
                "statements": [],
                "expressions": []
            }),
        ),
        producer_executable,
    ]);
    let context = execution_context(&interpreter);
    let stream_runtime = context.stream_runtime();
    let caller_addr = ExecutableAddr::service(0, 0);
    let producer_addr = ExecutableAddr::service(0, 1);
    let mut heap = RequestHeap::default();
    let mut env = Env::for_program_executable(&file.executables[0], None, 0).expect("caller env");
    let producer = StreamProducerCall {
        addr: producer_addr.clone(),
        receiver_const: None,
        producer_self: None,
        call: skiff_runtime_linked_program::CallIr {
            target: LinkedCallTarget::Executable {
                addr: producer_addr,
            },
            site: site(),
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
            actor_metadata: None,
        },
        item_type: RuntimeTypePlan::from_descriptor(&json!({
            "kind": "builtin",
            "name": "string",
            "args": []
        }))
        .expect("string stream item plan"),
    };
    let prepared = interpreter
        .prepare_stream_producer(
            RuntimeExecutionProjection::for_context(&interpreter, &context)
                .expect("legacy execution projection"),
            context.clone(),
            &mut heap,
            &mut env,
            &caller_addr,
            &file,
            &file.executables[0],
            producer,
        )
        .await
        .expect("deferred producer");
    let stream_value = prepared.stream_value.clone();
    let id = stream_id(&stream_value)
        .expect("prepared stream has an id")
        .to_string();
    interpreter.deferred_stream_producers.insert(id, prepared);
    DeferredFixture {
        interpreter,
        context,
        caller_addr,
        stream_runtime,
        stream_value,
    }
}

async fn assert_stream_closed(stream_runtime: &StreamRuntime, stream_value: &serde_json::Value) {
    let terminal = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        stream_runtime.next(stream_value),
    )
    .await
    .expect("stream registry must not remain pending")
    .expect("closed stream should have a terminal result");
    assert!(matches!(terminal, StreamPoll::End));
}

fn execution_context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    let actor = test_runtime::actor_context_with_trace("trace:stream-supervision");
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

fn file_with_executables(executables: Vec<LinkedExecutable>) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:stream-supervision".to_string(),
        source_ast_hash: "source:stream-supervision".to_string(),
        module_path: "stream.supervision".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: vec![producer_error_decl()],
        constants: Vec::new(),
        executables,
        external_refs: Default::default(),
    }
}

fn stream_consumer_route() -> LinkedExecutable {
    stream_consumer_route_with(1, 2)
}

fn stream_producer_route(producer_index: usize) -> LinkedExecutable {
    let mut route = executable(
        "run",
        Vec::new(),
        SlotLayoutIr::default(),
        json!({
            "blocks": [{ "label": "entry", "statements": [{ "statement": 0 }] }],
            "statements": [{ "kind": "return", "value": { "expression": 0 } }],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "site": site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, producer_index)
                            ).unwrap()
                        },
                        "args": []
                    }
                }
            ]
        }),
    );
    route.return_type = Some(stream_type(string_type()));
    route
}

fn stream_consumer_route_with(consumer_index: usize, producer_index: usize) -> LinkedExecutable {
    executable(
        "run",
        Vec::new(),
        SlotLayoutIr::default(),
        json!({
            "blocks": [{ "label": "entry", "statements": [{ "statement": 0 }] }],
            "statements": [{ "kind": "return", "value": { "expression": 1 } }],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "site": site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, producer_index)
                            ).unwrap()
                        },
                        "args": []
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, consumer_index)
                            ).unwrap()
                        },
                        "args": [{ "expression": 0 }]
                    }
                }
            ]
        }),
    )
}

fn nested_stream_consumer(nested_index: usize) -> LinkedExecutable {
    executable(
        "nestedConsume",
        vec![ParamIr {
            name: "source".to_string(),
            slot: 0,
            ty: stream_type(string_type()),
        }],
        SlotLayoutIr {
            slots: vec![slot(0, "source", "param")],
            frame_size: 1,
        },
        json!({
            "blocks": [{ "label": "entry", "statements": [{ "statement": 0 }] }],
            "statements": [{ "kind": "return", "value": { "expression": 1 } }],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "site": site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::service(0, nested_index)
                            ).unwrap()
                        },
                        "args": [{ "expression": 0 }]
                    }
                }
            ]
        }),
    )
}

fn failing_stream_consumer() -> LinkedExecutable {
    executable(
        "consume",
        vec![ParamIr {
            name: "source".to_string(),
            slot: 0,
            ty: stream_type(string_type()),
        }],
        SlotLayoutIr {
            slots: vec![slot(0, "source", "param"), slot(1, "item", "local")],
            frame_size: 2,
        },
        json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }, { "statement": 1 }]
                },
                {
                    "label": "consume",
                    "statements": [{ "statement": 2 }]
                }
            ],
            "statements": [
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 0 },
                    "body": "consume"
                },
                { "kind": "return", "value": { "expression": 2 } },
                { "kind": "return", "value": { "expression": 1 } }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 98 },
                { "kind": "literal", "value": { "kind": "null" } },
                { "kind": "loadSlot", "slot": 1 }
            ]
        }),
    )
}

fn draining_stream_consumer() -> LinkedExecutable {
    executable(
        "drain",
        vec![ParamIr {
            name: "source".to_string(),
            slot: 0,
            ty: stream_type(string_type()),
        }],
        SlotLayoutIr {
            slots: vec![slot(0, "source", "param"), slot(1, "item", "local")],
            frame_size: 2,
        },
        json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }, { "statement": 1 }]
                },
                { "label": "consume", "statements": [] }
            ],
            "statements": [
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 0 },
                    "body": "consume"
                },
                { "kind": "return", "value": { "expression": 1 } }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        }),
    )
}

fn emit_then_end_producer() -> LinkedExecutable {
    let mut executable = executable(
        "produce",
        Vec::new(),
        SlotLayoutIr::default(),
        json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }, { "statement": 1 }]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 0 }
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "first" } },
                { "kind": "literal", "value": { "kind": "string", "value": "second" } }
            ]
        }),
    );
    executable.return_type = Some(stream_type(string_type()));
    executable
}

fn emit_then_fail_producer() -> LinkedExecutable {
    let mut executable = executable(
        "produce",
        Vec::new(),
        SlotLayoutIr::default(),
        json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }, { "statement": 1 }]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 0 }
                },
                {
                    "kind": "expr",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "first" } },
                { "kind": "literal", "value": { "kind": "string", "value": "late producer error" } },
                {
                    "kind": "construct",
                    "typeRef": {
                        "kind": "address",
                        "addr": serde_json::to_value(producer_error_addr()).unwrap()
                    },
                    "fields": {
                        "message": { "expression": 1 }
                    }
                },
                {
                    "kind": "throw",
                    "site": site(),
                    "value": { "expression": 2 },
                    "payloadType": {
                        "kind": "address",
                        "addr": serde_json::to_value(producer_error_addr()).unwrap()
                    }
                }
            ]
        }),
    );
    executable.return_type = Some(stream_type(string_type()));
    executable
}

fn executable(
    symbol: &str,
    params: Vec<ParamIr>,
    slots: SlotLayoutIr,
    body: serde_json::Value,
) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params,
        return_type: None,
        self_type: None,
        slots,
        may_suspend: false,
        body: serde_json::from_value::<LinkedExecutableBody>(body)
            .expect("test executable body must decode"),
    }
}

fn slot(index: usize, name: &str, kind: &str) -> SlotIr {
    SlotIr {
        index,
        name: name.to_string(),
        kind: kind.to_string(),
    }
}

fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

fn stream_type(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Stream".to_string(),
        args: vec![item],
    }
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn producer_error_addr() -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    }
}

fn producer_error_decl() -> TypeDeclIr {
    TypeDeclIr {
        name: "ProducerError".to_string(),
        descriptor: LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([("message".to_string(), string_type())]),
        },
        ..TypeDeclIr::default()
    }
}

fn producer_error_identity() -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: producer_error_addr(),
            type_arguments: Vec::new(),
        },
    ))
}

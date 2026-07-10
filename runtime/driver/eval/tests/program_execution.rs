use std::{
    collections::{BTreeMap, HashMap},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use serde_json::{json, Value};
use skiff_runtime_boundary::date_value;
use skiff_runtime_boundary::json::RuntimeBoundaryCodec;
use skiff_runtime_boundary::plan::BoundaryUse;
use skiff_runtime_boundary::stream::STREAM_ID_KEY;
use skiff_runtime_boundary::type_descriptor::{
    RuntimeTypeNode, RuntimeTypePlan, RuntimeTypePlanDescriptorExt,
};
use skiff_runtime_boundary::{
    binary::{decode_payload, encode_payload, encode_payload_plan},
    payload::PayloadBoundary,
};
use skiff_runtime_host::eval_capability_adapter;
use skiff_runtime_model::{
    error::WirePayload,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
};
use skiff_runtime_request::cancellation::CancellationToken;
use tokio::time::sleep;

use super::*;
use crate::eval::InterpreterEnv as Env;
use skiff_artifact_model::{builtin_receiver_op_by_name, DbMetadataIr, PublicationResourceRef};
use skiff_runtime_linked_program::{LoadedPublicationResource, PublicationResourceTable};

const PROTOCOL_OUTBOUND: &str =
    "skiff-protocol-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const BUILD_OUTBOUND: &str = "skiff-service-build-v1:sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const STD_HTTP_HEADER_TYPE_INDEX: usize = 0;
const STD_HTTP_QUERY_PARAM_TYPE_INDEX: usize = 1;
const STD_HTTP_REQUEST_TYPE_INDEX: usize = 2;
const STD_HTTP_RESPONSE_TYPE_INDEX: usize = 3;
const STD_HTTP_RESPONSE_STREAM_EVENT_TYPE_INDEX: usize = 4;
const STD_HTTP_CLIENT_REQUEST_TYPE_INDEX: usize = 5;
const STD_HTTP_CLIENT_RESPONSE_TYPE_INDEX: usize = 6;
const STD_HTTP_CLIENT_STREAM_HANDLE_TYPE_INDEX: usize = 7;
const STD_HTTP_SSE_EVENT_TYPE_INDEX: usize = 8;

fn runtime_factory() -> crate::eval::capabilities::EvalRuntimeFactory {
    eval_capability_adapter::runtime_factory()
}

use crate::{
    eval::error::{
        unwrap_diagnostic_source_context, BudgetReason, RuntimeError, TypeIdentity, UserException,
    },
    eval::program::{
        anonymous_type_decl, types::PackageSymbolKey, CallIr, ConstAddr, ConstIr, ExecutableAddr,
        ExecutableKind, ExprRefIr, FileAddr, FileDeclarations, FileLinkTargets, GatewayConfig,
        LinkOverlay, LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr,
        LinkedFileUnit, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef, LiteralIr,
        MetadataValue, NativeTarget, PackageUnit, ParamIr, ResolvedSymbol, RuntimeActivation,
        RuntimeProgram, RuntimeTypeContext, ServiceDependencyConstraint,
        ServiceDependencySymbolRef, ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, StmtRefIr,
        TypeAddr, TypeDeclIr, UnitAddr,
    },
    eval::{
        capabilities::{OutboundServiceContext, StreamPoll, StreamRuntime, TypedStreamSink},
        native_capability::project_runtime_native_capability_context,
        native_invocation::resolve_runtime_native_invocation,
        program_execution::{
            executable_type_param_names, OwnedProgramExecutionContext, ProgramExecutionInput,
        },
        program_invocation::{ProgramInvocationContext, ProgramInvocationInput},
        service_dispatch::outbound_control_and_payload_for_test,
        TestEffectDouble,
    },
    type_descriptor::{PlanContext, RuntimeTypePlanLinkedExt},
};
use skiff_runtime_native::dispatch::NativeDispatch;

fn account_lookup_symbol() -> ServiceDependencySymbolRef {
    ServiceDependencySymbolRef {
        dependency_ref: "account".to_string(),
        operation: account_lookup_operation_ref(),
    }
}

const ACCOUNT_LOOKUP_OPERATION_ABI_ID: &str = "operation:account:lookup";
const REMOTE_READER_INTERFACE_ABI_ID: &str = "svc.main.RemoteReader";
const REMOTE_READER_METHOD_ABI_ID: &str = "method:svc.main.RemoteReader.read";
const REMOTE_READER_PUBLIC_INSTANCE: &str = "reader";
const REMOTE_READER_PUBLIC_PATH: &str = "reader.read";
const REMOTE_READER_OPERATION_ABI_ID: &str = "operation:account:reader.read";

fn account_lookup_operation_ref() -> skiff_artifact_model::OperationAbiRef {
    skiff_artifact_model::OperationAbiRef {
        operation_abi_id: ACCOUNT_LOOKUP_OPERATION_ABI_ID.to_string(),
        kind: skiff_artifact_model::PublicationOperationKind::PublicFunction,
        public_path: "lookup".to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: "lookup".to_string(),
    }
}

fn remote_reader_interface_ref() -> skiff_artifact_model::InterfaceInstantiationRef {
    skiff_artifact_model::InterfaceInstantiationRef {
        interface_abi_id: REMOTE_READER_INTERFACE_ABI_ID.to_string(),
        canonical_type_args: Vec::new(),
    }
}

fn remote_reader_interface_json() -> serde_json::Value {
    json!({
        "interfaceAbiId": REMOTE_READER_INTERFACE_ABI_ID,
        "canonicalTypeArgs": []
    })
}

fn remote_reader_interface_method_target() -> serde_json::Value {
    json!({
        "kind": "interfaceMethod",
        "interface": remote_reader_interface_json(),
        "methodAbiId": REMOTE_READER_METHOD_ABI_ID,
        "slot": 0
    })
}

fn remote_reader_operation_ref() -> skiff_artifact_model::OperationAbiRef {
    skiff_artifact_model::OperationAbiRef {
        operation_abi_id: REMOTE_READER_OPERATION_ABI_ID.to_string(),
        kind: skiff_artifact_model::PublicationOperationKind::PublicInstanceMethod,
        public_path: REMOTE_READER_PUBLIC_PATH.to_string(),
        public_instance_key: Some(REMOTE_READER_PUBLIC_INSTANCE.to_string()),
        interface: Some(remote_reader_interface_ref()),
        method_abi_id: Some(REMOTE_READER_METHOD_ABI_ID.to_string()),
        display_name: REMOTE_READER_PUBLIC_PATH.to_string(),
    }
}

fn remote_reader_symbol() -> ServiceDependencySymbolRef {
    ServiceDependencySymbolRef {
        dependency_ref: "account".to_string(),
        operation: remote_reader_operation_ref(),
    }
}

fn linked_builtin_type(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn receiver_builtin_target(root: &str, method: &str) -> serde_json::Value {
    let op = builtin_receiver_op_by_name(root, method).expect("receiver op must exist");
    json!({
        "kind": "receiverBuiltin",
        "op": serde_json::to_value(op).unwrap()
    })
}

fn local_const_receiver_target(executable_index: usize) -> serde_json::Value {
    json!({
        "kind": "localConstReceiverExecutable",
        "constAddr": {
            "unit": { "kind": "service" },
            "file": { "kind": "loadedFileIndex", "value": 0 },
            "constIndex": 0
        },
        "executableAddr": serde_json::to_value(ExecutableAddr::service(0, executable_index)).unwrap(),
        "methodAbiId": "method:svc.main.ManagedLlm.sendChat",
        "receiverCallAbi": "explicitSelfFirst"
    })
}

#[tokio::test]
async fn runtime_program_executes_route_by_executable_addr() {
    let program = Arc::new(program_with_executable(run_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("program route should execute");

    assert_eq!(
        value,
        json!({
            "label": "Ada!",
            "copy": "Ada!"
        })
    );
}

#[tokio::test]
async fn runtime_program_route_skips_explicit_self_request_parameter() {
    let program = Arc::new(program_with_executable(explicit_self_route_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("explicit self route should execute with request args only");

    assert_eq!(value, json!("Ada"));
}

#[test]
fn linked_ir_rejects_legacy_provider_call_target() {
    let error = serde_json::from_value::<LinkedExprIr>(json!({
        "kind": "call",
        "call": {
            "target": {
                "kind": "provider",
                "target": {
                    "providerId": "test-provider",
                    "capability": "test",
                    "operation": "test.echo"
                }
            },
            "args": []
        }
    }))
    .expect_err("legacy provider call target should fail closed")
    .to_string();

    assert!(
        error.contains("unknown variant `provider`"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_executes_receiver_builtin_call() {
    let mut executable = run_executable();
    executable.body.expressions.push(expression(json!({
        "kind": "call",
        "call": {
            "target": receiver_builtin_target("string", "concat"),
            "args": [
                { "expression": 0 },
                { "expression": 1 }
            ]
        }
    })));
    executable.body.statements[0] = statement(json!({
        "kind": "return",
        "value": { "expression": 5 }
    }));
    let program = Arc::new(program_with_executable(executable));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("receiver builtin call should execute");

    assert_eq!(value, json!("Ada!"));
}

#[tokio::test]
async fn runtime_program_executes_local_const_receiver_executable_call() {
    let mut run = run_executable();
    run.return_type = Some(linked_builtin_type("Json"));
    run.body.expressions.push(expression(json!({
        "kind": "call",
        "call": {
            "target": local_const_receiver_target(1),
            "args": []
        }
    })));
    run.body.statements[0] = statement(json!({
        "kind": "return",
        "value": { "expression": 5 }
    }));
    let method = read_self_executable();
    let mut program = program_with_executables(vec![run, method]);
    Arc::make_mut(&mut program.service_files[0])
        .constants
        .push(ConstIr {
            name: "managedLlmService".to_string(),
            ty: linked_builtin_type("Json"),
            body: executable_body(json!({
                "blocks": [
                    {
                        "label": "entry",
                        "statements": [
                            { "statement": 0 }
                        ]
                    }
                ],
                "statements": [
                    {
                        "kind": "return",
                        "value": { "expression": 0 }
                    }
                ],
                "expressions": [
                    {
                        "kind": "mapLiteral",
                        "entries": {
                            "name": { "expression": 1 }
                        }
                    },
                    {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "Ada" }
                    }
                ]
            })),
            source_span: None,
        });
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "ignored");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("local const receiver executable call should execute");

    assert_eq!(value, json!({ "name": "Ada" }));
}

#[tokio::test]
async fn runtime_program_route_receiver_const_injects_self() {
    let mut program = program_with_executables(vec![read_self_executable()]);
    program.operation_receivers.insert(
        "run".to_string(),
        ConstAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            const_index: 0,
        },
    );
    Arc::make_mut(&mut program.service_files[0])
        .constants
        .push(ConstIr {
            name: "managedLlmService".to_string(),
            ty: linked_builtin_type("Json"),
            body: executable_body(json!({
                "blocks": [
                    {
                        "label": "entry",
                        "statements": [
                            { "statement": 0 }
                        ]
                    }
                ],
                "statements": [
                    {
                        "kind": "return",
                        "value": { "expression": 0 }
                    }
                ],
                "expressions": [
                    {
                        "kind": "mapLiteral",
                        "entries": {
                            "name": { "expression": 1 }
                        }
                    },
                    {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "Ada" }
                    }
                ]
            })),
            source_span: None,
        });
    let receiver_const = program.operation_receivers.get("run").cloned();
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.receiver_const = receiver_const;

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("route receiver const should inject self");

    assert_eq!(value, json!({ "name": "Ada" }));
}

#[tokio::test]
async fn runtime_program_consumes_local_const_receiver_stream_producer() {
    let mut program = program_with_executables(vec![
        local_const_receiver_stream_first_item_route_executable(),
        local_const_receiver_stream_producer_executable(),
    ]);
    Arc::make_mut(&mut program.service_files[0])
        .constants
        .push(ConstIr {
            name: "managedLlmService".to_string(),
            ty: linked_builtin_type("Json"),
            body: executable_body(json!({
                "blocks": [
                    {
                        "label": "entry",
                        "statements": [
                            { "statement": 0 }
                        ]
                    }
                ],
                "statements": [
                    {
                        "kind": "return",
                        "value": { "expression": 0 }
                    }
                ],
                "expressions": [
                    {
                        "kind": "mapLiteral",
                        "entries": {
                            "name": { "expression": 1 }
                        }
                    },
                    {
                        "kind": "literal",
                        "value": { "kind": "string", "value": "Ada" }
                    }
                ]
            })),
            source_span: None,
        });
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("local const receiver stream producer should execute");

    assert_eq!(value, json!("Ada"));
}

#[tokio::test]
async fn runtime_program_executes_receiver_builtin_mutation_and_index_assignment() {
    let program = Arc::new(program_with_executable(receiver_builtin_array_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("receiver builtin mutation should execute");

    assert_eq!(value, json!(["z", "b"]));
}

#[tokio::test]
async fn runtime_program_executes_bytes_natives_without_json_registry() {
    let program = Arc::new(program_with_executable(bytes_concat_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("bytes natives should execute in RuntimeProgram");

    assert_eq!(value, json!("hello"));
}

#[tokio::test]
async fn runtime_program_executes_time_sleep_native_without_json_registry() {
    let program = Arc::new(program_with_executable(time_sleep_executable(20)));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let started_at = std::time::Instant::now();
    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.time.sleep native should execute in RuntimeProgram");

    assert_eq!(value, json!(null));
    assert!(started_at.elapsed() >= Duration::from_millis(10));
}

#[tokio::test]
async fn runtime_program_time_sleep_negative_returns_immediately() {
    let program = Arc::new(program_with_executable(time_sleep_executable(-1)));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("negative std.time.sleep should not wait")
    .expect("negative std.time.sleep native should execute in RuntimeProgram");

    assert_eq!(value, json!(null));
}

#[tokio::test]
async fn runtime_program_time_sleep_observes_cancellation() {
    let program = Arc::new(program_with_executable(time_sleep_executable(100)));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");
    let cancellation = frame.cancellation.clone();

    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(15)).await;
        cancellation.cancel();
    });
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep should observe cancellation")
    .expect_err("cancelled std.time.sleep should fail");
    cancel_task
        .await
        .expect("cancellation task should complete");

    assert!(matches!(error, RuntimeError::Cancelled));
}

#[tokio::test]
async fn runtime_program_time_sleep_observes_deadline() {
    let program = Arc::new(program_with_executable(time_sleep_executable(100)));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.execution_budget = std::sync::Arc::new(crate::execution_budget::ExecutionBudget::new(
        crate::execution_budget::ExecutionBudgetConfig::runtime_default(),
        Some(std::time::Instant::now() + Duration::from_millis(15)),
    ));

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep should observe request deadline")
    .expect_err("expired std.time.sleep should fail");

    assert!(matches!(
        error,
        RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::DeadlineExceeded,
            ..
        }
    ));
}

#[tokio::test]
async fn runtime_program_bytes_native_args_use_native_signature() {
    let program = Arc::new(program_with_executable(
        bytes_from_utf8_invalid_arg_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.bytes.fromUtf8 arg should be validated by native signature");

    let message = error.to_string();
    assert!(message.contains("std.bytes.fromUtf8 argument 0"));
    assert!(message.contains("expected runtime string"));
}

fn runtime_scalar_json(value: &RuntimeValue) -> Option<Value> {
    match value {
        RuntimeValue::Null => Some(Value::Null),
        RuntimeValue::Bool(value) => Some(Value::Bool(*value)),
        RuntimeValue::Number(value) => {
            if value.is_finite()
                && value.fract() == 0.0
                && *value >= i64::MIN as f64
                && *value <= i64::MAX as f64
            {
                return Some(Value::Number(serde_json::Number::from(*value as i64)));
            }
            serde_json::Number::from_f64(*value).map(Value::Number)
        }
        RuntimeValue::String(value) => Some(Value::String(value.clone())),
        RuntimeValue::Date(ms) => date_value::format_epoch_millis(*ms, "test runtime scalar Date")
            .ok()
            .map(Value::String),
        RuntimeValue::ActorRef(_) | RuntimeValue::Heap(_) => None,
    }
}

#[tokio::test]
async fn runtime_program_executes_package_function_call() {
    let service_addr = ExecutableAddr::service(0, 0);
    let package_addr = ExecutableAddr::package(0, 0, 0);
    let mut program = program_with_service_and_package_executables(
        package_call_executable(),
        package_echo_executable(),
    );
    program.packages = vec![Arc::new(package_unit("example.com/pkg"))];
    program
        .link_overlay
        .package_slots_by_id
        .insert("example.com/pkg".to_string(), 0);
    program.link_overlay.symbols.insert(
        "package[0]::pkg.echo".to_string(),
        ResolvedSymbol::Executable { addr: package_addr },
    );
    program
        .routes
        .insert("svc.main.run".to_string(), service_addr.clone());
    program.operations.insert("run".to_string(), service_addr);

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("package function call should execute through package overlay key");

    assert_eq!(value, json!("Ada from package"));
}

#[tokio::test]
async fn runtime_program_executes_package_function_call_by_package_id_ref() {
    let mut program = program_with_service_and_package_executables(
        package_call_executable_with_package_ref(json!({
            "kind": "packageId",
            "packageId": "example.com/pkg"
        })),
        package_echo_executable(),
    );
    program.packages = vec![Arc::new(package_unit("example.com/pkg"))];
    program
        .link_overlay
        .package_slots_by_id
        .insert("example.com/pkg".to_string(), 0);
    program.link_overlay.symbols.insert(
        "package[0]::pkg.echo".to_string(),
        ResolvedSymbol::Executable {
            addr: ExecutableAddr::package(0, 0, 0),
        },
    );

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("package id ref should resolve through link overlay");

    assert_eq!(value, json!("Ada from package"));
}

#[tokio::test]
async fn runtime_program_executes_package_function_call_by_dependency_ref() {
    let mut program = program_with_service_and_package_executables(
        package_call_executable_with_package_ref(json!({
            "kind": "dependency",
            "dependencyRef": "mongo"
        })),
        package_echo_executable(),
    );
    program.packages = vec![Arc::new(package_unit("example.com/pkg"))];
    program
        .link_overlay
        .package_slots_by_dependency_ref
        .insert("mongo".to_string(), 0);
    program.link_overlay.symbols.insert(
        "package[0]::pkg.echo".to_string(),
        ResolvedSymbol::Executable {
            addr: ExecutableAddr::package(0, 0, 0),
        },
    );

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("dependency ref should resolve through link overlay");

    assert_eq!(value, json!("Ada from package"));
}

#[tokio::test]
async fn runtime_program_substitutes_package_generic_type_args_for_native_wrapper() {
    let mut program = program_with_service_and_package_executables(
        package_generic_json_decode_call_executable(),
        generic_json_decode_native_wrapper_executable(),
    );
    program.packages = vec![Arc::new(package_unit("skiff.run/std"))];
    program
        .link_overlay
        .package_slots_by_id
        .insert("skiff.run/std".to_string(), 0);
    program.link_overlay.symbols.insert(
        "package[0]::json.decode".to_string(),
        ResolvedSymbol::Executable {
            addr: ExecutableAddr::package(0, 0, 0),
        },
    );

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("generic package native wrapper should substitute call type args");

    assert_eq!(value, json!({ "name": "Ada" }));
}

#[tokio::test]
async fn runtime_program_substitutes_generic_type_args_for_config_native_wrapper() {
    let mut program = program_with_service_and_package_executables(
        package_generic_config_require_call_executable(),
        generic_config_require_wrapper_executable(),
    );
    program.packages = vec![Arc::new(package_unit("example.com/config"))];

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.package_configs = vec![RuntimeConfigView::from_value(json!({
        "sessionSecret": "package-secret"
    }))];

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("generic config wrapper should substitute call type args");

    assert_eq!(value, json!("package-secret"));
}

#[tokio::test]
async fn runtime_program_json_native_direct_type_args_use_native_signature() {
    let program = Arc::new(program_with_executable(
        json_native_direct_type_args_with_nullable_json_object_return_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("direct std.json native type args should not use caller return type");

    assert_eq!(value, json!({ "name": "Ada" }));
}

#[tokio::test]
async fn runtime_program_json_decode_native_missing_type_args_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_decode_native_missing_type_args_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.json.decode without direct typeArgs should fail closed");

    let payload = error.payload();
    assert_eq!(payload.code, "InvalidArtifact");
    let message = payload.message;
    assert!(message.contains("std.json.decode"));
    assert!(message.contains("typeArgs[0]"));
}

#[tokio::test]
async fn runtime_program_std_native_without_binding_key_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_decode_native_missing_binding_key_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std native without bindingKey should fail closed");

    let message = match error {
        RuntimeError::InvalidArtifact(message) => message,
        other => panic!("unexpected error: {other}"),
    };
    assert!(message.contains("std.json.decode"));
    assert!(message.contains("missing artifact bindingKey"));
}

#[tokio::test]
async fn runtime_program_json_encode_native_missing_type_args_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_encode_native_missing_type_args_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.json.encode without direct typeArgs should fail closed");

    let payload = error.payload();
    assert_eq!(payload.code, "InvalidArtifact");
    let message = payload.message;
    assert!(message.contains("std.json.encode"));
    assert!(message.contains("typeArgs[0]"));
}

#[tokio::test]
async fn runtime_program_json_native_missing_t0_type_arg_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_decode_native_missing_t0_type_arg_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.json.decode without T0 typeArg should fail closed");

    let payload = error.payload();
    assert_eq!(payload.code, "InvalidArtifact");
    let message = payload.message;
    assert!(message.contains("std.json.decode"));
    assert!(message.contains("unexpected generic typeArgs[1]"));
}

#[tokio::test]
async fn runtime_program_json_native_unresolved_type_arg_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_decode_native_unresolved_type_arg_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.json.decode with unresolved direct typeArgs should fail closed");

    let payload = error.payload();
    assert_eq!(payload.code, "InvalidArtifact");
    let message = payload.message;
    assert!(message.contains("std.json.decode"));
    assert!(message.contains("unresolved typeArgs[0]"));
}

#[tokio::test]
async fn runtime_program_json_native_target_metadata_fails_invalid_artifact() {
    let program = Arc::new(program_with_executable(
        json_decode_native_target_metadata_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("known std native target metadata should fail closed at execution");

    let message = match error {
        RuntimeError::InvalidArtifact(message) => message,
        other => panic!("unexpected error: {other}"),
    };
    assert!(message.contains("std.json.decode"));
    assert!(message.contains("target metadata is not supported"));
}

#[tokio::test]
async fn runtime_program_telemetry_native_uses_registered_signature_dispatch() {
    let program = Arc::new(program_with_executable(
        telemetry_emit_native_direct_call_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.telemetry.emit should dispatch through registered native signature");

    assert_eq!(value, Value::Null);
}

#[tokio::test]
async fn runtime_program_resource_text_reads_service_resource() {
    let mut program = program_with_executable(resource_text_native_executable("prompts/system.md"));
    program.service_resources = resource_table("prompts/system.md", b"service text");
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.resource.text should read service resources");

    assert_eq!(value, json!("service text"));
}

#[tokio::test]
async fn runtime_program_resource_exists_returns_false_for_invalid_and_missing_paths() {
    for path in ["./bad", "missing.txt"] {
        let program = Arc::new(program_with_executable(resource_exists_native_executable(
            path,
        )));
        let interpreter = Interpreter::with_program(program, runtime_factory());
        let frame = test_invocation("svc.main.run");

        let value = execute_test_program_route(&interpreter, &frame)
            .await
            .expect("std.resource.exists should not throw for invalid or missing paths");

        assert_eq!(value, json!(false), "path {path}");
    }
}

#[tokio::test]
async fn runtime_program_resource_text_missing_path_throws_resource_error() {
    let program = Arc::new(program_with_executable(resource_text_native_executable(
        "missing.txt",
    )));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for missing resources");

    assert_resource_error(&error, "missing.txt");
}

#[tokio::test]
async fn runtime_program_resource_text_invalid_path_throws_resource_error() {
    let program = Arc::new(program_with_executable(resource_text_native_executable(
        "./bad",
    )));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for invalid paths");

    assert_resource_error(&error, "./bad");
}

#[tokio::test]
async fn runtime_program_resource_text_invalid_utf8_throws_resource_error() {
    let mut program = program_with_executable(resource_text_native_executable("bad.txt"));
    program.service_resources = resource_table("bad.txt", &[0xff, 0xfe]);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for invalid UTF-8");

    assert_resource_error(&error, "bad.txt");
}

#[tokio::test]
async fn runtime_program_resource_json_syntax_error_uses_json_decode_error_shape() {
    let mut program = program_with_executable(resource_json_object_native_executable("bad.json"));
    program.service_resources = resource_table("bad.json", b"{");
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should map syntax errors to std.json.DecodeError");

    assert_resource_json_decode_error(&error, "bad.json");
}

#[tokio::test]
async fn runtime_program_resource_json_type_error_uses_json_decode_error_shape() {
    let mut program = program_with_executable(resource_json_object_native_executable("bad.json"));
    program.service_resources = resource_table("bad.json", b"[]");
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should map type errors to std.json.DecodeError");

    assert_resource_json_decode_error(&error, "bad.json");
}

#[tokio::test]
async fn runtime_program_resource_json_invalid_utf8_throws_resource_error() {
    let mut program = program_with_executable(resource_json_object_native_executable("bad.json"));
    program.service_resources = resource_table("bad.json", &[0xff, 0xfe]);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should throw ResourceError for invalid UTF-8");

    assert_resource_error(&error, "bad.json");
}

#[tokio::test]
async fn runtime_program_resource_package_call_site_reads_package_resource() {
    let mut program = program_with_service_and_package_executables(
        service_calls_package_resource_text_executable(),
        resource_text_native_executable("prompts/system.md"),
    );
    program.packages = vec![Arc::new(package_unit("example.com/pkg"))];
    program.service_resources = resource_table("prompts/system.md", b"service text");
    program.package_resources = vec![resource_table("prompts/system.md", b"package text")];
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("package call-site owner should read package resources");

    assert_eq!(value, json!("package text"));
}

#[tokio::test]
async fn runtime_program_config_reads_called_package_slot_scope() {
    let mut program = program_with_executable(run_executable());
    program.packages = vec![
        Arc::new(package_unit("skiff.run/track")),
        Arc::new(package_unit("skiff.run/http-session")),
    ];
    program.package_files = vec![
        vec![Arc::new(package_file_unit(
            "file:track",
            "track.main",
            package_call_config_reader_executable(),
        ))],
        vec![Arc::new(package_file_unit(
            "file:http-session",
            "httpSession.main",
            config_require_string_executable("sessionSecret"),
        ))],
    ];
    program.package_resources = vec![Default::default(), Default::default()];
    let target = "package.skiff.run%2Ftrack.record";
    program
        .routes
        .insert(target.to_string(), ExecutableAddr::package(0, 0, 0));
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation(target);
    frame.route_addr = ExecutableAddr::package(0, 0, 0);
    frame.config = RuntimeConfigView::from_value(json!({
        "sessionSecret": "service-secret"
    }));
    frame.package_configs = vec![
        RuntimeConfigView::from_value(json!({ "sessionSecret": "track-secret" })),
        RuntimeConfigView::from_value(json!({ "sessionSecret": "http-session-secret" })),
    ];

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("package-to-package config read should execute");

    assert_eq!(value, json!("http-session-secret"));
}

#[test]
fn runtime_program_collects_type_params_from_structural_return_types() {
    let mut executable = run_executable();
    executable.params[0].ty = LinkedTypeRef::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: "svc.main".to_string(),
            symbol: "Thread".to_string(),
        },
    };
    executable.return_type = Some(LinkedTypeRef::Record {
        fields: BTreeMap::from([(
            "value".to_string(),
            LinkedTypeRef::TypeParam {
                name: "U".to_string(),
            },
        )]),
    });

    assert_eq!(
        executable_type_param_names(&executable),
        vec!["U".to_string()]
    );
}

#[test]
fn runtime_program_db_insert_one_decodes_business_json_through_ordinary_result_plan() {
    let program = program_with_executable(run_executable());
    let addr = ExecutableAddr::service(0, 0);
    let result_type = LinkedTypeRef::Record {
        fields: BTreeMap::from([
            (
                "id".to_string(),
                LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            ),
            (
                "title".to_string(),
                LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            ),
        ]),
    };
    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(&result_type, &PlanContext::new(&image, &addr))
        .expect("ordinary DB insert result plan should build");
    let mut heap = RequestHeap::default();

    let decoded = RuntimeBoundaryCodec::new(&plan, BoundaryUse::DbResultDecode, "db test result")
        .from_wire_json(&json!({ "id": "thread-1", "title": "First" }), &mut heap)
        .expect("ordinary DB insert result should decode");

    let RuntimeValue::Heap(handle) = decoded else {
        panic!("expected decoded insert result object");
    };
    let fields = match heap.get(handle).expect("decoded object should exist") {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected decoded object, got {other:?}"),
    };
    assert_eq!(
        fields.get("id"),
        Some(&RuntimeValue::String("thread-1".to_string()))
    );
    assert_eq!(
        fields.get("title"),
        Some(&RuntimeValue::String("First".to_string()))
    );
}

#[test]
fn runtime_program_db_insert_one_decodes_db_object_symbol_result_plan() {
    let mut program = program_with_executable(run_executable());
    let addr = ExecutableAddr::service(0, 0);
    let object_type_addr = TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    program.types.descriptors.insert(
        object_type_addr.clone(),
        anonymous_type_decl(
            "DbObject",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    (
                        "id".to_string(),
                        LinkedTypeRef::Native {
                            name: "string".to_string(),
                            args: Vec::new(),
                        },
                    ),
                    (
                        "title".to_string(),
                        LinkedTypeRef::Native {
                            name: "string".to_string(),
                            args: Vec::new(),
                        },
                    ),
                ]),
            },
        ),
    );
    program.types.exported_types.insert_service(
        crate::eval::program::types::ServiceSymbolKey::new("svc.main", "Thread"),
        object_type_addr,
    );
    let result_type = LinkedTypeRef::DbObjectSymbol {
        symbol: ServiceSymbolRef {
            module_path: "svc.main".to_string(),
            symbol: "Thread".to_string(),
        },
    };
    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(&result_type, &PlanContext::new(&image, &addr))
        .expect("DB object result plan should resolve the attached record type");
    let mut heap = RequestHeap::default();

    let decoded = RuntimeBoundaryCodec::new(&plan, BoundaryUse::DbResultDecode, "db test result")
        .from_wire_json(&json!({ "id": "thread-1", "title": "First" }), &mut heap)
        .expect("DB object insert result should decode");

    let RuntimeValue::Heap(handle) = decoded else {
        panic!("expected decoded insert result object");
    };
    let fields = match heap.get(handle).expect("decoded object should exist") {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected decoded object, got {other:?}"),
    };
    assert_eq!(
        fields.get("id"),
        Some(&RuntimeValue::String("thread-1".to_string()))
    );
    assert_eq!(
        fields.get("title"),
        Some(&RuntimeValue::String("First".to_string()))
    );
}

#[test]
fn runtime_program_decodes_nested_anonymous_record_result_plan_with_nullable_nested_record() {
    let program = program_with_executable(run_executable());
    let addr = ExecutableAddr::service(0, 0);
    let nested_record = LinkedTypeRef::Record {
        fields: BTreeMap::from([(
            "displayName".to_string(),
            LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        )]),
    };
    let result_type = LinkedTypeRef::Record {
        fields: BTreeMap::from([(
            "profile".to_string(),
            LinkedTypeRef::Nullable {
                inner: Box::new(nested_record),
            },
        )]),
    };
    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(&result_type, &PlanContext::new(&image, &addr))
        .expect("nested anonymous result plan should build");
    let mut heap = RequestHeap::default();

    let decoded = RuntimeBoundaryCodec::new(&plan, BoundaryUse::DbResultDecode, "db test result")
        .from_wire_json(&json!({ "profile": { "displayName": "Ada" } }), &mut heap)
        .expect("nested anonymous result should decode");

    let RuntimeValue::Heap(handle) = decoded else {
        panic!("expected decoded result object");
    };
    let fields = match heap.get(handle).expect("decoded object should exist") {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected decoded object, got {other:?}"),
    };
    let profile_handle = match fields.get("profile") {
        Some(RuntimeValue::Heap(handle)) => *handle,
        other => panic!("expected profile object, got {other:?}"),
    };
    let profile_fields = match heap
        .get(profile_handle)
        .expect("decoded profile object should exist")
    {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected decoded profile object, got {other:?}"),
    };
    assert_eq!(
        profile_fields.get("displayName"),
        Some(&RuntimeValue::String("Ada".to_string()))
    );

    let mut null_heap = RequestHeap::default();
    let decoded_null =
        RuntimeBoundaryCodec::new(&plan, BoundaryUse::DbResultDecode, "db test result")
            .from_wire_json(&json!({ "profile": null }), &mut null_heap)
            .expect("nullable nested record should decode null");
    let RuntimeValue::Heap(null_handle) = decoded_null else {
        panic!("expected decoded result object for null profile");
    };
    let null_fields = match null_heap
        .get(null_handle)
        .expect("decoded null-profile object should exist")
    {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected decoded object, got {other:?}"),
    };
    assert_eq!(null_fields.get("profile"), Some(&RuntimeValue::Null));
}

#[tokio::test]
async fn runtime_program_declares_parameter_from_slot_def() {
    let program = Arc::new(program_with_executable(parameter_slot_def_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_string_arg(&mut frame, "input", "Ada");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("parameter slot def should be used when explicit parameter slots are absent");

    assert_eq!(value, json!("Ada"));
}

#[tokio::test]
async fn runtime_program_executes_for_in_and_value_block() {
    let program = Arc::new(program_with_executable(for_in_value_block_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("forIn and valueBlock should execute");

    assert_eq!(value, json!("abc"));
}

/// Regression for the worker-thread stack overflow that crash-looped the runtime
/// on the deep LLM streaming chain. Each forwarding producer binds the next
/// producer to a value and re-emits its items
/// (`let s = produce_next(); for item in s { emit item }`), so consuming the
/// chain used to nest the producer/consumer poll-chain (`run_stream_producer` ->
/// producer body -> `exec_program_stream_for_in` -> ...) many levels deep within
/// a single tokio task.
///
/// Root fix: each `Stream` producer now runs in its own `tokio::spawn`ed task
/// (see `spawn_stream_producer` in `eval/program_stream.rs`); the consumer
/// only polls the bounded channel, so native-stack depth is constant regardless
/// of producer nesting. This test drives a chain far deeper than the production
/// LLM path (~8) and asserts all items propagate. The companion test
/// `runtime_program_deeply_nested_stream_producers_are_stack_depth_independent`
/// runs the same chain on a deliberately small (1 MiB) stack to prove the fix
/// removed the stack-depth dependence (it overflowed and aborted the process on
/// the pre-fix co-driven code). A stack overflow aborts the whole process rather
/// than unwinding, so a stack test can only assert the positive (completion).
///
/// `SKIFF_NESTED_PRODUCER_DEPTH` / `SKIFF_NESTED_PRODUCER_STACK_KIB` override the
/// depth and stack for manual before/after stack characterization.
#[test]
fn runtime_program_deeply_nested_stream_producers_run_to_completion() {
    // 40 deep is ~5x the production LLM chain.
    let depth: usize = std::env::var("SKIFF_NESTED_PRODUCER_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40)
        .max(2);
    let stack_bytes: usize = std::env::var("SKIFF_NESTED_PRODUCER_STACK_KIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(|kib: usize| kib * 1024)
        .unwrap_or(crate::config::RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES);

    let handle = std::thread::Builder::new()
        .name("nested-stream-producer-test".to_string())
        .stack_size(stack_bytes)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime should build");
            runtime.block_on(async move {
                // executables[0]        = route consuming produce_0() and aggregating
                // executables[1..depth] = forwarding producers (deferred path)
                // executables[depth]    = leaf producer emitting "a","b","c"
                let mut executables = vec![local_stream_aggregate_route_executable()];
                for level in 1..depth {
                    executables.push(forwarding_string_stream_producer_executable(level + 1));
                }
                executables.push(local_string_stream_producer_executable());

                let program = Arc::new(program_with_executables(executables));
                let interpreter = Interpreter::with_program(program, runtime_factory());
                let frame = test_invocation("svc.main.run");

                let value = execute_test_program_route(&interpreter, &frame)
                    .await
                    .expect(
                        "deeply nested stream producer chain should run without stack overflow",
                    );

                // Each forwarding level passes every item through unchanged, so
                // the leaf's "a","b","c" must arrive intact at the aggregator.
                assert_eq!(value, json!("abc"));
            });
        })
        .expect("test worker thread should spawn");

    handle
        .join()
        .expect("nested stream producer chain must not overflow the worker stack");
}

/// The real acceptance test for the root fix: drive a 40-deep forwarding stream
/// producer chain on a *small* (1 MiB) stack — both the executor thread and the
/// runtime's worker thread are sized to 1 MiB. The pre-fix co-driven model nested
/// one future per producer level on a single native stack and overflowed/aborted
/// well before depth 40 at 1 MiB. With every producer running in its own
/// `tokio::spawn`ed task, the consumer only polls the bounded channel, so the
/// native stack depth is constant regardless of nesting depth and the chain
/// completes on the small stack. This distinguishes the root fix from the
/// 64 MiB worker-stack mitigation: if the producers were still co-driven, this
/// test would abort the process.
#[test]
fn runtime_program_deeply_nested_stream_producers_are_stack_depth_independent() {
    let depth: usize = std::env::var("SKIFF_NESTED_PRODUCER_DEPTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(40)
        .max(2);
    // 1 MiB: far below the 64 MiB mitigation, and below the ~8 MiB at which the
    // pre-fix code already aborted by depth 32. If producers were still co-driven
    // this would overflow and abort the process.
    let stack_bytes: usize = 1024 * 1024;

    let handle = std::thread::Builder::new()
        .name("nested-stream-producer-small-stack-test".to_string())
        .stack_size(stack_bytes)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_stack_size(stack_bytes)
                .enable_all()
                .build()
                .expect("multi-thread runtime should build");
            runtime.block_on(async move {
                let mut executables = vec![local_stream_aggregate_route_executable()];
                for level in 1..depth {
                    executables.push(forwarding_string_stream_producer_executable(level + 1));
                }
                executables.push(local_string_stream_producer_executable());

                let program = Arc::new(program_with_executables(executables));
                let interpreter = Interpreter::with_program(program, runtime_factory());
                let frame = test_invocation("svc.main.run");

                let value = execute_test_program_route(&interpreter, &frame)
                    .await
                    .expect("deep producer chain should run depth-independently on a small stack");

                // Every forwarding level passes items through unchanged, so the
                // leaf's "a","b","c" must still aggregate to "abc".
                assert_eq!(value, json!("abc"));
            });
        })
        .expect("small-stack test worker thread should spawn");

    handle
        .join()
        .expect("deep producer chain must complete on a 1 MiB stack (depth-independent)");
}

#[tokio::test]
async fn runtime_program_route_for_in_local_stream_producer_aggregates_emits() {
    let program = Arc::new(program_with_executables(vec![
        local_stream_aggregate_route_executable(),
        local_string_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("route should consume local stream producer");

    assert_eq!(value, json!("abc"));
}

#[tokio::test]
async fn runtime_program_stream_producer_emits_http_sse_response_event() {
    let program = Arc::new(program_with_executables_and_std_http_types(vec![
        local_stream_first_item_route_executable(),
        local_http_sse_response_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("Stream<std.http.HttpSseEvent> should accept the response event branch");

    assert_eq!(
        value,
        json!({
            "tag": "response",
            "status": 200,
            "headers": [
                {
                    "name": "content-type",
                    "value": "text/event-stream"
                }
            ]
        })
    );
}

#[tokio::test]
async fn runtime_program_stream_producer_argument_uses_its_own_item_type() {
    let program = Arc::new(program_with_executables_and_std_http_types(vec![
        local_stream_first_item_route_executable(),
        outer_string_stream_from_sse_producer_executable(),
        sse_tag_string_stream_converter_executable(),
        local_http_sse_response_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("nested stream producer argument should not inherit the outer stream item type");

    assert_eq!(value, json!("response"));
}

#[tokio::test]
async fn runtime_program_for_in_stream_returning_wrapper_consumes_returned_stream_handle() {
    let program = Arc::new(program_with_executables_and_std_http_types(vec![
        local_stream_first_item_route_executable(),
        local_native_stream_wrapper_executable(),
    ]));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.sse".to_string(),
            TestEffectDouble {
                expect_request: Some(json!({
                    "method": "GET",
                    "url": "https://example.test/events",
                    "headers": [],
                    "body": null,
                    "timeoutMs": null,
                })),
                response: json!([
                    { "tag": "event", "event": null, "id": null, "data": "abc" }
                ]),
            },
        )]),
        runtime_factory(),
    );
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("route should consume the Stream handle returned by a non-emit wrapper");

    assert_eq!(
        value,
        json!({ "tag": "event", "event": null, "id": null, "data": "abc" })
    );
}

#[tokio::test]
async fn runtime_program_stream_variable_for_in_decodes_item_with_item_type() {
    let program = Arc::new(program_with_executable(
        stream_variable_json_object_length_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");
    let context = program_invocation_context(&interpreter, &frame).execution_context();
    let mut heap = context.request_heap();

    let (stream_value, stream_sink) = interpreter.stream_runtime.channel_stream();
    stream_sink
        .send(json!({ "name": "Ada", "role": "pilot" }))
        .await
        .expect("stream item should enqueue");
    let stream_id = stream_value
        .get(STREAM_ID_KEY)
        .and_then(Value::as_str)
        .expect("test stream should expose an internal stream id");
    let stream_arg = RuntimeValue::Heap(
        heap.alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            STREAM_ID_KEY.to_string(),
            RuntimeValue::String(stream_id.to_string()),
        )])))
        .expect("stream handle object should allocate"),
    );
    let run_addr = ExecutableAddr::service(0, 0);

    let value = interpreter
        .call_program_executable(
            context,
            &mut heap,
            &Env::new(),
            &run_addr,
            &run_addr,
            &BTreeMap::new(),
            vec![stream_arg],
        )
        .await
        .expect("stream variable for-in should decode wire item with itemType");

    assert_eq!(value, RuntimeValue::Number(2.0));
}

#[tokio::test]
async fn runtime_program_forwards_native_http_sse_response_event() {
    let program = Arc::new(program_with_executables_and_std_http_types(vec![
        local_stream_first_item_route_executable(),
        local_native_sse_forwarding_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.sse".to_string(),
            TestEffectDouble {
                expect_request: Some(json!({
                    "method": "GET",
                    "url": "https://example.test/events",
                    "headers": [],
                    "body": null,
                    "timeoutMs": null,
                })),
                response: json!([
                    {
                        "tag": "response",
                        "status": 200,
                        "headers": [
                            {
                                "name": "content-type",
                                "value": "text/event-stream"
                            }
                        ]
                    },
                    { "tag": "event", "event": null, "id": null, "data": "abc" }
                ]),
            },
        )]),
        runtime_factory(),
    );
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("forwarded std.http.sse response event should satisfy HttpSseEvent");

    assert_eq!(
        value,
        json!({
            "tag": "response",
            "status": 200,
            "headers": [
                {
                    "name": "content-type",
                    "value": "text/event-stream"
                }
            ]
        })
    );
}

#[tokio::test]
async fn runtime_program_http_stream_effect_uses_native_signature_inside_http_handler() {
    let program = Arc::new(program_with_executable_and_std_http_types(
        http_stream_effect_in_http_handler_executable(),
    ));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.stream".to_string(),
            TestEffectDouble {
                expect_request: Some(json!({
                    "method": "POST",
                    "url": "https://example.test/chat/completions",
                    "headers": [],
                    "body": { "__skiffBytesBase64": "aGVsbG8gd29ybGQ=" },
                    "timeoutMs": null,
                })),
                response: json!({ "status": 200, "headers": [], "body": { "__skiffStreamId": "test-stream" } }),
            },
        )]),
        runtime_factory(),
    );
    let mut frame = test_invocation("svc.main.run");
    set_request_http_arg(&mut frame, "request");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.http.stream should use the native HttpClientRequest signature");

    assert_eq!(value, json!(200));
}

#[tokio::test]
async fn runtime_program_http_stream_event_helper_uses_native_signature_inside_http_handler() {
    let program = Arc::new(program_with_executable_and_std_http_types(
        http_stream_start_helper_in_http_handler_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    set_request_http_arg(&mut frame, "request");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.http.streamStart should use the native helper signature");

    assert_eq!(
        value,
        json!({ "tag": "start", "status": 200, "headers": [] })
    );
}

#[test]
fn test_host_operation_double_matches_bytes_request_without_materializing_actual_input() {
    let program = Arc::new(program_with_executable(run_executable()));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.request".to_string(),
            TestEffectDouble {
                expect_request: Some(json!({
                    "method": "POST",
                    "url": "https://example.test/upload",
                    "headers": [],
                    "body": { "__skiffBytesBase64": "aGVsbG8gd29ybGQ=" }
                })),
                response: json!({
                    "status": 204,
                    "headers": [],
                    "body": { "__skiffBytesBase64": "" }
                }),
            },
        )]),
        runtime_factory(),
    );
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_materialize_output_bytes: 1,
        ..RequestHeapLimits::default()
    });
    let input = http_client_request_runtime_value(&mut heap);
    let arg_type = json!({ "kind": "builtin", "name": "std.http.HttpClientRequest", "args": [] });
    let return_type =
        json!({ "kind": "builtin", "name": "std.http.HttpClientResponse", "args": [] });
    let arg_plan = RuntimeTypePlan::from_descriptor(&arg_type).expect("arg plan should build");
    let return_plan =
        RuntimeTypePlan::from_descriptor(&return_type).expect("return plan should build");

    let value = interpreter
        .dispatch_test_http_effect_invocation_double(
            "std.http.client.request",
            Some(&input),
            Some(&arg_plan),
            Some(&return_plan),
            &mut heap,
        )
        .expect("test double should dispatch")
        .expect("test double should match bytes input without materializing it");

    assert!(matches!(value, RuntimeValue::Heap(_)));
    assert_eq!(heap.stats().materialize_output_bytes, 0);
}

#[test]
fn test_host_operation_double_rejects_public_http_target_id() {
    let program = Arc::new(program_with_executable(run_executable()));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.request".to_string(),
            TestEffectDouble {
                expect_request: None,
                response: json!({
                    "status": 204,
                    "headers": [],
                    "body": { "__skiffBytesBase64": "" }
                }),
            },
        )]),
        runtime_factory(),
    );
    let mut heap = RequestHeap::default();
    let input = RuntimeValue::Null;
    let arg_type = json!({ "kind": "builtin", "name": "std.http.HttpClientRequest", "args": [] });
    let return_type =
        json!({ "kind": "builtin", "name": "std.http.HttpClientResponse", "args": [] });
    let arg_plan = RuntimeTypePlan::from_descriptor(&arg_type).expect("arg plan should build");
    let return_plan =
        RuntimeTypePlan::from_descriptor(&return_type).expect("return plan should build");

    let result = interpreter.dispatch_test_http_effect_invocation_double(
        "std.http.request",
        Some(&input),
        Some(&arg_plan),
        Some(&return_plan),
        &mut heap,
    );

    assert!(
        result.is_none(),
        "runtime HTTP effect doubles must be keyed by stable bindingKey"
    );
}

#[tokio::test]
async fn runtime_program_stream_producer_emit_is_driven_by_for_body() {
    let program = Arc::new(program_with_executables(vec![
        local_stream_first_item_route_executable(),
        local_string_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("for body should process the first emitted item and cancel producer");

    assert_eq!(value, json!("a"));
}

/// Cancel-timing across the new producer-task boundary. The producer emits three
/// items into a bounded (cap 1) channel, but the consumer takes only the first
/// item and returns. The producer then sits blocked on `send_with_cancel`
/// backpressure for its second emit, on its *own* spawned task. The consumer's
/// `Flow::Return` cancels the stream (`stream_runtime.cancel`), which must reach
/// the producer task via the cross-task cancel flag/notify and unblock its
/// pending `send`, letting the whole route finish. Pre-fix, the producer was
/// co-driven in the same task and cancellation was observed synchronously at the
/// next poll; this test proves the signal still terminates the producer now that
/// it lives on a separate task. The `timeout` guards against a regression where
/// cancellation fails to cross the boundary (the route would otherwise hang on a
/// detached, backpressured producer task).
#[tokio::test]
async fn runtime_program_stream_producer_cancelled_across_task_boundary_on_consumer_return() {
    let program = Arc::new(program_with_executables(vec![
        local_stream_first_item_route_executable(),
        local_string_stream_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("route must not hang: consumer return must cancel the spawned producer task")
    .expect("route consuming only the first stream item should return it");

    // Consumer returns after the first item; producer is cancelled before its
    // remaining "b"/"c" emits matter.
    assert_eq!(value, json!("a"));
}

#[tokio::test]
async fn runtime_program_create_from_stream_prefers_producer_error_after_consumer_error() {
    let program = Arc::new(program_with_executables(vec![
        create_from_stream_route_executable(),
        bytes_stream_emit_then_bad_emit_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("producer error should win over missing file store consumer error");

    assert!(
        error
            .to_string()
            .contains("stream emit item: expected runtime bytes"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_create_from_stream_items_use_request_heap_budget() {
    let program = Arc::new(program_with_executable(
        emit_response_stream_helper_executable(),
    ));
    let interpreter = Interpreter::with_program(program.clone(), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(
        Arc::new(
            skiff_runtime_service_db::ServiceDbRuntime::new(
                "example.com/create-from-stream-budget".to_string(),
                "mongodb://127.0.0.1:27017".to_string(),
                &[],
            )
            .expect("serviceDb metadata should parse without connecting"),
        )
        .capability_factory(),
    );
    frame.request_heap_limits = RequestHeapLimits {
        max_estimated_bytes: 1,
        ..RequestHeapLimits::default()
    };

    let (stream_value, stream_sink) = interpreter.stream_runtime.channel_stream();
    stream_sink
        .send(json!({
            "__skiffBytesBase64": "MDEyMzQ1Njc4OWFiY2RlZg=="
        }))
        .await
        .expect("stream item should enqueue");
    let stream_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("bytes")],
        })
        .expect("stream plan should build");
    let mut heap = RequestHeap::default();
    let stream_arg = RuntimeBoundaryCodec::new(
        &stream_plan,
        BoundaryUse::NativeReturn,
        "createFromStream budget test stream",
    )
    .from_wire_json_internal_handle(&stream_value, &mut heap)
    .expect("stream handle should decode for native call");

    let invocation_context = program_invocation_context(&interpreter, &frame);
    let execution_context = invocation_context.execution_context();
    let native_dispatch = NativeDispatch::new();
    let addr = ExecutableAddr::service(0, 0);
    let env = Env::default();
    let call = create_from_stream_call_ir();
    let target = NativeTarget {
        namespace: "std.file".to_string(),
        symbol: "createFromStream".to_string(),
        binding_key: Some("std.file.createFromStream".to_string()),
        metadata: BTreeMap::new(),
    };
    let invocation = resolve_runtime_native_invocation(&interpreter, &addr, &env, &call, &target)
        .expect("createFromStream invocation should resolve");
    let eval_program = crate::eval::EvalRuntimeProgram::from_source(program.as_ref());
    let native_capability_context = project_runtime_native_capability_context(
        &execution_context,
        eval_program.projection(),
        env.stream_capability_context(),
        invocation.required_context(),
    );
    let error = native_dispatch
        .dispatch_resolved_native_call(
            native_capability_context,
            invocation,
            vec![stream_arg, RuntimeValue::Null],
            &mut heap,
        )
        .await
        .expect_err("stream item conversion should enforce request heap budget");
    let payload = error.payload();
    assert_eq!(payload.code, "ResourceLimitExceeded");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["resource"].as_str()),
        Some("requestHeap"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_emit_response_stream_uses_response_sink_not_inner_sink() {
    let program = Arc::new(program_with_executable_and_std_http_types(
        emit_response_stream_helper_executable(),
    ));
    let interpreter = Interpreter::with_program(program.clone(), runtime_factory());
    let frame = test_invocation("svc.main.run");
    let addr = ExecutableAddr::service(0, 0);
    let mut heap = RequestHeap::default();
    let mut env = Env::default();
    let (response_stream, response_sink) = interpreter.stream_runtime.channel_stream();
    let (archive_stream, archive_sink) = interpreter.stream_runtime.channel_stream();
    env.stream_sink = Some(archive_sink.clone());
    env.response_stream_sink = Some(TypedStreamSink {
        sink: response_sink.clone(),
        item_type: std_http_type_plan_for_test(
            program.as_ref(),
            &addr,
            STD_HTTP_RESPONSE_STREAM_EVENT_TYPE_INDEX,
        ),
    });

    let invocation_context = program_invocation_context(&interpreter, &frame);
    let execution_context = invocation_context.execution_context();
    let native_dispatch = NativeDispatch::new();
    let emit_response_call = emit_response_stream_call_ir();
    let emit_response_target = NativeTarget {
        namespace: "std.http".to_string(),
        symbol: "emitResponseStream".to_string(),
        binding_key: Some("std.http.stream.emitResponse".to_string()),
        metadata: BTreeMap::new(),
    };
    let invocation = resolve_runtime_native_invocation(
        &interpreter,
        &addr,
        &env,
        &emit_response_call,
        &emit_response_target,
    )
    .expect("emitResponseStream invocation should resolve");
    let eval_program = crate::eval::EvalRuntimeProgram::from_source(program.as_ref());
    let native_capability_context = project_runtime_native_capability_context(
        &execution_context,
        eval_program.projection(),
        env.stream_capability_context(),
        invocation.required_context(),
    );
    let result = native_dispatch
        .dispatch_resolved_native_call(
            native_capability_context,
            invocation,
            vec![http_stream_chunk_value(&mut heap, b"client")],
            &mut heap,
        )
        .await
        .expect("emitResponseStream should send to response stream");
    assert!(matches!(result, RuntimeValue::Null));

    let response_event = interpreter
        .stream_runtime
        .next(&response_stream)
        .await
        .expect("response stream should receive forwarded event");
    assert!(matches!(
        response_event,
        StreamPoll::Item(value)
            if value == json!({ "tag": "chunk", "value": { "__skiffBytesBase64": "Y2xpZW50" } })
    ));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(50),
            interpreter.stream_runtime.next(&archive_stream)
        )
        .await
        .is_err(),
        "archive stream should not receive emitResponseStream event"
    );

    archive_sink
        .cancel_flag()
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let invocation = resolve_runtime_native_invocation(
        &interpreter,
        &addr,
        &env,
        &emit_response_call,
        &emit_response_target,
    )
    .expect("emitResponseStream invocation should resolve");
    let eval_program = crate::eval::EvalRuntimeProgram::from_source(program.as_ref());
    let native_capability_context = project_runtime_native_capability_context(
        &execution_context,
        eval_program.projection(),
        env.stream_capability_context(),
        invocation.required_context(),
    );
    let error = native_dispatch
        .dispatch_resolved_native_call(
            native_capability_context,
            invocation,
            vec![http_stream_chunk_value(&mut heap, b"after-cancel")],
            &mut heap,
        )
        .await
        .expect_err("archive sink cancellation should stop nested forwarding");
    let payload = error.payload();
    assert_eq!(payload.code, "CancelError", "unexpected error: {error}");
}

#[tokio::test]
async fn runtime_program_emit_response_stream_requires_response_stream_context() {
    let program = Arc::new(program_with_executable_and_std_http_types(
        emit_response_stream_helper_executable(),
    ));
    let interpreter = Interpreter::with_program(program.clone(), runtime_factory());
    let frame = test_invocation("svc.main.run");
    let addr = ExecutableAddr::service(0, 0);
    let mut heap = RequestHeap::default();
    let env = Env::default();

    let invocation_context = program_invocation_context(&interpreter, &frame);
    let execution_context = invocation_context.execution_context();
    let native_dispatch = NativeDispatch::new();
    let emit_response_call = emit_response_stream_call_ir();
    let emit_response_target = NativeTarget {
        namespace: "std.http".to_string(),
        symbol: "emitResponseStream".to_string(),
        binding_key: Some("std.http.stream.emitResponse".to_string()),
        metadata: BTreeMap::new(),
    };
    let invocation = resolve_runtime_native_invocation(
        &interpreter,
        &addr,
        &env,
        &emit_response_call,
        &emit_response_target,
    )
    .expect("emitResponseStream invocation should resolve");
    let eval_program = crate::eval::EvalRuntimeProgram::from_source(program.as_ref());
    let native_capability_context = project_runtime_native_capability_context(
        &execution_context,
        eval_program.projection(),
        env.stream_capability_context(),
        invocation.required_context(),
    );
    let error = native_dispatch
        .dispatch_resolved_native_call(
            native_capability_context,
            invocation,
            vec![http_stream_chunk_value(&mut heap, b"client")],
            &mut heap,
        )
        .await
        .expect_err("emitResponseStream should require a response stream context");
    let message = error.to_string();
    assert!(
        message.ends_with("used outside a raw HTTP streaming response context"),
        "unexpected error message: {message}"
    );
}

#[tokio::test]
async fn runtime_program_executes_match_statement() {
    let program = Arc::new(program_with_executable(match_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("match statement should execute");

    assert_eq!(value, json!("matched"));
}

#[tokio::test]
async fn runtime_program_catches_typed_throw_expression() {
    let program = Arc::new(program_with_executables_and_local_error_type(
        vec![catch_throw_executable()],
        "AuthError",
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("catch expression should catch matching typed throw");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "address",
            "addr": serde_json::to_value(service_type_addr(0)).unwrap()
        })
    );
    assert_eq!(
        value["exception"]["__skiffActualPayloadTypeDebug"],
        "service:file[0]:type[0]"
    );
    assert_no_legacy_skiff_type_key(&value["exception"]);
    assert_eq!(value["exception"]["error"]["message"], "denied");
}

#[tokio::test]
async fn runtime_program_catches_without_type_catches_user_exception() {
    let program = Arc::new(program_with_executables_and_local_error_type(
        vec![catch_throw_without_catch_type_executable()],
        "AuthError",
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("catch without type should catch matching user throw");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "address",
            "addr": serde_json::to_value(service_type_addr(0)).unwrap()
        })
    );
    assert_no_legacy_skiff_type_key(&value["exception"]);
    assert_eq!(value["exception"]["error"]["message"], "denied");
}

#[tokio::test]
async fn runtime_program_catches_builtin_error_throw_expression() {
    let program = Arc::new(program_with_executable(
        catch_builtin_decode_error_throw_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("builtin error catch should catch matching typed throw");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "builtin",
            "name": "std.json.DecodeError"
        })
    );
    assert_no_legacy_skiff_type_key(&value["exception"]);
    assert_eq!(value["exception"]["error"]["target"], "test.decode");
    assert_eq!(value["exception"]["error"]["message"], "denied");
}

#[tokio::test]
async fn runtime_program_catches_nonmatching_builtin_error_throw_expression() {
    let program = Arc::new(program_with_executable(
        catch_builtin_decode_error_throw_with_catch_type_executable("std.service.ProtocolError"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.service.ProtocolError catch must not catch std.json.DecodeError throw");

    match runtime_error_leaf(&error) {
        RuntimeError::UserException(exception) => {
            assert_eq!(
                exception.actual_payload_type(),
                &TypeIdentity::builtin("std.json.DecodeError")
            );
            assert_no_legacy_skiff_type_key(&exception.envelope());
        }
        other => panic!("expected uncaught std.json.DecodeError user exception, got {other:?}"),
    }
}

#[tokio::test]
async fn runtime_program_catches_native_decode_error_with_builtin_catch_type() {
    let program = Arc::new(program_with_executable(
        catch_native_decode_error_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.json.DecodeError catch should catch std.json.decode failure");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "builtin",
            "name": "std.json.DecodeError"
        })
    );
    assert_no_legacy_skiff_type_key(&value["exception"]);
    assert_eq!(value["exception"]["error"]["target"], "std.json.decode");
}

#[tokio::test]
async fn runtime_program_accepts_std_http_error_builtin_catch_type() {
    let program = Arc::new(program_with_executable(
        catch_literal_with_catch_type_executable("std.http.HttpError"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.http.HttpError catch type should resolve as a concrete builtin error");

    assert_eq!(value["tag"], "ok");
    assert_eq!(value["value"], 7);
}

#[tokio::test]
async fn runtime_program_catches_without_type_does_not_catch_native_decode_error() {
    let program = Arc::new(program_with_executable(
        catch_native_decode_error_without_catch_type_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("catch without type must not catch native std.json.DecodeError");

    let payload = runtime_error_leaf(&error).payload();
    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["target"].as_str()),
        Some("std.json.decode")
    );
}

#[tokio::test]
async fn runtime_program_catches_without_type_does_not_swallow_cancellation() {
    let program = Arc::new(program_with_executable(
        catch_time_sleep_without_catch_type_executable(100),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");
    let cancellation = frame.cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(10)).await;
        cancellation.cancel();
    });

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep catch test should observe cancellation")
    .expect_err("catch without type must not swallow cancellation");
    cancel_task
        .await
        .expect("cancellation task should complete");

    assert!(matches!(
        runtime_error_leaf(&error),
        RuntimeError::Cancelled
    ));
}

#[tokio::test]
async fn runtime_program_catches_cancel_error_with_builtin_catch_type() {
    let program = Arc::new(program_with_executable(
        catch_time_sleep_with_catch_type_executable(100, "CancelError"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");
    let cancellation = frame.cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        sleep(Duration::from_millis(10)).await;
        cancellation.cancel();
    });

    let value = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep catch test should observe cancellation")
    .expect("CancelError catch should catch cancellation");
    cancel_task
        .await
        .expect("cancellation task should complete");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "builtin",
            "name": "CancelError"
        })
    );
    assert_eq!(
        value["exception"]["error"]["message"],
        "request was cancelled"
    );
}

#[tokio::test]
async fn runtime_program_catches_without_type_does_not_swallow_execution_budget() {
    let program = Arc::new(program_with_executable(
        catch_time_sleep_without_catch_type_executable(100),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.execution_budget = Arc::new(crate::execution_budget::ExecutionBudget::new(
        crate::execution_budget::ExecutionBudgetConfig::runtime_default(),
        Some(std::time::Instant::now() + Duration::from_millis(15)),
    ));

    let error = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep catch test should observe request deadline")
    .expect_err("catch without type must not swallow execution budget");

    assert!(matches!(
        runtime_error_leaf(&error),
        RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::DeadlineExceeded,
            ..
        }
    ));
}

#[tokio::test]
async fn runtime_program_catches_timeout_error_with_builtin_catch_type() {
    let program = Arc::new(program_with_executable(
        catch_time_sleep_with_catch_type_executable(100, "TimeoutError"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.execution_budget = Arc::new(crate::execution_budget::ExecutionBudget::new(
        crate::execution_budget::ExecutionBudgetConfig::runtime_default(),
        Some(std::time::Instant::now() + Duration::from_millis(15)),
    ));

    let value = tokio::time::timeout(
        Duration::from_secs(1),
        execute_test_program_route(&interpreter, &frame),
    )
    .await
    .expect("std.time.sleep catch test should observe request deadline")
    .expect("TimeoutError catch should catch execution budget");

    assert_eq!(value["tag"], "err");
    assert_eq!(
        value["exception"]["__skiffActualPayloadType"],
        json!({
            "kind": "builtin",
            "name": "TimeoutError"
        })
    );
    assert_eq!(value["exception"]["error"]["reason"], "deadlineExceeded");
}

#[test]
fn user_exception_rethrow_envelope_accepts_erased_payload() {
    let exception = UserException::from_typed_payload(
        json!({ "message": "denied" }),
        TypeIdentity::address(service_type_addr(0)),
        Some(TypeIdentity::address(service_type_addr(0))),
    )
    .expect("typed user exception should be constructed");

    let mut envelope = exception.envelope();
    envelope["__skiffActualPayloadTypeDebug"] = json!("svc.main.AuthError");
    let rethrown = UserException::from_envelope(envelope).expect("envelope should rethrow");

    assert_eq!(
        rethrown.actual_payload_type(),
        &TypeIdentity::address(service_type_addr(0))
    );
    assert_eq!(
        rethrown.envelope().pointer("/error/message"),
        Some(&json!("denied"))
    );
    assert!(rethrown.envelope().pointer("/error/__skiffType").is_none());
}

#[test]
fn user_exception_rethrow_rejects_string_payload_type_identity() {
    let exception = UserException::from_typed_payload(
        json!({ "message": "denied" }),
        TypeIdentity::address(service_type_addr(0)),
        Some(TypeIdentity::address(service_type_addr(0))),
    )
    .expect("typed user exception should be constructed");

    let mut envelope = exception.envelope();
    envelope["__skiffActualPayloadType"] = json!("service:file[0]:type[0]");

    let error = UserException::from_envelope(envelope)
        .expect_err("string payload type must not rebuild TypeIdentity");
    assert!(error.to_string().contains("invalid actual payload type"));
}

#[tokio::test]
async fn runtime_program_does_not_catch_same_named_error_with_different_type_addr() {
    let program = Arc::new(program_with_two_same_named_error_types(vec![
        catch_throw_with_type_addrs_executable(service_type_addr(1), service_type_addr(0)),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("catch type addr 0 must not catch thrown type addr 1");

    match error {
        RuntimeError::UserException(exception) => {
            assert_eq!(
                exception.actual_payload_type(),
                &TypeIdentity::address(service_type_addr(1))
            );
        }
        other => panic!("expected user exception, got {other:?}"),
    }
}

#[tokio::test]
async fn runtime_program_type_pattern_fails_closed_for_erased_value() {
    let program = Arc::new(program_with_executable(type_pattern_match_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("nominal type pattern should fail closed for erased value");

    assert!(
        error
            .to_string()
            .contains("nominal type pattern cannot match an erased runtime value"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_assert_false_returns_decode_error() {
    let program = Arc::new(program_with_executable(assert_executable(false)));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("assert false should fail");

    assert!(
        error.to_string().contains("assert failed in program"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_declares_self_from_slot_def_for_local_call() {
    let program = Arc::new(program_with_executables(vec![
        self_local_call_executable(),
        read_self_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("self slot def should be declared before local call reads caller self");

    assert_eq!(value, json!({}));
}

#[tokio::test]
async fn runtime_program_db_rejects_old_dotted_builtin_surface() {
    let program = Arc::new(program_with_executable(old_db_builtin_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("old dotted db builtin should be rejected before object DB execution");

    assert!(
        error
            .to_string()
            .contains("old RuntimeProgram db builtin db.create is not supported"),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_program_db_rejects_negative_offset_before_querying() {
    let program = program_with_executable(db_negative_offset_executable());
    let service_db = Arc::new(
        skiff_runtime_service_db::ServiceDbRuntime::new(
            "example.com/svc".to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &thread_db_metadata(),
        )
        .expect("serviceDb metadata should parse"),
    );
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(service_db.capability_factory());

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("negative offset should be rejected before querying");

    assert!(
        error
            .to_string()
            .contains("db find many offset must be a non-negative integer"),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_program_db_rejects_after_pagination_before_querying() {
    let program = program_with_executable(db_after_executable());
    let service_db = Arc::new(
        skiff_runtime_service_db::ServiceDbRuntime::new(
            "example.com/svc".to_string(),
            "mongodb://127.0.0.1:27017".to_string(),
            &thread_db_metadata(),
        )
        .expect("serviceDb metadata should parse"),
    );
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(service_db.capability_factory());

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("after pagination should be rejected before querying");

    assert!(
        error
            .to_string()
            .contains("db find many after is not supported; use offset and limit"),
        "{error}"
    );
}

#[test]
fn runtime_type_plan_resolves_package_db_object_symbol_from_file_declarations() {
    let mut program = program_with_executable(run_executable());
    let db_object_descriptor = LinkedTypeDescriptor::Record {
        fields: BTreeMap::from([
            ("id".to_string(), linked_builtin_type("string")),
            ("kind".to_string(), linked_builtin_type("string")),
        ]),
    };
    let declarations: FileDeclarations = serde_json::from_value(json!({
        "types": {
            "BrowserSession": {
                "typeIndex": 0,
                "symbol": "session.BrowserSession"
            }
        }
    }))
    .expect("test file declarations should decode");
    let mut package_file = package_file_unit("file:http-session", "session", run_executable());
    package_file.declarations = declarations;
    package_file.types = vec![TypeDeclIr {
        name: "BrowserSession".to_string(),
        descriptor: db_object_descriptor.clone(),
        type_params: Vec::new(),
        discriminator: None,
        implements: Vec::new(),
        source_span: None,
    }];
    program.packages = vec![Arc::new(package_unit("skiff.run/http-session"))];
    program.package_files = vec![vec![Arc::new(package_file)]];
    program.package_resources = vec![Default::default()];
    program.types.descriptors.insert(
        TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        },
        anonymous_type_decl("BrowserSession", db_object_descriptor),
    );

    let image = program.linked_image();
    let plan = RuntimeTypePlan::from_linked(
        &LinkedTypeRef::Native {
            name: "DbUpsertResult".to_string(),
            args: vec![LinkedTypeRef::DbObjectSymbol {
                symbol: ServiceSymbolRef {
                    module_path: "session".to_string(),
                    symbol: "BrowserSession".to_string(),
                },
            }],
        },
        &PlanContext::new(&image, &ExecutableAddr::package(0, 0, 0)),
    )
    .expect("package DB object result plan should resolve");

    let RuntimeTypeNode::Record { fields, .. } = plan.node() else {
        panic!("DbUpsertResult should be a record");
    };
    let value_field = fields
        .iter()
        .find(|field| field.name == "value")
        .expect("DbUpsertResult should expose value");
    assert!(matches!(
        value_field.ty.node(),
        RuntimeTypeNode::Record { fields, .. }
            if fields.iter().any(|field| field.name == "id")
                && fields.iter().any(|field| field.name == "kind")
    ));
}

#[tokio::test]
async fn runtime_program_db_query_value_evaluates_conditional_predicates_and_options() {
    let program = Arc::new(program_with_executable(db_query_value_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("db query value should execute without service db");

    assert_eq!(
        value["trueOnly"]["filter"],
        json!({ "status": { "$eq": "open" } })
    );
    assert_eq!(value["falseOnly"]["filter"], Value::Null);
    assert_eq!(
        value["mixed"]["filter"],
        json!({
            "$and": [
                { "score": { "$gt": 10 } },
                { "status": { "$eq": "open" } }
            ]
        })
    );
    assert_eq!(value["mixed"]["typeName"], json!("Thread"));
    assert_eq!(value["mixed"]["limit"], json!(5));
    assert_eq!(value["mixed"]["offset"], json!(2));
    assert_eq!(value["mixed"]["after"], json!("cursor-1"));
    assert_eq!(
        value["mixed"]["order"],
        json!([
            {
                "field": { "text": "score", "segments": ["score"] },
                "direction": "desc"
            }
        ])
    );
    assert_eq!(
        value["mixed"]["target"],
        json!({
            "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
            "typeName": "Thread"
        })
    );
}

#[tokio::test]
async fn runtime_program_db_many_key_selector_is_rejected() {
    let program = Arc::new(program_with_executable(db_many_key_selector_executable()));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(
        Arc::new(
            skiff_runtime_service_db::ServiceDbRuntime::new(
                "example.com/test".to_string(),
                "mongodb://127.0.0.1:27017".to_string(),
                &thread_db_metadata(),
            )
            .expect("serviceDb metadata should parse"),
        )
        .capability_factory(),
    );

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("many key selector should be rejected before db execution");

    assert!(
        error
            .to_string()
            .contains("db many operation cannot use a key selector"),
        "{error}"
    );
}

#[tokio::test]
async fn runtime_program_constructs_outbound_service_request_start() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("unary")];
    program.timeout.default_ms = Some(1000);
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.request.extra.insert(
        "trace".to_string(),
        json!({
            "traceId": "trace-caller",
            "spanId": "span-caller",
            "sampled": true
        }),
    );
    frame.request.extra.insert(
        "deadline".to_string(),
        json!({
            "timeoutMs": 5000,
            "expiresAt": "2999-01-01T00:00:00Z"
        }),
    );
    let mut heap = RequestHeap::default();
    let (request, payload) = outbound_control_and_payload_for_test(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .expect("outbound request should build");

    assert_eq!(request.mode, "unary");
    assert_eq!(request.caller.kind, "service");
    assert_eq!(request.caller.target, "svc.main.run");
    assert_eq!(request.service_id.as_deref(), Some("skiff.run/account"));
    assert_eq!(request.build_id, BUILD_OUTBOUND);
    assert_eq!(request.service_protocol_identity, PROTOCOL_OUTBOUND);
    assert_eq!(request.target, "lookup");
    assert_eq!(request.trace.trace_id, "trace-caller");
    assert_eq!(request.trace.parent_span_id.as_deref(), Some("span-caller"));
    assert_eq!(request.trace.sampled, Some(true));
    assert_eq!(
        request
            .deadline
            .as_ref()
            .map(|deadline| deadline.timeout_ms),
        Some(1000)
    );

    let mut decoded_heap = RequestHeap::default();
    let decoded = decode_payload(
        &payload,
        &json!({
            "kind": "record",
            "fields": {
                "userId": { "kind": "builtin", "name": "string", "args": [] }
            }
        }),
        &mut decoded_heap,
    )
    .expect("payload should decode");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("expected decoded args record");
    };
    let fields = match decoded_heap.get(handle).expect("args handle should exist") {
        HeapNode::Object(object) => object.fields(),
        other => panic!("expected object args, got {other:?}"),
    };
    assert_eq!(
        fields.get("userId"),
        Some(&RuntimeValue::String("user-1".to_string()))
    );
}

#[tokio::test]
async fn runtime_program_service_dependency_call_missing_alias_fails_closed() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let program = Arc::new(program_with_executable(run_executable()));
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation_for_program("svc.main.run", runtime_activation);
    let mut heap = RequestHeap::default();
    let error = outbound_control_and_payload_for_test(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        Vec::new(),
    )
    .expect_err("missing service dependency should fail closed");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error
        .to_string()
        .contains("service dependency alias account is not declared"));
}

#[tokio::test]
async fn runtime_program_service_dependency_call_missing_operation_fails_closed() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    let mut dependency = account_service_dependency("unary");
    dependency.publication_abi.operation_abi.clear();
    program.service_dependencies = vec![dependency];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation_for_program("svc.main.run", runtime_activation);
    let mut heap = RequestHeap::default();

    let error = outbound_control_and_payload_for_test(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .expect_err("missing service dependency operation should fail closed");

    assert!(matches!(error, RuntimeError::InvalidArtifact(_)));
    assert!(error
        .to_string()
        .contains("service dependency alias account does not declare operationAbiId"));
}

#[tokio::test]
async fn runtime_program_service_dependency_server_stream_returns_stream_handle() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("serverStream")];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, mut router_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    let mut heap = RequestHeap::default();

    let value = crate::eval::service_dispatch::call_outbound_service(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &Env::default(),
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .await
    .expect("serverStream dependency call should return a stream handle");

    let request = router_receiver
        .recv()
        .await
        .expect("outbound request.start should be sent");
    let (header, _payload) = request_start_control(request);
    assert_eq!(header.mode, "serverStream");
    assert_eq!(
        header.operation_abi_id.as_deref(),
        Some(ACCOUNT_LOOKUP_OPERATION_ABI_ID)
    );
    assert_eq!(
        header.selector.as_deref(),
        Some("operation:operation:account:lookup")
    );

    let outbound = frame
        .outbound_requests
        .sender(&header.request_id)
        .expect("serverStream outbound response should remain pending");
    outbound
        .send(crate::request::OutboundResponse::Start {
            http_response: crate::request::HttpResponseMetadata {
                status: 200,
                headers: Vec::new(),
            },
        })
        .expect("response.start should send");
    let item_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::native("string"))
            .expect("string item plan should build");
    let payload = encode_payload_plan(
        &RuntimeValue::String("stream-item".to_string()),
        &item_plan,
        &PayloadBoundary::runtime_internal(),
        &RequestHeap::default(),
    )
    .expect("chunk payload should encode");
    outbound
        .send(crate::request::OutboundResponse::Chunk { seq: 0, payload })
        .expect("response.chunk should send");
    outbound
        .send(crate::request::OutboundResponse::End {
            payload: Vec::new(),
        })
        .expect("response.end should send");

    let stream_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("string")],
        })
        .expect("stream plan should build");
    let stream_value = RuntimeBoundaryCodec::new(
        &stream_plan,
        BoundaryUse::NativeReturn,
        "serverStream test result",
    )
    .to_wire_json_internal_handle(&value, &mut heap)
    .expect("stream handle should materialize for test");
    let item = interpreter
        .stream_runtime
        .next(&stream_value)
        .await
        .expect("stream item should decode");
    assert!(matches!(
        item,
        StreamPoll::Item(serde_json::Value::String(value)) if value == "stream-item"
    ));
    let end = interpreter
        .stream_runtime
        .next(&stream_value)
        .await
        .expect("stream should end");
    assert!(matches!(end, StreamPoll::End));
}

#[tokio::test]
async fn runtime_program_service_dependency_server_stream_includes_service_timeout_deadline() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("serverStream")];
    program.timeout.default_ms = Some(120_000);
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, mut router_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    let mut heap = RequestHeap::default();

    let value = crate::eval::service_dispatch::call_outbound_service(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &Env::default(),
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .await
    .expect("serverStream dependency call should return a stream handle");

    let request = router_receiver
        .recv()
        .await
        .expect("outbound request.start should be sent");
    let (header, _payload) = request_start_control(request);
    assert_eq!(header.mode, "serverStream");
    assert_eq!(
        header.deadline.as_ref().map(|deadline| deadline.timeout_ms),
        Some(120_000)
    );

    drop(value);
}

#[tokio::test]
async fn runtime_program_service_dependency_server_stream_chunks_use_request_heap_budget() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut dependency = account_service_dependency("serverStream");
    dependency.publication_abi.operation_abi[0]
        .public_signature
        .return_type = skiff_artifact_model::TypeRefIr::Native {
        name: "Stream".to_string(),
        args: vec![skiff_artifact_model::TypeRefIr::native("bytes")],
    };
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![dependency];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, mut router_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    frame.request_heap_limits = RequestHeapLimits {
        max_estimated_bytes: 1,
        ..RequestHeapLimits::default()
    };
    let mut heap = RequestHeap::default();

    let value = crate::eval::service_dispatch::call_outbound_service(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &Env::default(),
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .await
    .expect("serverStream dependency call should return a stream handle");

    let request = router_receiver
        .recv()
        .await
        .expect("outbound request.start should be sent");
    let (header, _payload) = request_start_control(request);
    let outbound = frame
        .outbound_requests
        .sender(&header.request_id)
        .expect("serverStream outbound response should remain pending");
    outbound
        .send(crate::request::OutboundResponse::Start {
            http_response: crate::request::HttpResponseMetadata {
                status: 200,
                headers: Vec::new(),
            },
        })
        .expect("response.start should send");
    let item_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::native("bytes"))
            .expect("bytes item plan should build");
    let mut encode_heap = RequestHeap::default();
    let bytes_handle = encode_heap
        .alloc_bytes(vec![0_u8; 16])
        .expect("bytes response should allocate for encoding");
    let payload = encode_payload_plan(
        &RuntimeValue::Heap(bytes_handle),
        &item_plan,
        &PayloadBoundary::runtime_internal(),
        &encode_heap,
    )
    .expect("chunk payload should encode");
    outbound
        .send(crate::request::OutboundResponse::Chunk { seq: 0, payload })
        .expect("response.chunk should send");

    let stream_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("bytes")],
        })
        .expect("stream plan should build");
    let stream_value = RuntimeBoundaryCodec::new(
        &stream_plan,
        BoundaryUse::NativeReturn,
        "serverStream bytes budget test result",
    )
    .to_wire_json_internal_handle(&value, &mut heap)
    .expect("stream handle should materialize for test");
    let error = interpreter
        .stream_runtime
        .next(&stream_value)
        .await
        .expect_err("chunk decode should enforce request heap budget");
    let error = crate::error::RuntimeError::from(error);
    let payload = error.payload();
    assert_eq!(payload.code, "ResourceLimitExceeded");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["resource"].as_str()),
        Some("requestHeap"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn runtime_program_service_dependency_server_stream_decode_error_cancels_outbound() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("serverStream")];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, mut router_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    let mut heap = RequestHeap::default();

    let value = crate::eval::service_dispatch::call_outbound_service(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &Env::default(),
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .await
    .expect("serverStream dependency call should return a stream handle");

    let request = router_receiver
        .recv()
        .await
        .expect("outbound request.start should be sent");
    let (header, _payload) = request_start_control(request);
    let outbound = frame
        .outbound_requests
        .sender(&header.request_id)
        .expect("serverStream outbound response should remain pending");
    outbound
        .send(crate::request::OutboundResponse::Start {
            http_response: crate::request::HttpResponseMetadata {
                status: 200,
                headers: Vec::new(),
            },
        })
        .expect("response.start should send");
    outbound
        .send(crate::request::OutboundResponse::Chunk {
            seq: 0,
            payload: vec![0xff, 0x00],
        })
        .expect("invalid response.chunk should send");

    let stream_plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("string")],
        })
        .expect("stream plan should build");
    let stream_value = RuntimeBoundaryCodec::new(
        &stream_plan,
        BoundaryUse::NativeReturn,
        "serverStream test result",
    )
    .to_wire_json_internal_handle(&value, &mut heap)
    .expect("stream handle should materialize for test");
    let error = interpreter
        .stream_runtime
        .next(&stream_value)
        .await
        .expect_err("invalid chunk payload should fail stream polling");
    assert!(
        error.to_string().contains("decode") || error.to_string().contains("payload"),
        "unexpected decode error: {error}"
    );
    assert!(
        frame.outbound_requests.sender(&header.request_id).is_none(),
        "decode error should clear pending outbound response"
    );

    let cancel = tokio::time::timeout(std::time::Duration::from_secs(1), router_receiver.recv())
        .await
        .expect("request.cancel should be sent after decode error")
        .expect("router channel should stay open");
    let cancel_header = request_cancel_control(cancel);
    assert_eq!(cancel_header.request_id, header.request_id);
    assert_eq!(cancel_header.reason, "protocol_error");
}

#[tokio::test]
async fn runtime_program_service_dependency_expired_deadline_fails_before_send() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("unary")];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    frame.request.extra.insert(
        "deadline".to_string(),
        json!({
            "timeoutMs": 5000,
            "expiresAt": "2020-01-01T00:00:00Z"
        }),
    );
    let mut heap = RequestHeap::default();

    let error = crate::eval::service_dispatch::call_outbound_service(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &Env::default(),
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .await
    .expect_err("expired caller deadline should fail before outbound send");

    let payload = error.payload();
    assert_eq!(payload.code, "TimeoutError");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["reason"].as_str()),
        Some("deadlineExceeded")
    );
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn runtime_program_constructs_outbound_service_dependency_request_start() {
    let symbol = account_lookup_symbol();
    let call = outbound_service_dependency_call(symbol.clone());
    let mut program = program_with_executable(run_executable());
    program.service_dependencies = vec![account_service_dependency("unary")];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program.clone(), runtime_factory());
    let mut heap = RequestHeap::default();
    let frame = test_invocation_for_program("svc.main.run", runtime_activation);

    let (request, _payload) = outbound_control_and_payload_for_test(
        &interpreter,
        &outbound_context(&frame),
        &mut heap,
        &ExecutableAddr::service(0, 0),
        &call,
        &symbol,
        vec![RuntimeValue::String("user-1".to_string())],
    )
    .expect("service dependency outbound request should build");

    assert_eq!(request.service_id.as_deref(), Some("skiff.run/account"));
    assert_eq!(request.build_id, BUILD_OUTBOUND);
    assert_eq!(request.service_protocol_identity, PROTOCOL_OUTBOUND);
    assert_eq!(request.target, "lookup");
}

#[tokio::test]
async fn any_interface_remote_unary_dispatch_uses_operation_table_and_outbound() {
    let task = spawn_remote_any_program(
        remote_any_reader_call_executable("unary"),
        remote_reader_service_dependency("unary"),
    );
    let RemoteAnyProgramTask {
        handle,
        mut router_receiver,
        outbound_requests,
    } = task;

    let request = router_receiver
        .recv()
        .await
        .expect("remote any dispatch should send outbound request.start");
    let (header, _payload) = request_start_control(request);
    assert_eq!(header.mode, "unary");
    assert_eq!(header.target, REMOTE_READER_PUBLIC_PATH);
    assert_eq!(
        header.operation_abi_id.as_deref(),
        Some(REMOTE_READER_OPERATION_ABI_ID)
    );
    assert_eq!(
        header.selector.as_deref(),
        Some("operation:operation:account:reader.read")
    );
    send_outbound_unary_string_response(&outbound_requests, &header.request_id, "remote-ok");

    let value = handle
        .await
        .expect("remote any task should join")
        .expect("remote any unary dispatch should complete");
    assert_eq!(value, RuntimeValue::String("remote-ok".to_string()));
}

#[tokio::test]
async fn any_interface_remote_server_stream_dispatch_can_be_consumed_by_for() {
    let task = spawn_remote_any_program(
        remote_any_reader_stream_for_executable(),
        remote_reader_service_dependency("serverStream"),
    );
    let RemoteAnyProgramTask {
        handle,
        mut router_receiver,
        outbound_requests,
    } = task;

    let request = router_receiver
        .recv()
        .await
        .expect("remote any serverStream dispatch should send outbound request.start");
    let (header, _payload) = request_start_control(request);
    assert_eq!(header.mode, "serverStream");
    assert_eq!(
        header.operation_abi_id.as_deref(),
        Some(REMOTE_READER_OPERATION_ABI_ID)
    );
    send_outbound_stream_strings(&outbound_requests, &header.request_id, ["a", "b"]);

    let value = handle
        .await
        .expect("remote any stream task should join")
        .expect("remote any serverStream for-in should complete");
    assert_eq!(value, RuntimeValue::String("ab".to_string()));
}

#[tokio::test]
async fn any_interface_remote_direct_and_indirect_dispatch_hit_same_operation_on_distinct_paths() {
    let executable = remote_any_direct_then_indirect_executable();
    let expressions = &executable.body.expressions;
    assert!(matches!(
        &expressions[0],
        LinkedExprIr::Call {
            call: CallIr {
                target: LinkedCallTarget::ServiceDependencySymbol { .. },
                ..
            }
        }
    ));
    assert!(matches!(
        &expressions[3],
        LinkedExprIr::Call {
            call: CallIr {
                target: LinkedCallTarget::InterfaceMethod { .. },
                ..
            }
        }
    ));

    let task = spawn_remote_any_program(executable, remote_reader_service_dependency("unary"));
    let RemoteAnyProgramTask {
        handle,
        mut router_receiver,
        outbound_requests,
    } = task;

    let direct = router_receiver
        .recv()
        .await
        .expect("direct service dependency call should send first request");
    let (direct_header, _payload) = request_start_control(direct);
    assert_eq!(
        direct_header.operation_abi_id.as_deref(),
        Some(REMOTE_READER_OPERATION_ABI_ID)
    );
    send_outbound_unary_string_response(&outbound_requests, &direct_header.request_id, "D");

    let indirect = router_receiver
        .recv()
        .await
        .expect("indirect remote any call should send second request");
    let (indirect_header, _payload) = request_start_control(indirect);
    assert_eq!(
        indirect_header.operation_abi_id.as_deref(),
        Some(REMOTE_READER_OPERATION_ABI_ID)
    );
    assert_eq!(indirect_header.target, direct_header.target);
    send_outbound_unary_string_response(&outbound_requests, &indirect_header.request_id, "I");

    let value = handle
        .await
        .expect("direct/indirect task should join")
        .expect("direct/indirect dispatch should complete");
    assert_eq!(value, RuntimeValue::String("DI".to_string()));
}

struct ProgramTestInvocation {
    request: RequestEnvelope,
    operation: RuntimeOperation,
    route_addr: ExecutableAddr,
    receiver_const: Option<ConstAddr>,
    runtime_id: String,
    service_id: String,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
    service_http_response_max_bytes: usize,
    config: RuntimeConfigView,
    package_configs: Vec<RuntimeConfigView>,
    runtime_activation: Arc<RuntimeActivation>,
    service_db: Option<skiff_runtime_service_db::ServiceDbCapabilityFactory>,
    file_runtime: Arc<crate::host::file_runtime::FileRuntime>,
    db_request_state: Arc<tokio::sync::Mutex<skiff_runtime_service_db::DbRequestState>>,
    execution_budget: Arc<crate::execution_budget::ExecutionBudget>,
    request_heap_limits: RequestHeapLimits,
    router_sender: Option<tokio::sync::mpsc::UnboundedSender<crate::host::RouterWriterMessage>>,
    outbound_requests: Arc<crate::host::OutboundRequestRegistry>,
}

impl ProgramTestInvocation {
    fn execution_control(&self) -> crate::eval::capabilities::ExecutionControl<'_> {
        test_execution_control(self)
    }

    fn file_context(&self) -> crate::eval::capabilities::FileCapabilityContext {
        eval_capability_adapter::file_source(crate::capability_context::FileCapabilitySource::new(
            self.file_runtime.clone(),
        ))
        .context_for_request(test_db_context(self))
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> crate::eval::capabilities::FileSourceStreamContext<'_> {
        crate::eval::capabilities::FileSourceStreamContext::new(
            stream_runtime,
            test_execution_control(self),
        )
    }

    fn time_context(&self) -> crate::eval::capabilities::TimeCapabilityContext<'_> {
        crate::eval::capabilities::TimeCapabilityContext::new(test_execution_control(self))
    }

    fn websocket_context(&self) -> crate::eval::capabilities::WebsocketCapabilityContext<'_> {
        eval_capability_adapter::websocket_from_request(
            &self.service_id,
            None,
            self.router_sender.as_ref(),
        )
    }

    fn config_context(&self) -> crate::eval::capabilities::ConfigCapabilityContext<'_> {
        eval_capability_adapter::config_context(
            crate::capability_context::ConfigCapabilityContext::new(
                &self.config,
                &self.package_configs,
            ),
        )
    }

    fn telemetry_context(&self) -> Option<crate::telemetry::RequestTelemetryContext> {
        None
    }
}

fn test_invocation(target: &str) -> ProgramTestInvocation {
    let operation_abi_id = format!("operation:{target}");
    let cancellation = CancellationToken::new();
    let cancelled = cancellation.cancel_flag();
    ProgramTestInvocation {
        request: RequestEnvelope {
            request_id: "request-program".to_string(),
            mode: "unary".to_string(),
            target: target.to_string(),
            operation_abi_id: Some(operation_abi_id.clone()),
            selector: Some(format!("operation:{operation_abi_id}")),
            service_id: None,
            build_id: "build:program".to_string(),
            service_protocol_identity: String::new(),
            contract_identity: None,
            activation_identity: None,
            http_adapter: None,
            websocket_adapter: None,
            binary_http: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        },
        operation: RuntimeOperation {
            operation_abi_id: Some(operation_abi_id),
            operation: "run".to_string(),
            target: target.to_string(),
            mode: "unary".to_string(),
            parameters: Vec::new(),
            service_protocol_identity: None,
            extra: serde_json::Map::new(),
        },
        route_addr: ExecutableAddr::service(0, 0),
        receiver_const: None,
        runtime_id: "runtime-program".to_string(),
        service_id: "svc".to_string(),
        cancellation,
        cancelled,
        service_http_response_max_bytes: DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        config: RuntimeConfigView::empty(),
        package_configs: Vec::new(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: String::new(),
                display_name: None,
                metadata: Default::default(),
            },
            version: String::new(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: GatewayConfig::default(),
        }),
        service_db: None,
        file_runtime: Arc::new(crate::host::file_runtime::FileRuntime::new(
            None,
            std::env::temp_dir().join("skiff-runtime-test-file-tmp"),
        )),
        db_request_state: Arc::new(tokio::sync::Mutex::new(
            skiff_runtime_service_db::DbRequestState::default(),
        )),
        execution_budget: Arc::new(crate::execution_budget::ExecutionBudget::disabled()),
        request_heap_limits: RequestHeapLimits::default(),
        router_sender: None,
        outbound_requests: Arc::new(crate::host::OutboundRequestRegistry::default()),
    }
}

fn test_invocation_for_program(
    target: &str,
    runtime_activation: RuntimeActivation,
) -> ProgramTestInvocation {
    let mut invocation = test_invocation(target);
    invocation.runtime_activation = Arc::new(runtime_activation);
    invocation.service_id = invocation.runtime_activation.service.id.clone();
    invocation
}

fn runtime_activation_from_program(program: &RuntimeProgram) -> RuntimeActivation {
    RuntimeActivation {
        service: program.service.clone(),
        version: program.version.clone(),
        package_configs: Vec::new(),
        service_dependencies: program.service_dependencies.clone(),
        timeout: program.timeout.clone(),
        operation_route_bindings: program.operation_route_bindings.clone(),
        db: program.db.clone(),
        actors: program.actors.clone(),
        gateway: program.gateway.clone(),
    }
}

fn concrete_execution_control(
    frame: &ProgramTestInvocation,
) -> crate::request::ExecutionControl<'_> {
    crate::request::ExecutionControl::new(frame.cancellation.clone(), &frame.execution_budget)
}

fn test_execution_control(
    frame: &ProgramTestInvocation,
) -> crate::eval::capabilities::ExecutionControl<'_> {
    eval_capability_adapter::execution_control(concrete_execution_control(frame))
}

fn test_db_context(
    frame: &ProgramTestInvocation,
) -> crate::eval::capabilities::DbCapabilityContext {
    eval_capability_adapter::db_context(
        crate::capability_context::DbCapabilityContext::from_handle(
            skiff_runtime_service_db::ServiceDbCapabilityHandle::with_state(
                frame.service_db.clone(),
                frame.db_request_state.clone(),
            ),
        ),
    )
}

fn test_outbound_context(frame: &ProgramTestInvocation) -> OutboundServiceContext {
    let execution = concrete_execution_control(frame);
    eval_capability_adapter::outbound(
        eval_capability_adapter::outbound_service_context_from_request(
            &frame.request,
            frame.operation.target.as_str(),
            frame.execution_budget.clone(),
            execution.cancellation_token(),
            frame.request_heap_limits.clone(),
            frame.router_sender.clone(),
            frame.outbound_requests.clone(),
            &frame.runtime_activation.service_dependencies,
            &frame.runtime_activation.timeout,
        ),
    )
}

fn outbound_context(frame: &ProgramTestInvocation) -> OutboundServiceContext {
    test_outbound_context(frame)
}

struct RemoteAnyProgramTask {
    handle: tokio::task::JoinHandle<crate::eval::error::Result<RuntimeValue>>,
    router_receiver: tokio::sync::mpsc::UnboundedReceiver<crate::host::RouterWriterMessage>,
    outbound_requests: Arc<crate::host::OutboundRequestRegistry>,
}

fn spawn_remote_any_program(
    executable: LinkedExecutable,
    dependency: ServiceDependencyConstraint,
) -> RemoteAnyProgramTask {
    let mut program = program_with_executable(executable);
    program.service_dependencies = vec![dependency];
    let program = Arc::new(program);
    let runtime_activation = runtime_activation_from_program(&program);
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let (sender, router_receiver) = tokio::sync::mpsc::unbounded_channel();
    let mut frame = test_invocation_for_program("svc.main.run", runtime_activation);
    frame.router_sender = Some(sender);
    let invocation_context = program_invocation_context(&interpreter, &frame);
    let context = invocation_context.execution_context();
    let context = Arc::new(OwnedProgramExecutionContext::capture(&context));
    let outbound_requests = frame.outbound_requests.clone();
    let handle = tokio::spawn(async move {
        let context = context.borrow();
        let mut heap = context.request_heap();
        let run_addr = ExecutableAddr::service(0, 0);
        interpreter
            .call_program_executable(
                context,
                &mut heap,
                &Env::new(),
                &run_addr,
                &run_addr,
                &BTreeMap::new(),
                Vec::new(),
            )
            .await
    });
    RemoteAnyProgramTask {
        handle,
        router_receiver,
        outbound_requests,
    }
}

fn send_outbound_unary_string_response(
    outbound_requests: &crate::host::OutboundRequestRegistry,
    request_id: &str,
    value: &str,
) {
    let payload = encode_string_payload(value);
    outbound_requests
        .sender(request_id)
        .expect("outbound response should be registered")
        .send(crate::request::OutboundResponse::End { payload })
        .expect("unary outbound response should send");
}

fn send_outbound_stream_strings<const N: usize>(
    outbound_requests: &crate::host::OutboundRequestRegistry,
    request_id: &str,
    values: [&str; N],
) {
    let outbound = outbound_requests
        .sender(request_id)
        .expect("serverStream outbound response should be registered");
    outbound
        .send(crate::request::OutboundResponse::Start {
            http_response: crate::request::HttpResponseMetadata {
                status: 200,
                headers: Vec::new(),
            },
        })
        .expect("response.start should send");
    for (seq, value) in values.into_iter().enumerate() {
        outbound
            .send(crate::request::OutboundResponse::Chunk {
                seq: seq as u64,
                payload: encode_string_payload(value),
            })
            .expect("response.chunk should send");
    }
    outbound
        .send(crate::request::OutboundResponse::End {
            payload: Vec::new(),
        })
        .expect("response.end should send");
}

fn encode_string_payload(value: &str) -> Vec<u8> {
    let plan =
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::native("string"))
            .expect("string payload plan should build");
    encode_payload_plan(
        &RuntimeValue::String(value.to_string()),
        &plan,
        &PayloadBoundary::runtime_internal(),
        &RequestHeap::default(),
    )
    .expect("string payload should encode")
}

fn request_start_control(
    message: crate::host::RouterWriterMessage,
) -> (crate::request::RequestStartControl, Vec<u8>) {
    match message {
        crate::host::RouterWriterMessage::Control(
            crate::request::OutboundControlMessage::RequestStart { request, payload },
        ) => (request, payload),
        other => panic!("expected request.start control command, got {other:?}"),
    }
}

fn request_cancel_control(
    message: crate::host::RouterWriterMessage,
) -> crate::request::RequestCancelControl {
    match message {
        crate::host::RouterWriterMessage::Control(
            crate::request::OutboundControlMessage::RequestCancel { request },
        ) => request,
        other => panic!("expected request.cancel control command, got {other:?}"),
    }
}

async fn execute_test_program_route(
    interpreter: &Interpreter,
    frame: &ProgramTestInvocation,
) -> crate::eval::error::Result<Value> {
    let context = program_invocation_context(interpreter, frame);
    interpreter
        .execute_program_addr_with_receiver_const(
            &context,
            &frame.route_addr,
            frame.receiver_const.as_ref(),
        )
        .await
}

fn program_invocation_context<'a>(
    interpreter: &Interpreter,
    frame: &'a ProgramTestInvocation,
) -> ProgramInvocationContext<'a> {
    let execution = frame.execution_control();
    let actor = eval_capability_adapter::actor_from_request(
        &frame.runtime_id,
        &frame.service_id,
        "0.0.0-test",
        &frame.request,
        &frame.operation,
        frame.router_sender.as_ref(),
        &frame.outbound_requests,
        execution.cancellation_token(),
    );
    let effects = eval_capability_adapter::effects(
        eval_capability_adapter::effect_dispatch_context_from_request(
            &frame.request,
            frame.service_http_response_max_bytes,
            execution.cancellation_token(),
            frame.telemetry_context(),
            skiff_runtime_capability_context::HttpRuntimeOptions::from_env(),
        ),
    );
    let execution_input = ProgramExecutionInput {
        execution: execution.clone(),
        config: frame.config_context(),
        db: test_db_context(frame),
        file: frame.file_context(),
        file_source_stream: frame.file_source_stream_context(interpreter.stream_runtime.clone()),
        time: frame.time_context(),
        websocket: frame.websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: frame.runtime_activation.clone(),
        actor: actor.clone(),
        spawn: actor,
        outbound: test_outbound_context(frame),
        request_heap_limits: frame.request_heap_limits.clone(),
    };
    ProgramInvocationContext::new(ProgramInvocationInput {
        request: crate::request::request_payload_context_from_request(&frame.request),
        operation: frame.operation.operation.as_str(),
        execution: execution_input,
        http_response_max_bytes: frame.service_http_response_max_bytes,
        request_heap_limits: frame.request_heap_limits.clone(),
    })
}

fn set_request_string_arg(frame: &mut ProgramTestInvocation, name: &str, value: &str) {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            name: { "kind": "builtin", "name": "Json", "args": [] }
        }
    });
    let mut heap = RequestHeap::default();
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            name.to_string(),
            RuntimeValue::String(value.to_string()),
        )])))
        .expect("test args record should allocate");
    frame.request.payload_bytes =
        encode_payload(&RuntimeValue::Heap(args_handle), &descriptor, &heap)
            .expect("test args payload should encode");
}

fn set_request_http_arg(frame: &mut ProgramTestInvocation, name: &str) {
    let descriptor = json!({
        "kind": "record",
        "fields": {
            name: std_http_request_descriptor_for_payload()
        }
    });
    let mut heap = RequestHeap::default();
    let request = http_request_runtime_value(&mut heap);
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            name.to_string(),
            request,
        )])))
        .expect("test args record should allocate");
    frame.request.payload_bytes =
        encode_payload(&RuntimeValue::Heap(args_handle), &descriptor, &heap)
            .expect("test HTTP args payload should encode");
}

fn std_http_request_descriptor_for_payload() -> Value {
    json!({
        "kind": "record",
        "fields": {
            "method": { "kind": "builtin", "name": "string", "args": [] },
            "url": { "kind": "builtin", "name": "string", "args": [] },
            "path": { "kind": "builtin", "name": "string", "args": [] },
            "query": {
                "kind": "builtin",
                "name": "Array",
                "args": [
                    {
                        "kind": "record",
                        "fields": {
                            "name": { "kind": "builtin", "name": "string", "args": [] },
                            "value": { "kind": "builtin", "name": "string", "args": [] }
                        }
                    }
                ]
            },
            "headers": {
                "kind": "builtin",
                "name": "Array",
                "args": [std_http_header_descriptor_for_payload()]
            },
            "body": { "kind": "builtin", "name": "bytes", "args": [] }
        }
    })
}

fn std_http_header_descriptor_for_payload() -> Value {
    json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] },
            "value": { "kind": "builtin", "name": "string", "args": [] }
        }
    })
}

fn http_request_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    let query = heap
        .alloc_array(Vec::new())
        .expect("test query array should allocate");
    let headers = heap
        .alloc_array(Vec::new())
        .expect("test headers array should allocate");
    let body = heap
        .alloc_bytes(b"hello world".as_slice())
        .expect("test bytes body should allocate");
    let request = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            (
                "method".to_string(),
                RuntimeValue::String("POST".to_string()),
            ),
            (
                "url".to_string(),
                RuntimeValue::String("https://example.test/upload".to_string()),
            ),
            (
                "path".to_string(),
                RuntimeValue::String("/upload".to_string()),
            ),
            ("query".to_string(), RuntimeValue::Heap(query)),
            ("headers".to_string(), RuntimeValue::Heap(headers)),
            ("body".to_string(), RuntimeValue::Heap(body)),
        ])))
        .expect("test http request object should allocate");
    RuntimeValue::Heap(request)
}

fn http_client_request_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
    let headers = heap
        .alloc_array(Vec::new())
        .expect("test headers array should allocate");
    let body = heap
        .alloc_bytes(b"hello world".as_slice())
        .expect("test bytes body should allocate");
    let request = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            (
                "method".to_string(),
                RuntimeValue::String("POST".to_string()),
            ),
            (
                "url".to_string(),
                RuntimeValue::String("https://example.test/upload".to_string()),
            ),
            ("headers".to_string(), RuntimeValue::Heap(headers)),
            ("body".to_string(), RuntimeValue::Heap(body)),
            ("timeoutMs".to_string(), RuntimeValue::Null),
        ])))
        .expect("test http client request object should allocate");
    RuntimeValue::Heap(request)
}

fn db_metadata(mut value: Value) -> Vec<DbMetadataIr> {
    let entries = value
        .as_array_mut()
        .expect("test db metadata should be an array");
    for entry in entries {
        let object = entry
            .as_object_mut()
            .expect("test db metadata entry should be an object");
        object
            .entry("modulePath")
            .or_insert_with(|| Value::String("svc.main".to_string()));
        object
            .entry("sourceRole")
            .or_insert_with(|| Value::String("service".to_string()));
        let type_name = object
            .get("typeName")
            .and_then(Value::as_str)
            .expect("test db metadata entry should have typeName")
            .to_string();
        object.entry("type").or_insert_with(|| {
            json!({
                "kind": "dbObjectSymbol",
                "symbol": { "modulePath": "svc.main", "symbol": type_name }
            })
        });
        object
            .entry("collectionName")
            .or_insert_with(|| Value::String(type_name));
        if let Some(key) = object.get_mut("key").and_then(Value::as_object_mut) {
            key.entry("type")
                .or_insert_with(|| json!({ "kind": "builtin", "name": "string" }));
        }
        object.entry("leases").or_insert_with(|| json!([]));
        object.entry("indexes").or_insert_with(|| json!([]));
    }
    serde_json::from_value(value).expect("test db metadata should decode as typed IR")
}

fn thread_db_metadata() -> Vec<DbMetadataIr> {
    db_metadata(json!([
        {
            "kind": "object",
            "typeName": "Thread",
            "collectionName": "Thread",
            "key": { "name": "id", "type": { "kind": "builtin", "name": "string" } },
            "fields": [
                { "name": "title", "type": { "kind": "builtin", "name": "string" } },
                { "name": "status", "type": { "kind": "builtin", "name": "string" } },
                { "name": "score", "type": { "kind": "builtin", "name": "number" } },
                { "name": "archived", "type": { "kind": "builtin", "name": "boolean" } },
                { "name": "tag", "type": { "kind": "builtin", "name": "string" } },
                { "name": "visitCount", "type": { "kind": "builtin", "name": "number" } },
                { "name": "lastSeenAt", "type": { "kind": "builtin", "name": "string" } },
                { "name": "createdAt", "type": { "kind": "builtin", "name": "string" } },
                { "name": "optional", "type": { "kind": "builtin", "name": "boolean" } }
            ],
            "indexes": []
        }
    ]))
}

fn program_with_executables(executables: Vec<LinkedExecutable>) -> RuntimeProgram {
    let addr = ExecutableAddr::service(0, 0);
    RuntimeProgram {
        service: ServiceMeta {
            id: "svc".to_string(),
            display_name: Some("Service".to_string()),
            metadata: Default::default(),
        },
        version: "v1".to_string(),
        build_id: "build:program".to_string(),
        service_files: vec![Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:svc".to_string(),
            source_ast_hash: "source:svc".to_string(),
            module_path: "svc.main".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables,
            external_refs: Default::default(),
        })],
        packages: Vec::new(),
        package_files: Vec::new(),
        service_resources: Default::default(),
        package_resources: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        routes: HashMap::from([("svc.main.run".to_string(), addr.clone())]),
        spawn_routes: HashMap::new(),
        operations: HashMap::from([("run".to_string(), addr)]),
        operation_receivers: HashMap::new(),
        db: Vec::new(),
        actors: Vec::new(),
        link_overlay: LinkOverlay::default(),
        gateway: GatewayConfig::default(),
        types: RuntimeTypeContext::default(),
    }
}

fn program_with_executables_and_std_http_types(
    executables: Vec<LinkedExecutable>,
) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    install_std_http_types(&mut program);
    program
}

fn program_with_executable_and_std_http_types(executable: LinkedExecutable) -> RuntimeProgram {
    program_with_executables_and_std_http_types(vec![executable])
}

fn install_std_http_types(program: &mut RuntimeProgram) {
    let package_slot = program.packages.len();
    assert_eq!(
        package_slot, 0,
        "std HTTP test fixture currently expects an otherwise package-free program"
    );
    program
        .packages
        .push(Arc::new(package_unit("skiff.run/std")));
    program.package_resources.push(Default::default());
    program
        .link_overlay
        .package_slots_by_id
        .insert("skiff.run/std".to_string(), package_slot);
    program
        .link_overlay
        .package_slots_by_dependency_ref
        .insert("std".to_string(), package_slot);

    let declarations = std_http_type_declarations(package_slot);
    program.package_files.push(vec![Arc::new(std_http_file_unit(
        declarations
            .iter()
            .map(|(_, declaration)| declaration.clone())
            .collect(),
    ))]);
    for (index, (symbol_path, declaration)) in declarations.into_iter().enumerate() {
        let addr = std_http_type_addr_for_package(package_slot, index);
        program.types.descriptors.insert(addr.clone(), declaration);
        program.types.exported_types.insert_package(
            PackageSymbolKey::new(package_slot, symbol_path),
            addr.clone(),
        );
        if let Some(short_path) = symbol_path.strip_prefix("std.") {
            program
                .types
                .exported_types
                .insert_package(PackageSymbolKey::new(package_slot, short_path), addr);
        }
    }
}

fn program_with_executables_and_local_error_type(
    executables: Vec<LinkedExecutable>,
    error_type_name: &str,
) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    let file = Arc::make_mut(
        program
            .service_files
            .get_mut(0)
            .expect("test program should have a service file"),
    );
    file.types.push(crate::eval::program::TypeDeclIr {
        name: error_type_name.to_string(),
        descriptor: LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([(
                "message".to_string(),
                LinkedTypeRef::Native {
                    name: "string".to_string(),
                    args: Vec::new(),
                },
            )]),
        },
        ..crate::eval::program::TypeDeclIr::default()
    });
    program.types.descriptors.insert(
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(0),
            type_index: 0,
        },
        file.types[0].clone(),
    );
    program
}

fn program_with_two_same_named_error_types(executables: Vec<LinkedExecutable>) -> RuntimeProgram {
    let mut program = program_with_executables(executables);
    let file = Arc::make_mut(
        program
            .service_files
            .get_mut(0)
            .expect("test program should have a service file"),
    );
    for _ in 0..2 {
        file.types.push(crate::eval::program::TypeDeclIr {
            name: "AuthError".to_string(),
            descriptor: local_error_descriptor(),
            ..crate::eval::program::TypeDeclIr::default()
        });
    }
    for type_index in 0..2 {
        program.types.descriptors.insert(
            service_type_addr(type_index),
            file.types[type_index].clone(),
        );
    }
    program
}

fn local_error_descriptor() -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: BTreeMap::from([(
            "message".to_string(),
            LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        )]),
    }
}

fn service_type_addr(type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn std_http_type_addr(type_index: usize) -> TypeAddr {
    std_http_type_addr_for_package(0, type_index)
}

fn std_http_type_addr_for_package(package_slot: usize, type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(package_slot),
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn std_http_type_ref(type_index: usize) -> LinkedTypeRef {
    LinkedTypeRef::Address {
        addr: std_http_type_addr(type_index),
    }
}

fn std_http_type_plan_for_test(
    program: &RuntimeProgram,
    addr: &ExecutableAddr,
    type_index: usize,
) -> RuntimeTypePlan {
    let image = program.linked_image();
    RuntimeTypePlan::from_linked(
        &std_http_type_ref(type_index),
        &PlanContext::new(&image, addr),
    )
    .expect("std HTTP fixture type plan should build")
}

fn std_http_file_unit(types: Vec<TypeDeclIr>) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:std-http".to_string(),
        source_ast_hash: "source:std-http".to_string(),
        module_path: "std.http".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        types,
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: Default::default(),
    }
}

fn std_http_type_declarations(package_slot: usize) -> Vec<(&'static str, TypeDeclIr)> {
    let header = LinkedTypeRef::Address {
        addr: std_http_type_addr_for_package(package_slot, STD_HTTP_HEADER_TYPE_INDEX),
    };
    let query_param = LinkedTypeRef::Address {
        addr: std_http_type_addr_for_package(package_slot, STD_HTTP_QUERY_PARAM_TYPE_INDEX),
    };
    vec![
        (
            "std.http.HttpHeader",
            anonymous_type_decl(
                "std.http.HttpHeader",
                linked_record_descriptor(vec![
                    ("name", linked_builtin_type("string")),
                    ("value", linked_builtin_type("string")),
                ]),
            ),
        ),
        (
            "std.http.HttpQueryParam",
            anonymous_type_decl(
                "std.http.HttpQueryParam",
                linked_record_descriptor(vec![
                    ("name", linked_builtin_type("string")),
                    ("value", linked_builtin_type("string")),
                ]),
            ),
        ),
        (
            "std.http.HttpRequest",
            anonymous_type_decl(
                "std.http.HttpRequest",
                linked_record_descriptor(vec![
                    ("method", linked_builtin_type("string")),
                    ("url", linked_builtin_type("string")),
                    ("path", linked_builtin_type("string")),
                    ("query", linked_array_type(query_param.clone())),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpResponse",
            anonymous_type_decl(
                "std.http.HttpResponse",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpResponseStreamEvent",
            anonymous_type_decl(
                "std.http.HttpResponseStreamEvent",
                LinkedTypeDescriptor::Union {
                    variants: vec![
                        linked_record_type(vec![
                            ("tag", linked_literal_string("start")),
                            ("status", linked_builtin_type("integer")),
                            ("headers", linked_array_type(header.clone())),
                        ]),
                        linked_record_type(vec![
                            ("tag", linked_literal_string("chunk")),
                            ("value", linked_builtin_type("bytes")),
                        ]),
                        linked_record_type(vec![("tag", linked_literal_string("end"))]),
                    ],
                },
            ),
        ),
        (
            "std.http.HttpClientRequest",
            anonymous_type_decl(
                "std.http.HttpClientRequest",
                linked_record_descriptor(vec![
                    ("method", linked_builtin_type("string")),
                    ("url", linked_builtin_type("string")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_nullable_type(linked_builtin_type("bytes"))),
                    (
                        "timeoutMs",
                        linked_nullable_type(linked_builtin_type("integer")),
                    ),
                ]),
            ),
        ),
        (
            "std.http.HttpClientResponse",
            anonymous_type_decl(
                "std.http.HttpClientResponse",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_builtin_type("bytes")),
                ]),
            ),
        ),
        (
            "std.http.HttpClientStreamHandle",
            anonymous_type_decl(
                "std.http.HttpClientStreamHandle",
                linked_record_descriptor(vec![
                    ("status", linked_builtin_type("integer")),
                    ("headers", linked_array_type(header.clone())),
                    ("body", linked_stream_type(linked_builtin_type("bytes"))),
                ]),
            ),
        ),
        (
            "std.http.HttpSseEvent",
            anonymous_type_decl(
                "std.http.HttpSseEvent",
                LinkedTypeDescriptor::Union {
                    variants: vec![
                        linked_record_type(vec![
                            ("tag", linked_literal_string("response")),
                            ("status", linked_builtin_type("integer")),
                            ("headers", linked_array_type(header)),
                        ]),
                        linked_record_type(vec![
                            ("tag", linked_literal_string("body")),
                            ("value", linked_builtin_type("bytes")),
                        ]),
                        linked_record_type(vec![
                            ("tag", linked_literal_string("event")),
                            ("event", linked_nullable_type(linked_builtin_type("string"))),
                            ("id", linked_nullable_type(linked_builtin_type("string"))),
                            ("data", linked_builtin_type("string")),
                        ]),
                    ],
                },
            ),
        ),
    ]
}

fn linked_record_descriptor(fields: Vec<(&str, LinkedTypeRef)>) -> LinkedTypeDescriptor {
    LinkedTypeDescriptor::Record {
        fields: linked_field_map(fields),
    }
}

fn linked_record_type(fields: Vec<(&str, LinkedTypeRef)>) -> LinkedTypeRef {
    LinkedTypeRef::Record {
        fields: linked_field_map(fields),
    }
}

fn linked_field_map(fields: Vec<(&str, LinkedTypeRef)>) -> BTreeMap<String, LinkedTypeRef> {
    fields
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty))
        .collect()
}

fn linked_array_type(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Array".to_string(),
        args: vec![item],
    }
}

fn linked_nullable_type(inner: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Nullable {
        inner: Box::new(inner),
    }
}

fn linked_stream_type(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Stream".to_string(),
        args: vec![item],
    }
}

fn linked_literal_string(value: &str) -> LinkedTypeRef {
    LinkedTypeRef::Literal {
        value: LiteralIr::String {
            value: value.to_string(),
        },
    }
}

fn runtime_error_leaf(error: &RuntimeError) -> &RuntimeError {
    unwrap_diagnostic_source_context(error)
}

fn assert_no_legacy_skiff_type_key(value: &Value) {
    match value {
        Value::Object(object) => {
            assert!(
                !object.contains_key("__skiffType"),
                "exception envelope must not contain legacy __skiffType metadata: {value}"
            );
            for child in object.values() {
                assert_no_legacy_skiff_type_key(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                assert_no_legacy_skiff_type_key(child);
            }
        }
        _ => {}
    }
}

fn assert_unsupported_foreground_wait_error(error: &RuntimeError) {
    assert!(
        error
            .to_string()
            .contains("foreground/activate wait until parking is unsupported in this runtime path"),
        "unexpected error: {error}"
    );
}

fn program_with_service_and_package_executables(
    service_executable: LinkedExecutable,
    package_executable: LinkedExecutable,
) -> RuntimeProgram {
    let mut program = program_with_executable(service_executable);
    program.package_files = vec![vec![Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:pkg".to_string(),
        source_ast_hash: "source:pkg".to_string(),
        module_path: "pkg.main".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![package_executable],
        external_refs: Default::default(),
    })]];
    program
}

fn package_unit(package_id: &str) -> PackageUnit {
    PackageUnit::empty(
        package_id,
        "1.0.0",
        format!("{package_id}:build"),
        format!("{package_id}:abi"),
    )
}

fn program_with_executable(executable: LinkedExecutable) -> RuntimeProgram {
    program_with_executables(vec![executable])
}

fn executable_body(value: Value) -> LinkedExecutableBody {
    serde_json::from_value(value).expect("typed executable body should deserialize")
}

fn expression(value: Value) -> LinkedExprIr {
    serde_json::from_value(value).expect("typed expression should deserialize")
}

fn statement(value: Value) -> LinkedStmtIr {
    serde_json::from_value(value).expect("typed statement should deserialize")
}

fn outbound_service_dependency_call(
    symbol: ServiceDependencySymbolRef,
) -> crate::eval::program::CallIr {
    crate::eval::program::CallIr {
        target: crate::eval::program::LinkedCallTarget::ServiceDependencySymbol { symbol },
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn account_service_dependency(mode: &str) -> ServiceDependencyConstraint {
    let operation = account_lookup_operation_ref();
    let return_type = match mode {
        "serverStream" => skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("string")],
        },
        _ => skiff_artifact_model::TypeRefIr::native("string"),
    };
    let public_signature = skiff_artifact_model::CanonicalPublicCallableSignature {
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "userId".to_string(),
            ty: skiff_artifact_model::TypeRefIr::native("string"),
        }],
        return_type,
        may_suspend: false,
    };
    ServiceDependencyConstraint {
        id: "skiff.run/account".to_string(),
        version: "0.1.0".to_string(),
        alias: "account".to_string(),
        build_id: BUILD_OUTBOUND.to_string(),
        service_protocol_identity: PROTOCOL_OUTBOUND.to_string(),
        publication_abi: skiff_artifact_model::PublicationAbiUnit {
            operation_exports: vec![operation.clone()],
            operation_abi: vec![skiff_artifact_model::PublicationOperationAbi {
                operation: operation.clone(),
                public_signature,
                schema_closure: Vec::new(),
                stream_effect_throw_config: BTreeMap::new(),
            }],
            ..Default::default()
        },
    }
}

fn remote_reader_service_dependency(mode: &str) -> ServiceDependencyConstraint {
    let operation = remote_reader_operation_ref();
    let return_type = match mode {
        "serverStream" => skiff_artifact_model::TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::native("string")],
        },
        _ => skiff_artifact_model::TypeRefIr::native("string"),
    };
    let public_signature = skiff_artifact_model::CanonicalPublicCallableSignature {
        params: Vec::new(),
        return_type,
        may_suspend: false,
    };
    ServiceDependencyConstraint {
        id: "skiff.run/account".to_string(),
        version: "0.1.0".to_string(),
        alias: "account".to_string(),
        build_id: BUILD_OUTBOUND.to_string(),
        service_protocol_identity: PROTOCOL_OUTBOUND.to_string(),
        publication_abi: skiff_artifact_model::PublicationAbiUnit {
            operation_exports: vec![operation.clone()],
            operation_abi: vec![skiff_artifact_model::PublicationOperationAbi {
                operation: operation.clone(),
                public_signature,
                schema_closure: Vec::new(),
                stream_effect_throw_config: BTreeMap::new(),
            }],
            public_instances: vec![skiff_artifact_model::PublicationPublicInstanceExport {
                public_instance_key: REMOTE_READER_PUBLIC_INSTANCE.to_string(),
                interfaces: vec![remote_reader_interface_ref()],
                source_call_method_index: vec![skiff_artifact_model::SourceCallMethodIndexEntry {
                    method_name: "read".to_string(),
                    operation: operation.clone(),
                }],
                method_operations: vec![operation],
            }],
            ..Default::default()
        },
    }
}

fn remote_any_reader_call_executable(mode: &str) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_builtin_type("string")),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 0 },
                    "interface": remote_reader_interface_json(),
                    "source": remote_reader_box_source(mode)
                },
                {
                    "kind": "call",
                    "call": {
                        "target": remote_reader_interface_method_target(),
                        "args": [
                            { "expression": 1 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn remote_any_reader_stream_for_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_builtin_type("string")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "out".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "item".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 3 }
                    ]
                },
                {
                    "label": "body",
                    "statements": [
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 0,
                    "value": { "expression": 3 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "itemType": serde_json::to_value(linked_builtin_type("string")).unwrap(),
                    "iterable": { "expression": 2 },
                    "body": "body"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 0 },
                    "value": { "expression": 6 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 0 },
                    "interface": remote_reader_interface_json(),
                    "source": remote_reader_box_source("serverStream")
                },
                {
                    "kind": "call",
                    "call": {
                        "target": remote_reader_interface_method_target(),
                        "args": [
                            { "expression": 1 }
                        ]
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 4 },
                    "right": { "expression": 5 }
                },
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn remote_any_direct_then_indirect_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_builtin_type("string")),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 4 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "serviceDependencySymbol",
                            "symbol": serde_json::to_value(remote_reader_symbol()).unwrap()
                        },
                        "args": []
                    }
                },
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 1 },
                    "interface": remote_reader_interface_json(),
                    "source": remote_reader_box_source("unary")
                },
                {
                    "kind": "call",
                    "call": {
                        "target": remote_reader_interface_method_target(),
                        "args": [
                            { "expression": 2 }
                        ]
                    }
                },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 0 },
                    "right": { "expression": 3 }
                }
            ]
        })),
    }
}

fn remote_reader_box_source(mode: &str) -> serde_json::Value {
    let return_type = match mode {
        "serverStream" => linked_stream_type(linked_builtin_type("string")),
        _ => linked_builtin_type("string"),
    };
    json!({
        "kind": "remote",
        "dependencyRef": "account",
        "publicInstanceKey": REMOTE_READER_PUBLIC_INSTANCE,
        "operations": {
            "interface": remote_reader_interface_json(),
            "slots": [
                {
                    "slot": 0,
                    "methodAbiId": REMOTE_READER_METHOD_ABI_ID,
                    "signature": {
                        "params": [],
                        "returnType": serde_json::to_value(return_type).unwrap()
                    },
                    "operationAbiId": REMOTE_READER_OPERATION_ABI_ID
                }
            ]
        },
        "calleeProtocolIdentity": PROTOCOL_OUTBOUND
    })
}

fn old_db_builtin_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "unused" } },
                db_call_expr_without_type("db.create", [json!({ "expression": 0 })])
            ]
        })),
    }
}

fn db_negative_offset_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "mapLiteral", "entries": {} },
                { "kind": "literal", "value": { "kind": "number", "value": -1 } },
                {
                    "kind": "dbOperation",
                    "operation": {
                        "op": "find",
                        "many": true,
                        "target": {
                            "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
                            "typeName": "Thread"
                        },
                        "query": {
                            "order": [
                                {
                                    "field": { "text": "score", "segments": ["score"] },
                                    "direction": "desc"
                                }
                            ],
                            "offset": { "expression": 1 }
                        },
                        "resultType": { "kind": "builtin", "name": "Json" }
                    }
                }
            ]
        })),
    }
}

fn db_after_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "mapLiteral", "entries": {} },
                { "kind": "literal", "value": { "kind": "string", "value": "old-page" } },
                {
                    "kind": "dbOperation",
                    "operation": {
                        "op": "find",
                        "many": true,
                        "target": {
                            "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
                            "typeName": "Thread"
                        },
                        "query": {
                            "after": { "expression": 1 }
                        },
                        "resultType": { "kind": "builtin", "name": "Json" }
                    }
                }
            ]
        })),
    }
}

fn db_query_value_executable() -> LinkedExecutable {
    let mut expressions = Vec::new();
    let true_condition = push_expr(&mut expressions, literal_bool_expr(true));
    let false_condition = push_expr(&mut expressions, literal_bool_expr(false));
    let status_open = push_expr(&mut expressions, literal_string_expr("open"));
    let score_gt = push_expr(&mut expressions, literal_number_expr(10));
    let limit = push_expr(&mut expressions, literal_number_expr(5));
    let offset = push_expr(&mut expressions, literal_number_expr(2));
    let after = push_expr(&mut expressions, literal_string_expr("cursor-1"));

    let status_predicate = db_predicate_compare("status", "eq", expr_ref_json(status_open));
    let true_only = push_expr(
        &mut expressions,
        db_query_value_expr(db_query(vec![db_predicate_conditional(
            expr_ref_json(true_condition),
            status_predicate.clone(),
        )])),
    );
    let false_only = push_expr(
        &mut expressions,
        db_query_value_expr(db_query(vec![db_predicate_conditional(
            expr_ref_json(false_condition),
            status_predicate.clone(),
        )])),
    );
    let mixed = push_expr(
        &mut expressions,
        db_query_value_expr(json!({
            "where": [
                db_predicate_compare("score", "gt", expr_ref_json(score_gt)),
                db_predicate_conditional(expr_ref_json(false_condition), status_predicate.clone()),
                db_predicate_conditional(expr_ref_json(true_condition), status_predicate)
            ],
            "order": [
                {
                    "field": { "text": "score", "segments": ["score"] },
                    "direction": "desc"
                }
            ],
            "limit": expr_ref_json(limit),
            "offset": expr_ref_json(offset),
            "after": expr_ref_json(after)
        })),
    );
    let result = push_expr(
        &mut expressions,
        json!({
            "kind": "mapLiteral",
            "entries": {
                "trueOnly": expr_ref_json(true_only),
                "falseOnly": expr_ref_json(false_only),
                "mixed": expr_ref_json(mixed)
            }
        }),
    );

    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": result }
                }
            ],
            "expressions": expressions
        })),
    }
}

fn db_many_key_selector_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "thread-1" } },
                {
                    "kind": "dbOperation",
                    "operation": {
                        "op": "find",
                        "many": true,
                        "target": {
                            "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
                            "typeName": "Thread"
                        },
                        "selector": { "kind": "key", "value": { "expression": 0 } },
                        "resultType": { "kind": "builtin", "name": "Json" }
                    }
                }
            ]
        })),
    }
}

fn db_call_expr_without_type<const N: usize>(op: &str, args: [Value; N]) -> Value {
    let args = args.into_iter().collect::<Vec<_>>();
    json!({
        "kind": "call",
        "call": {
            "target": {
                "kind": "builtin",
                "op": op
            },
            "args": args
        }
    })
}

fn push_expr(expressions: &mut Vec<Value>, expression: Value) -> usize {
    let index = expressions.len();
    expressions.push(expression);
    index
}

fn expr_ref_json(index: usize) -> Value {
    json!({ "expression": index })
}

fn db_query_value_expr(query: Value) -> Value {
    json!({
        "kind": "dbQuery",
        "target": thread_db_target_json(),
        "query": query,
        "resultType": { "kind": "builtin", "name": "Json" }
    })
}

fn db_query(predicates: Vec<Value>) -> Value {
    if predicates.is_empty() {
        json!({})
    } else {
        json!({ "where": predicates })
    }
}

fn db_predicate_compare(field: &str, op: &str, value: Value) -> Value {
    json!({
        "kind": "compare",
        "field": db_field_path_json(field),
        "op": op,
        "value": value
    })
}

fn db_predicate_conditional(condition: Value, predicate: Value) -> Value {
    json!({ "kind": "conditional", "condition": condition, "predicate": predicate })
}

fn db_field_path_json(field: &str) -> Value {
    let segments = field.split('.').map(str::to_string).collect::<Vec<_>>();
    json!({ "text": field, "segments": segments })
}

fn thread_db_target_json() -> Value {
    json!({
        "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
        "typeName": "Thread"
    })
}

fn literal_string_expr(value: &str) -> Value {
    json!({ "kind": "literal", "value": { "kind": "string", "value": value } })
}

fn literal_number_expr(value: i64) -> Value {
    json!({ "kind": "literal", "value": { "kind": "number", "value": value } })
}

fn literal_bool_expr(value: bool) -> Value {
    json!({ "kind": "literal", "value": { "kind": "bool", "value": value } })
}

fn parameter_slot_def_executable() -> LinkedExecutable {
    let mut executable = run_executable();
    executable.body = executable_body(json!({
        "blocks": [
            {
                "label": "entry",
                "statements": [
                    { "statement": 0 }
                ]
            }
        ],
        "statements": [
            {
                "kind": "return",
                "value": { "expression": 0 }
            }
        ],
        "expressions": [
            { "kind": "loadSlot", "slot": 0 }
        ]
    }));
    executable
}

fn self_local_call_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "self".to_string(),
                kind: "selfValue".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
                        },
                        "args": []
                    }
                }
            ]
        })),
    }
}

fn receiver_builtin_array_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "items".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 },
                        { "statement": 3 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 0,
                    "value": { "expression": 1 }
                },
                {
                    "kind": "expr",
                    "value": { "expression": 4 }
                },
                {
                    "kind": "assign",
                    "target": {
                        "kind": "index",
                        "object": { "expression": 0 },
                        "index": { "expression": 5 }
                    },
                    "value": { "expression": 6 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "arrayLiteral",
                    "items": [
                        { "expression": 2 }
                    ]
                },
                { "kind": "literal", "value": { "kind": "string", "value": "a" } },
                { "kind": "literal", "value": { "kind": "string", "value": "b" } },
                {
                    "kind": "call",
                    "call": {
                        "target": receiver_builtin_target("Array", "push"),
                        "args": [
                            { "expression": 0 },
                            { "expression": 3 }
                        ]
                    }
                },
                { "kind": "literal", "value": { "kind": "number", "value": 0 } },
                { "kind": "literal", "value": { "kind": "string", "value": "z" } }
            ]
        })),
    }
}

fn bytes_concat_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 6 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "hel" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.bytes",
                                "symbol": "fromUtf8",
                                "bindingKey": "core.bytes.fromUtf8"
                            }
                        },
                        "args": [{ "expression": 0 }]
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "lo" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "bytes",
                                "symbol": "fromUtf8",
                                "bindingKey": "core.bytes.fromUtf8"
                            }
                        },
                        "args": [{ "expression": 2 }]
                    }
                },
                {
                    "kind": "arrayLiteral",
                    "items": [
                        { "expression": 1 },
                        { "expression": 3 }
                    ]
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.bytes",
                                "symbol": "concat",
                                "bindingKey": "core.bytes.concat"
                            }
                        },
                        "args": [{ "expression": 4 }]
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": receiver_builtin_target("bytes", "toUtf8String"),
                        "args": [{ "expression": 5 }]
                    }
                }
            ]
        })),
    }
}

fn bytes_from_utf8_invalid_arg_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "number", "value": 42 } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.bytes",
                                "symbol": "fromUtf8",
                                "bindingKey": "core.bytes.fromUtf8"
                            }
                        },
                        "args": [{ "expression": 0 }]
                    }
                }
            ]
        })),
    }
}

fn time_sleep_executable(ms: i64) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }, { "statement": 1 }]
                }
            ],
            "statements": [
                {
                    "kind": "expr",
                    "value": { "expression": 1 }
                },
                {
                    "kind": "return"
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "number", "value": ms } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.time",
                                "symbol": "sleep",
                                "bindingKey": "std.time.sleep"
                            }
                        },
                        "args": [{ "expression": 0 }]
                    }
                }
            ]
        })),
    }
}

fn catch_time_sleep_without_catch_type_executable(ms: i64) -> LinkedExecutable {
    catch_time_sleep_with_optional_catch_type_executable(ms, None)
}

fn catch_time_sleep_with_catch_type_executable(ms: i64, catch_type_name: &str) -> LinkedExecutable {
    catch_time_sleep_with_optional_catch_type_executable(ms, Some(catch_type_name))
}

fn catch_time_sleep_with_optional_catch_type_executable(
    ms: i64,
    catch_type_name: Option<&str>,
) -> LinkedExecutable {
    let catch_expression = match catch_type_name {
        Some(catch_type_name) => json!({
            "kind": "catch",
            "tryExpression": { "expression": 1 },
            "catchSlot": 0,
            "catchType": {
                "kind": "builtin",
                "name": catch_type_name
            },
            "body": { "expression": 2 }
        }),
        None => json!({
            "kind": "catch",
            "tryExpression": { "expression": 1 },
            "catchSlot": 0,
            "body": { "expression": 2 }
        }),
    };

    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "$catch0".to_string(),
                kind: "temp".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "number", "value": ms } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.time",
                                "symbol": "sleep",
                                "bindingKey": "std.time.sleep"
                            }
                        },
                        "args": [{ "expression": 0 }]
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                catch_expression
            ]
        })),
    }
}

fn read_self_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "readSelf".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "self".to_string(),
                kind: "selfValue".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn package_call_executable() -> LinkedExecutable {
    package_call_executable_with_package_ref(json!({
        "kind": "packageId",
        "packageId": "example.com/pkg"
    }))
}

fn package_call_executable_with_package_ref(package_ref: Value) -> LinkedExecutable {
    package_call_executable_with_symbol(json!({
        "package": package_ref,
        "symbolPath": "pkg.echo"
    }))
}

fn package_call_executable_with_symbol(_symbol: Value) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![crate::eval::program::ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "Json".to_string(),
                args: Vec::new(),
            },
        }],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "input".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::package(0, 0, 0)).unwrap()
                        },
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn telemetry_emit_native_direct_call_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: LinkedExecutableBody {
            blocks: vec![crate::eval::program::BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                LinkedStmtIr::Expr {
                    value: ExprRefIr { expression: 4 },
                },
                LinkedStmtIr::Return { value: None },
            ],
            expressions: vec![
                expression(literal_string_expr("info")),
                expression(literal_string_expr("native telemetry")),
                expression(literal_string_expr("runtime-test")),
                LinkedExprIr::MapLiteral {
                    entries: BTreeMap::from([("source".to_string(), ExprRefIr { expression: 2 })]),
                },
                LinkedExprIr::Call {
                    call: CallIr {
                        target: LinkedCallTarget::Native {
                            target: NativeTarget {
                                namespace: "std.telemetry".to_string(),
                                symbol: "emit".to_string(),
                                binding_key: Some("std.telemetry.emit".to_string()),
                                metadata: BTreeMap::new(),
                            },
                        },
                        args: vec![
                            ExprRefIr { expression: 0 },
                            ExprRefIr { expression: 1 },
                            ExprRefIr { expression: 3 },
                        ],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                },
            ],
        },
    }
}

fn resource_text_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "text",
        "std.resource.text",
        path,
        None,
        builtin_type("string"),
    )
}

fn resource_exists_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "exists",
        "std.resource.exists",
        path,
        None,
        builtin_type("bool"),
    )
}

fn resource_json_object_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "json",
        "std.resource.json",
        path,
        Some(json!({
            "T0": { "kind": "builtin", "name": "JsonObject" }
        })),
        builtin_type("JsonObject"),
    )
}

fn resource_native_executable(
    symbol: &str,
    binding_key: &str,
    path: &str,
    type_args: Option<Value>,
    return_type: LinkedTypeRef,
) -> LinkedExecutable {
    let mut call = json!({
        "target": {
            "kind": "native",
            "target": {
                "namespace": "std.resource",
                "symbol": symbol,
                "bindingKey": binding_key
            }
        },
        "args": [
            { "expression": 0 }
        ]
    });
    if let Some(type_args) = type_args {
        call["typeArgs"] = type_args;
    }

    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(return_type),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": path } },
                {
                    "kind": "call",
                    "call": call
                }
            ]
        })),
    }
}

fn service_calls_package_resource_text_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin_type("string")),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::package(0, 0, 0)).unwrap()
                        },
                        "args": []
                    }
                }
            ]
        })),
    }
}

fn resource_table(path: &str, bytes: &[u8]) -> PublicationResourceTable {
    let mut table = PublicationResourceTable::default();
    table.insert(path.to_string(), loaded_resource(path, bytes));
    table
}

fn loaded_resource(path: &str, bytes: &[u8]) -> LoadedPublicationResource {
    LoadedPublicationResource {
        meta: PublicationResourceRef {
            path: path.to_string(),
            sha256: format!("test-sha256:{}", bytes.len()),
            byte_len: bytes.len() as u64,
            content_type: Some("text/plain".to_string()),
            artifact_path: Some(format!("resources/{path}")),
        },
        bytes: Arc::from(bytes.to_vec().into_boxed_slice()),
    }
}

fn builtin_type(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn assert_resource_error(error: &RuntimeError, path: &str) {
    let payload = error.payload();
    assert_eq!(payload.code, "std.resource.ResourceError");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["path"].as_str()),
        Some(path),
        "unexpected payload: {payload:?}"
    );
}

fn assert_resource_json_decode_error(error: &RuntimeError, path: &str) {
    let payload = error.payload();
    assert_eq!(payload.code, "std.json.DecodeError");
    assert_eq!(
        payload
            .details
            .as_ref()
            .and_then(|details| details["target"].as_str()),
        Some("std.resource.json"),
        "unexpected payload: {payload:?}"
    );
    assert!(
        payload.message.contains(path),
        "decode error message should include resource path {path}: {payload:?}"
    );
}

fn for_in_value_block_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "item".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "acc".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                },
                {
                    "label": "append",
                    "statements": [
                        { "statement": 3 }
                    ]
                },
                {
                    "label": "value",
                    "statements": [
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 5 }
                },
                {
                    "kind": "let",
                    "slot": 1,
                    "value": { "expression": 0 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 1 },
                    "body": "append"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 1 },
                    "value": { "expression": 4 }
                },
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                {
                    "kind": "arrayLiteral",
                    "items": [
                        { "expression": 2 },
                        { "expression": 3 }
                    ]
                },
                { "kind": "literal", "value": { "kind": "string", "value": "a" } },
                { "kind": "literal", "value": { "kind": "string", "value": "bc" } },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 6 },
                    "right": { "expression": 7 }
                },
                {
                    "kind": "valueBlock",
                    "block": "value",
                    "result": { "expression": 6 }
                },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "loadSlot", "slot": 0 },
            ],
        })),
    }
}

fn local_stream_aggregate_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: stream_route_slots(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "append",
                    "statements": [
                        { "statement": 3 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 1,
                    "value": { "expression": 1 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 0 },
                    "body": "append"
                },
                {
                    "kind": "return",
                    "value": { "expression": 5 }
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 1 },
                    "value": { "expression": 4 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
                        },
                        "args": []
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 2 },
                    "right": { "expression": 3 }
                },
                { "kind": "loadSlot", "slot": 1 }
            ]
        })),
    }
}

fn local_stream_first_item_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: stream_route_slots(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                },
                {
                    "label": "first",
                    "statements": [
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 0 },
                    "body": "first"
                },
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
                        },
                        "args": []
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "empty" } },
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn local_const_receiver_stream_first_item_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: stream_route_slots(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                },
                {
                    "label": "first",
                    "statements": [
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 0 },
                    "body": "first"
                },
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": local_const_receiver_target(1),
                        "args": []
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "empty" } },
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn local_string_stream_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
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
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "a" } },
                { "kind": "literal", "value": { "kind": "string", "value": "b" } },
                { "kind": "literal", "value": { "kind": "string", "value": "c" } }
            ]
        })),
    }
}

/// A `Stream<string>` producer that forwards another producer's items through a
/// value binding, exercising the *deferred* stream-producer path (the one the
/// real LLM chain uses): `let s = produce_{next_index}(); for item in s { emit item }`.
/// Binding the producer to a slot parks it as a deferred producer; the `for-in`
/// then drives it through `drive_deferred_stream_producer` ->
/// `exec_prepared_native_stream_producer_arg` -> `run_stream_producer`, which is
/// the recursion boundary the producer-side boxing protects. Chaining several of
/// these builds a deep nested producer-consuming-producer stack.
fn forwarding_string_stream_producer_executable(next_index: usize) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.forward".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "source".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "item".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "forward",
                    "statements": [
                        { "statement": 3 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 0,
                    "value": { "expression": 0 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 1 },
                    "body": "forward"
                },
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, next_index)).unwrap()
                        },
                        "args": []
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        })),
    }
}

fn local_http_sse_response_stream_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_stream_type(std_http_type_ref(
            STD_HTTP_SSE_EVENT_TYPE_INDEX,
        ))),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 6 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "response" } },
                { "kind": "literal", "value": { "kind": "number", "value": 200 } },
                { "kind": "literal", "value": { "kind": "string", "value": "content-type" } },
                { "kind": "literal", "value": { "kind": "string", "value": "text/event-stream" } },
                {
                    "kind": "mapLiteral",
                    "entries": {
                        "name": { "expression": 2 },
                        "value": { "expression": 3 }
                    }
                },
                {
                    "kind": "arrayLiteral",
                    "items": [
                        { "expression": 4 }
                    ]
                },
                {
                    "kind": "mapLiteral",
                    "entries": {
                        "tag": { "expression": 0 },
                        "status": { "expression": 1 },
                        "headers": { "expression": 5 }
                    }
                }
            ]
        })),
    }
}

fn outer_string_stream_from_sse_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "tag".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "forward",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 2 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 1 },
                    "body": "forward"
                },
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 3)).unwrap()
                        },
                        "args": []
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 2)).unwrap()
                        },
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        })),
    }
}

fn sse_tag_string_stream_converter_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.convert".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "sse".to_string(),
            slot: 0,
            ty: linked_stream_type(std_http_type_ref(STD_HTTP_SSE_EVENT_TYPE_INDEX)),
        }],
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "sse".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "event".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "emit_tag",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 2 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 0 },
                    "body": "emit_tag"
                },
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "field",
                    "object": { "expression": 1 },
                    "field": "tag"
                },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        })),
    }
}

fn local_const_receiver_stream_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.ManagedLlm.sendChat".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: Some(linked_builtin_type("Json")),
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "self".to_string(),
                kind: "selfValue".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "field",
                    "object": { "expression": 0 },
                    "field": "name"
                }
            ]
        })),
    }
}

fn stream_variable_json_object_length_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "events".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "Stream".to_string(),
                args: vec![linked_builtin_type("JsonObject")],
            },
        }],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "events".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "stream".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 2,
                    name: "event".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 3,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "first",
                    "statements": [
                        { "statement": 3 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 1,
                    "value": { "expression": 0 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 2,
                    "itemType": { "kind": "builtin", "name": "JsonObject" },
                    "iterable": { "expression": 1 },
                    "body": "first"
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 4 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "literal", "value": { "kind": "number", "value": 0 } },
                { "kind": "loadSlot", "slot": 2 },
                {
                    "kind": "call",
                    "call": {
                        "target": receiver_builtin_target("JsonObject", "length"),
                        "args": [
                            { "expression": 3 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn create_from_stream_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
                        },
                        "args": []
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.file",
                                "symbol": "createFromStream",
                                "bindingKey": "std.file.createFromStream"
                            }
                        },
                        "args": [
                            { "expression": 0 },
                            { "expression": 2 }
                        ]
                    }
                },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        })),
    }
}

fn bytes_stream_emit_then_bad_emit_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![LinkedTypeRef::Native {
                name: "bytes".to_string(),
                args: Vec::new(),
            }],
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 1 }
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "ok" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.bytes",
                                "symbol": "fromUtf8",
                                "bindingKey": "core.bytes.fromUtf8"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "not bytes" } }
            ]
        })),
    }
}

fn emit_response_stream_helper_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": []
                }
            ],
            "statements": [],
            "expressions": []
        })),
    }
}

fn emit_response_stream_call_ir() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Native {
            target: NativeTarget {
                namespace: "std.http".to_string(),
                symbol: "emitResponseStream".to_string(),
                binding_key: Some("std.http.stream.emitResponse".to_string()),
                metadata: BTreeMap::new(),
            },
        },
        args: vec![ExprRefIr { expression: 0 }],
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn create_from_stream_call_ir() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Native {
            target: NativeTarget {
                namespace: "std.file".to_string(),
                symbol: "createFromStream".to_string(),
                binding_key: Some("std.file.createFromStream".to_string()),
                metadata: BTreeMap::new(),
            },
        },
        args: vec![ExprRefIr { expression: 0 }, ExprRefIr { expression: 1 }],
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
    }
}

fn http_stream_chunk_value(heap: &mut RequestHeap, bytes: &[u8]) -> RuntimeValue {
    let bytes = heap
        .alloc_bytes(bytes)
        .expect("chunk bytes should allocate");
    let event = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
            ("tag".to_string(), RuntimeValue::String("chunk".to_string())),
            ("value".to_string(), RuntimeValue::Heap(bytes)),
        ])))
        .expect("chunk event should allocate");
    RuntimeValue::Heap(event)
}

fn local_native_stream_wrapper_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_stream_type(std_http_type_ref(
            STD_HTTP_SSE_EVENT_TYPE_INDEX,
        ))),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 5 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "GET" } },
                { "kind": "literal", "value": { "kind": "string", "value": "https://example.test/events" } },
                { "kind": "arrayLiteral", "items": [] },
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "builtin", "name": "HttpClientRequest" },
                    "fields": {
                        "method": { "expression": 0 },
                        "url": { "expression": 1 },
                        "headers": { "expression": 2 },
                        "body": { "expression": 3 },
                        "timeoutMs": { "expression": 3 }
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.http",
                                "symbol": "sse",
                                "bindingKey": "std.http.client.sse"
                            }
                        },
                        "args": [
                            { "expression": 4 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn local_native_sse_forwarding_stream_producer_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "svc.main.produce".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_stream_type(std_http_type_ref(
            STD_HTTP_SSE_EVENT_TYPE_INDEX,
        ))),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "event".to_string(),
                kind: "local".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 2 }
                    ]
                },
                {
                    "label": "forward",
                    "statements": [
                        { "statement": 1 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "forIn",
                    "itemSlot": 0,
                    "iterable": { "expression": 5 },
                    "body": "forward"
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 6 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "GET" } },
                { "kind": "literal", "value": { "kind": "string", "value": "https://example.test/events" } },
                { "kind": "arrayLiteral", "items": [] },
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "builtin", "name": "HttpClientRequest" },
                    "fields": {
                        "method": { "expression": 0 },
                        "url": { "expression": 1 },
                        "headers": { "expression": 2 },
                        "body": { "expression": 3 },
                        "timeoutMs": { "expression": 3 }
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.http",
                                "symbol": "sse",
                                "bindingKey": "std.http.client.sse"
                            }
                        },
                        "args": [
                            { "expression": 4 }
                        ]
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]
        })),
    }
}

fn http_stream_effect_in_http_handler_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "request".to_string(),
            slot: 0,
            ty: std_http_type_ref(STD_HTTP_REQUEST_TYPE_INDEX),
        }],
        return_type: Some(LinkedTypeRef::Native {
            name: "integer".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "request".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "response".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 8 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "POST" } },
                { "kind": "literal", "value": { "kind": "string", "value": "https://example.test/chat/completions" } },
                { "kind": "arrayLiteral", "items": [] },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "field", "object": { "expression": 3 }, "field": "body" },
                { "kind": "literal", "value": { "kind": "null" } },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "builtin", "name": "HttpClientRequest" },
                    "fields": {
                        "method": { "expression": 0 },
                        "url": { "expression": 1 },
                        "headers": { "expression": 2 },
                        "body": { "expression": 4 },
                        "timeoutMs": { "expression": 5 }
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.http",
                                "symbol": "stream",
                                "bindingKey": "std.http.client.stream"
                            }
                        },
                        "args": [
                            { "expression": 6 }
                        ]
                    }
                },
                { "kind": "field", "object": { "expression": 7 }, "field": "status" }
            ]
        })),
    }
}

fn http_stream_start_helper_in_http_handler_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "request".to_string(),
            slot: 0,
            ty: std_http_type_ref(STD_HTTP_REQUEST_TYPE_INDEX),
        }],
        return_type: Some(std_http_type_ref(STD_HTTP_RESPONSE_STREAM_EVENT_TYPE_INDEX)),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "request".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "number", "value": 200 } },
                { "kind": "arrayLiteral", "items": [] },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.http",
                                "symbol": "streamStart",
                                "bindingKey": "std.http.stream.start"
                            }
                        },
                        "args": [
                            { "expression": 0 },
                            { "expression": 1 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn stream_route_slots() -> SlotLayoutIr {
    SlotLayoutIr {
        slots: vec![
            SlotIr {
                index: 0,
                name: "item".to_string(),
                kind: "local".to_string(),
            },
            SlotIr {
                index: 1,
                name: "acc".to_string(),
                kind: "local".to_string(),
            },
        ],
        frame_size: 2,
    }
}

fn match_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                },
                {
                    "label": "matched",
                    "statements": [
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "match",
                    "value": { "expression": 0 },
                    "arms": [
                        {
                            "pattern": {
                                "kind": "literal",
                                "value": { "kind": "string", "value": "ready" }
                            },
                            "body": "matched"
                        }
                    ]
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "ready" } },
                { "kind": "literal", "value": { "kind": "string", "value": "matched" } },
                { "kind": "literal", "value": { "kind": "string", "value": "missed" } }
            ]
        })),
    }
}

fn type_pattern_match_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                },
                {
                    "label": "matched",
                    "statements": [
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "match",
                    "value": { "expression": 1 },
                    "arms": [
                        {
                            "pattern": {
                                "kind": "type",
                                "ty": { "kind": "builtin", "name": "AuthError" }
                            },
                            "body": "matched"
                        }
                    ]
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "denied" } },
                {
                    "kind": "mapLiteral",
                    "entries": {
                        "message": { "expression": 0 }
                    }
                },
                { "kind": "literal", "value": { "kind": "string", "value": "missed" } },
                { "kind": "literal", "value": { "kind": "string", "value": "matched" } }
            ]
        })),
    }
}

fn catch_throw_executable() -> LinkedExecutable {
    catch_throw_with_type_addrs_executable(service_type_addr(0), service_type_addr(0))
}

fn catch_throw_without_catch_type_executable() -> LinkedExecutable {
    catch_throw_with_optional_type_addr_executable(service_type_addr(0), None)
}

fn catch_builtin_decode_error_throw_executable() -> LinkedExecutable {
    catch_builtin_decode_error_throw_with_catch_type_executable("std.json.DecodeError")
}

fn catch_builtin_decode_error_throw_with_catch_type_executable(
    catch_type_name: &str,
) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "$catch0".to_string(),
                kind: "temp".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 5 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "test.decode" } },
                { "kind": "literal", "value": { "kind": "string", "value": "denied" } },
                {
                    "kind": "mapLiteral",
                    "entries": {
                        "target": { "expression": 0 },
                        "message": { "expression": 1 }
                    }
                },
                {
                    "kind": "throw",
                    "value": { "expression": 2 },
                    "payloadType": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "catch",
                    "tryExpression": { "expression": 3 },
                    "catchSlot": 0,
                    "catchType": {
                        "kind": "builtin",
                        "name": catch_type_name
                    },
                    "body": { "expression": 4 }
                }
            ]
        })),
    }
}

fn catch_native_decode_error_executable() -> LinkedExecutable {
    catch_native_decode_error_with_catch_type_executable(Some("std.json.DecodeError"))
}

fn catch_native_decode_error_without_catch_type_executable() -> LinkedExecutable {
    catch_native_decode_error_with_catch_type_executable(None)
}

fn catch_literal_with_catch_type_executable(catch_type_name: &str) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "$catch0".to_string(),
                kind: "temp".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "number", "value": 7 } },
                {
                    "kind": "catch",
                    "tryExpression": { "expression": 0 },
                    "catchSlot": 0,
                    "catchType": {
                        "kind": "builtin",
                        "name": catch_type_name
                    },
                    "body": { "expression": 0 }
                }
            ]
        })),
    }
}

fn catch_native_decode_error_with_catch_type_executable(
    catch_type_name: Option<&str>,
) -> LinkedExecutable {
    let catch_expression = match catch_type_name {
        Some(catch_type_name) => json!({
            "kind": "catch",
            "tryExpression": { "expression": 1 },
            "catchSlot": 0,
            "catchType": {
                "kind": "builtin",
                "name": catch_type_name
            },
            "body": { "expression": 2 }
        }),
        None => json!({
            "kind": "catch",
            "tryExpression": { "expression": 1 },
            "catchSlot": 0,
            "body": { "expression": 2 }
        }),
    };

    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "$catch0".to_string(),
                kind: "temp".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "JsonObject" }
                        }
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                catch_expression
            ]
        })),
    }
}

fn catch_throw_with_type_addrs_executable(
    throw_type_addr: TypeAddr,
    catch_type_addr: TypeAddr,
) -> LinkedExecutable {
    catch_throw_with_optional_type_addr_executable(throw_type_addr, Some(catch_type_addr))
}

fn catch_throw_with_optional_type_addr_executable(
    throw_type_addr: TypeAddr,
    catch_type_addr: Option<TypeAddr>,
) -> LinkedExecutable {
    let catch_expression = match catch_type_addr {
        Some(catch_type_addr) => json!({
            "kind": "catch",
            "tryExpression": { "expression": 2 },
            "catchSlot": 0,
            "catchType": {
                "kind": "address",
                "addr": serde_json::to_value(catch_type_addr).unwrap()
            },
            "body": { "expression": 4 }
        }),
        None => json!({
            "kind": "catch",
            "tryExpression": { "expression": 2 },
            "catchSlot": 0,
            "body": { "expression": 4 }
        }),
    };

    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "$catch0".to_string(),
                kind: "temp".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "denied" } },
                {
                    "kind": "mapLiteral",
                    "entries": {
                        "message": { "expression": 0 }
                    }
                },
                {
                    "kind": "throw",
                    "value": { "expression": 1 },
                    "payloadType": {
                        "kind": "address",
                        "addr": serde_json::to_value(throw_type_addr).unwrap()
                    }
                },
                catch_expression,
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn assert_executable(condition: bool) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "assert",
                    "condition": { "expression": 0 },
                    "message": { "expression": 1 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                },
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "bool", "value": condition } },
                { "kind": "literal", "value": { "kind": "string", "value": "assert failed in program" } },
                { "kind": "literal", "value": { "kind": "string", "value": "ok" } },
            ],
        })),
    }
}

fn package_echo_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "echo".to_string(),
        type_params: Vec::new(),
        params: vec![crate::eval::program::ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "Json".to_string(),
                args: Vec::new(),
            },
        }],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "input".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "string", "value": " from package" } },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 0 },
                    "right": { "expression": 1 }
                },
            ],
        })),
    }
}

fn package_file_unit(
    identity: &str,
    module_path: &str,
    executable: LinkedExecutable,
) -> LinkedFileUnit {
    LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: identity.to_string(),
        source_ast_hash: format!("source:{identity}"),
        module_path: module_path.to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![executable],
        external_refs: Default::default(),
    }
}

fn package_call_config_reader_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "track.record".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": {
                                "unit": { "kind": "package", "value": 1 },
                                "file": { "kind": "loadedFileIndex", "value": 0 },
                                "executable": 0
                            }
                        },
                        "args": []
                    }
                }
            ]
        })),
    }
}

fn config_require_string_executable(path: &str) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "httpSession.read".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": path } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "builtin",
                            "op": "config.require"
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T": { "kind": "builtin", "name": "string" }
                        }
                    }
                }
            ]
        })),
    }
}

fn package_generic_json_decode_call_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{\"name\":\"Ada\"}" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::package(0, 0, 0)).unwrap()
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "JsonObject" }
                        }
                    }
                }
            ]
        })),
    }
}

fn package_generic_config_require_call_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::package(0, 0, 0)).unwrap()
                        },
                        "args": [],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "string" }
                        }
                    }
                }
            ]
        })),
    }
}

fn generic_json_decode_native_wrapper_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "decode".to_string(),
        type_params: Vec::new(),
        params: vec![crate::eval::program::ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        }],
        return_type: Some(LinkedTypeRef::TypeParam {
            name: "T".to_string(),
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "input".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "typeParam", "name": "T" }
                        }
                    }
                }
            ]
        })),
    }
}

fn generic_config_require_wrapper_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "readConfig".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::TypeParam {
            name: "T".to_string(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "sessionSecret" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "builtin",
                            "op": "config.require"
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "typeParam", "name": "T" }
                        }
                    }
                }
            ]
        })),
    }
}

fn json_decode_native_missing_type_args_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "JsonObject".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{\"name\":\"Ada\"}" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn json_decode_native_missing_binding_key_executable() -> LinkedExecutable {
    let mut executable = json_decode_native_missing_type_args_executable();
    let LinkedExprIr::Call { call } = executable
        .body
        .expressions
        .get_mut(1)
        .expect("test executable should have a native decode call")
    else {
        panic!("test executable expression 1 should be a native decode call");
    };
    call.type_args.insert(
        "T0".to_string(),
        LinkedTypeRef::Native {
            name: "JsonObject".to_string(),
            args: Vec::new(),
        },
    );
    let LinkedCallTarget::Native { target } = &mut call.target else {
        panic!("test executable call should target a native function");
    };
    target.binding_key = None;
    executable
}

fn json_encode_native_missing_type_args_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![crate::eval::program::ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "string".to_string(),
                args: Vec::new(),
            },
        }],
        return_type: Some(LinkedTypeRef::Native {
            name: "string".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "input".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "encode",
                                "bindingKey": "std.json.encode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn json_decode_native_missing_t0_type_arg_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "JsonObject".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{\"name\":\"Ada\"}" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T1": { "kind": "builtin", "name": "JsonObject" }
                        }
                    }
                }
            ]
        })),
    }
}

fn json_decode_native_unresolved_type_arg_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "JsonObject".to_string(),
            args: Vec::new(),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{\"name\":\"Ada\"}" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "typeParam", "name": "T" }
                        }
                    }
                }
            ]
        })),
    }
}

fn json_decode_native_target_metadata_executable() -> LinkedExecutable {
    let mut executable = json_decode_native_missing_type_args_executable();
    let LinkedExprIr::Call { call } = executable
        .body
        .expressions
        .get_mut(1)
        .expect("test executable should have a native decode call")
    else {
        panic!("test executable expression 1 should be a native decode call");
    };
    call.type_args.insert(
        "T0".to_string(),
        LinkedTypeRef::Native {
            name: "JsonObject".to_string(),
            args: Vec::new(),
        },
    );
    let LinkedCallTarget::Native { target } = &mut call.target else {
        panic!("test executable call should target a native function");
    };
    target.metadata.insert(
        "mode".to_string(),
        MetadataValue::String("ignored".to_string()),
    );
    executable
}

fn json_native_direct_type_args_with_nullable_json_object_return_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Nullable {
            inner: Box::new(LinkedTypeRef::Native {
                name: "JsonObject".to_string(),
                args: Vec::new(),
            }),
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 3 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "{\"name\":\"Ada\"}" } },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "Json" }
                        }
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "encode",
                                "bindingKey": "std.json.encode"
                            }
                        },
                        "args": [
                            { "expression": 1 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "Json" }
                        }
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.json",
                                "symbol": "decode",
                                "bindingKey": "std.json.decode"
                            }
                        },
                        "args": [
                            { "expression": 2 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "JsonObject" }
                        }
                    }
                }
            ]
        })),
    }
}

fn run_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![crate::eval::program::ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: LinkedTypeRef::Native {
                name: "Json".to_string(),
                args: Vec::new(),
            },
        }],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "input".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "copy".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 1,
                    "value": { "expression": 0 }
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 1 },
                    "value": { "expression": 2 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 4 }
                },
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "string", "value": "!" } },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 0 },
                    "right": { "expression": 1 }
                },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "localType", "typeIndex": 0 },
                    "fields": {
                        "label": { "expression": 3 },
                        "copy": { "expression": 3 }
                    }
                },
            ],
        })),
    }
}

fn explicit_self_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![
            crate::eval::program::ParamIr {
                name: "self".to_string(),
                slot: 0,
                ty: LinkedTypeRef::Native {
                    name: "Json".to_string(),
                    args: Vec::new(),
                },
            },
            crate::eval::program::ParamIr {
                name: "input".to_string(),
                slot: 1,
                ty: LinkedTypeRef::Native {
                    name: "Json".to_string(),
                    args: Vec::new(),
                },
            },
        ],
        return_type: None,
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "self".to_string(),
                    kind: "selfValue".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "input".to_string(),
                    kind: "param".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [{ "statement": 0 }]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 0 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 1 }
            ],
        })),
    }
}

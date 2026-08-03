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
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{
        HeapHandle, HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue, RuntimeValueCarrier,
    },
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, PlatformBuiltinErrorIdentity, RequestException,
    },
};
use skiff_runtime_request::cancellation::CancellationToken;
use tokio::time::sleep;

use super::support::*;
use super::*;
use crate::eval::InterpreterEnv as Env;
use skiff_artifact_model::{
    builtin_receiver_op_by_name, DbMetadataIr, FileIrRef, PackageArtifactRef, PackageBuildId,
    PackageLocalAbiIdentity, PackageLocalAbiSymbol, PublicationResourceRef, TypeDescriptorIr,
    TypeExport,
};
use skiff_runtime_capability_context::{
    DbCapabilityTarget, DbCapabilityTargetId, DbProviderTargetMetadata,
};
use skiff_runtime_linked_program::{
    linked::{DbDeclarationIr, DbObjectKeyIr, DbObjectKindIr, TypeDeclarationIr},
    DbObjectTargetId, LinkedNamedUnionBranch, LoadedPublicationResource, PublicationResourceTable,
    RuntimeExecutionPackage,
};

mod tail_call_execution;

use crate::{
    eval::error::{unwrap_diagnostic_source_context, RuntimeError},
    eval::exceptions::request_exception_for_rethrow,
    eval::program::{
        anonymous_type_decl, types::PackageSymbolKey, CallIr, ConstAddr, ConstIr, ExecutableAddr,
        ExecutableKind, ExprRefIr, FileAddr, FileDeclarations, FileLinkTargets, GatewayConfig,
        LinkOverlay, LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedExprIr,
        LinkedFileUnit, LinkedStmtIr, LinkedTypeDescriptor, LinkedTypeRef, LiteralIr,
        MetadataValue, NativeTarget, ParamIr, ResolvedSymbol, RuntimeProgram, RuntimeTypeContext,
        ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, StmtRefIr, TypeAddr, TypeDeclIr,
        UnitAddr,
    },
    eval::{
        capabilities::{StreamPoll, StreamRuntime, TypedStreamSink},
        native_capability::project_runtime_native_capability_context,
        native_invocation::resolve_runtime_native_invocation,
        program_execution::{
            executable_type_param_names, OwnedProgramExecutionContext, ProgramExecutionInput,
        },
        program_invocation::{ProgramInvocationContext, ProgramInvocationInput},
        TestEffectDouble,
    },
    type_descriptor::{PlanContext, RuntimeTypePlanLinkedExt},
};
use skiff_runtime_native::dispatch::NativeDispatch;

#[tokio::test]
async fn runtime_program_executes_route_by_executable_addr() {
    let mut program = program_with_executable(run_executable());
    install_run_result_type(&mut program);
    let program = Arc::new(program);
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
            "site": test_instruction_site(),
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
            "site": test_instruction_site(),
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
            "site": test_instruction_site(),
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
    let program = Arc::new(program_with_executable_and_std_builtins(
        time_sleep_executable(20),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let started_at = std::time::Instant::now();
    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.time.sleep native should execute in RuntimeProgram");

    assert_eq!(value, json!(null));
    assert!(started_at.elapsed() >= Duration::from_millis(10));
}

#[test]
fn std_builtin_package_types_resolve_to_exact_nominal_plans() {
    let program = program_with_executable_and_std_builtins(run_executable());
    let addr = ExecutableAddr::service(0, 0);

    for (type_index, expected_name) in [
        (STD_DURATION_TYPE_INDEX, "std.time.Duration"),
        (STD_FILE_IMMUTABLE_TYPE_INDEX, "std.file.ImmutableFile"),
        (STD_FILE_CREATE_OPTIONS_TYPE_INDEX, "std.file.CreateOptions"),
        (STD_FILE_INFO_TYPE_INDEX, "std.file.FileInfo"),
        (STD_HTTP_REQUEST_TYPE_INDEX, "std.http.HttpRequest"),
        (STD_RESOURCE_INFO_TYPE_INDEX, "std.resource.ResourceInfo"),
    ] {
        let plan = std_http_type_plan_for_test(&program, &addr, type_index);
        assert_eq!(plan.named_type_name.as_deref(), Some(expected_name));
    }
}

#[tokio::test]
async fn runtime_program_time_sleep_negative_returns_immediately() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        time_sleep_executable(-1),
    ));
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
    let program = Arc::new(program_with_executable_and_std_builtins(
        time_sleep_executable(100),
    ));
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

    assert!(error.is_cancellation_terminal());
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
}

#[tokio::test]
async fn runtime_program_time_sleep_observes_deadline() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        time_sleep_executable(100),
    ));
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

    assert!(
        matches!(&error, RuntimeError::ScopeTerminal(_)),
        "direct eval must retain the current scope deadline owner: {error:?}"
    );
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
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

#[tokio::test]
async fn runtime_program_executes_package_function_call() {
    let service_addr = ExecutableAddr::service(0, 0);
    let package_addr = ExecutableAddr::package(0, 0, 0);
    let mut program = program_with_service_and_package_executables(
        package_call_executable(),
        package_echo_executable(),
    );
    replace_single_package(&mut program, "example.com/pkg", Default::default());
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
async fn runtime_program_stream_variable_crosses_nested_package_producers() {
    let mut program = program_with_executables(vec![
        package_stream_chain_route_executable(),
        local_string_stream_producer_executable(),
    ]);
    let mut package_file = package_file_unit(
        "file:stream-forwarders",
        "forwarders",
        package_string_stream_forwarder_executable(0, Some(1)),
    );
    package_file
        .executables
        .push(package_string_stream_forwarder_executable(1, None));
    program.packages = vec![runtime_package(
        "example.com/stream-forwarders",
        0,
        vec![Arc::new(package_file)],
        Default::default(),
    )];

    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let value = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("Stream variables and returned Streams should retain identity across packages");

    assert_eq!(value, json!("abc"));
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
    replace_single_package(&mut program, "example.com/pkg", Default::default());
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
    replace_single_package(&mut program, "example.com/pkg", Default::default());
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
    replace_single_package(&mut program, "skiff.run/std", Default::default());
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
    replace_single_package(&mut program, "example.com/config", Default::default());

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

    let payload = error
        .ordinary_payload()
        .expect("invalid artifact remains ordinary");
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

    let payload = error
        .ordinary_payload()
        .expect("invalid artifact remains ordinary");
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

    let payload = error
        .ordinary_payload()
        .expect("invalid artifact remains ordinary");
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

    let payload = error
        .ordinary_payload()
        .expect("invalid artifact remains ordinary");
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
    let program = Arc::new(program_with_executable_and_std_builtins(
        resource_text_native_executable("missing.txt"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for missing resources");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_text_invalid_path_throws_resource_error() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        resource_text_native_executable("./bad"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for invalid paths");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_text_invalid_utf8_throws_resource_error() {
    let mut program =
        program_with_executable_and_std_builtins(resource_text_native_executable("bad.txt"));
    program.service_resources = resource_table("bad.txt", &[0xff, 0xfe]);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.text should throw ResourceError for invalid UTF-8");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_json_syntax_error_throws_request_local_json_decode_error() {
    let mut program = program_with_executable(resource_json_object_native_executable("bad.json"));
    program.service_resources = resource_table("bad.json", b"{");
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should map syntax errors to std.json.DecodeError");

    assert_resource_json_decode_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_json_type_error_throws_request_local_json_decode_error() {
    let mut program = program_with_executable(resource_json_object_native_executable("bad.json"));
    program.service_resources = resource_table("bad.json", b"[]");
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should map type errors to std.json.DecodeError");

    assert_resource_json_decode_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_json_allows_non_stream_skiff_fields() {
    let value = json!({
        "id": {
            "__skiffRepresentationType": "UserId",
            "value": "u1"
        },
        "avatar": {
            "__skiffBytesBase64": "YWJj"
        }
    });
    let mut program = program_with_executable(resource_json_object_native_executable("ok.json"));
    let resource_bytes = serde_json::to_vec(&value).expect("resource fixture JSON should encode");
    program.service_resources = resource_table("ok.json", &resource_bytes);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let output = execute_test_program_route(&interpreter, &frame)
        .await
        .expect("std.resource.json should allow non-stream Skiff-looking JSON fields");

    assert_eq!(output, value);
}

#[tokio::test]
async fn runtime_program_resource_json_rejects_stream_return_type() {
    let mut program =
        program_with_executable(resource_json_stream_native_executable("stream.json"));
    program.service_resources = resource_table("stream.json", br#"{"__skiffStreamId":"stream-0"}"#);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should not decode resource bytes as a Stream");

    assert_resource_json_decode_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_json_invalid_utf8_throws_resource_error() {
    let mut program = program_with_executable_and_std_builtins(
        resource_json_object_native_executable("bad.json"),
    );
    program.service_resources = resource_table("bad.json", &[0xff, 0xfe]);
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.json should throw ResourceError for invalid UTF-8");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_bytes_missing_path_throws_resource_error() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        resource_bytes_native_executable("missing.bin"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.bytes should throw ResourceError for missing resources");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_resource_info_missing_path_throws_resource_error() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        resource_info_native_executable("missing.bin"),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("std.resource.info should throw ResourceError for missing resources");

    assert_resource_exception(&error);
}

#[tokio::test]
async fn runtime_program_direct_native_resource_error_catch_preserves_exact_exception() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        catch_resource_text_native_executable("missing.txt"),
    ));
    let file = Arc::clone(
        program
            .service_files
            .first()
            .expect("test program service file"),
    );
    let executable = file
        .executables
        .first()
        .expect("test program executable")
        .clone();
    let interpreter = Interpreter::with_program(Arc::clone(&program), runtime_factory());
    let frame = test_invocation("svc.main.run");
    let invocation_context = program_invocation_context(&interpreter, &frame);
    let context = invocation_context.execution_context();
    let heap = RequestHeap::default();
    let mut env = Env::default();
    let mut access = skiff_runtime_eval::heap_access::HeapAccess::private(heap);

    let caught = interpreter
        .eval_program_expr_ref(
            context,
            &mut access,
            &mut env,
            &ExecutableAddr::service(0, 0),
            file.as_ref(),
            &executable,
            ExprRefIr { expression: 2 },
        )
        .await
        .expect("catch<ResourceError> should catch the direct native failure");
    let caught_handle = caught
        .as_heap_handle()
        .expect("catch result should be a request-local object");
    let tag = access
        .object_field_carrier(caught_handle, "tag")
        .expect("catch tag should be readable")
        .expect("catch result should have a tag");
    assert_eq!(tag.value(), &RuntimeValue::String("err".to_string()));
    let exception_handle = access
        .object_field_carrier(caught_handle, "exception")
        .expect("catch exception should be readable")
        .expect("err result should have an exception")
        .as_heap_handle()
        .expect("caught exception should remain a request-local handle");
    let HeapNode::Exception(exception) = access
        .get(exception_handle)
        .expect("caught exception handle should resolve")
    else {
        panic!("caught err payload should be an exception node");
    };
    assert_resource_request_exception(exception);
}

#[tokio::test]
async fn runtime_program_resource_error_requires_std_package_type() {
    let program = Arc::new(program_with_executable(resource_text_native_executable(
        "missing.txt",
    )));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("ResourceError projection must require the canonical std package type");

    assert_resource_error_invalid_artifact(&error);
}

#[tokio::test]
async fn runtime_program_resource_error_rejects_wrong_std_type_shape() {
    let mut program =
        program_with_executable_and_std_builtins(resource_text_native_executable("missing.txt"));
    replace_std_resource_error_type(
        &mut program,
        anonymous_type_decl(
            "ResourceError",
            linked_record_descriptor(vec![("message", linked_builtin_type("string"))]),
        ),
    );
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("ResourceError projection must reject a non-canonical std type shape");

    assert_resource_error_invalid_artifact(&error);
}

#[tokio::test]
async fn runtime_program_resource_error_rejects_std_implementation_only_type() {
    let mut program =
        program_with_executable_and_std_builtins(resource_text_native_executable("missing.txt"));
    let package = program.packages.first().expect("std package test fixture");
    let mut artifact = package.artifact().clone();
    artifact.package_local_abi.public_symbols.clear();
    program.packages[0] = crate::eval::test_support::runtime_execution_package_from_artifact(
        0,
        artifact,
        package.files().to_vec(),
        package.static_resources().clone(),
    );
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("implementation-only ResourceError must not become publicly catchable");

    assert_resource_error_invalid_artifact(&error);
}

#[tokio::test]
async fn runtime_program_resource_package_call_site_reads_package_resource() {
    let mut program = program_with_service_and_package_executables(
        service_calls_package_resource_text_executable(),
        resource_text_native_executable("prompts/system.md"),
    );
    program.service_resources = resource_table("prompts/system.md", b"service text");
    replace_single_package(
        &mut program,
        "example.com/pkg",
        resource_table("prompts/system.md", b"package text"),
    );
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
        runtime_package(
            "skiff.run/track",
            0,
            vec![Arc::new(package_file_unit(
                "file:track",
                "track.main",
                package_call_config_reader_executable(),
            ))],
            Default::default(),
        ),
        runtime_package(
            "skiff.run/http-session",
            1,
            vec![Arc::new(package_file_unit(
                "file:http-session",
                "httpSession.main",
                config_require_string_executable("sessionSecret"),
            ))],
            Default::default(),
        ),
    ];
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
/// producer chain while the Tokio worker that polls spawned producers has a
/// *small* (1 MiB) stack. The outer libtest thread keeps its ordinary stack: the
/// test is about producer-task isolation, not whether an entire debug runtime
/// can be constructed and driven inside 1 MiB.
///
/// The pre-fix co-driven model nested one future per producer level on the
/// worker's native stack and overflowed/aborted well before depth 40 at 1 MiB.
/// With every producer running in its own `tokio::spawn`ed task, each worker poll
/// sees one producer and the chain completes. This distinguishes the root fix
/// from the 64 MiB worker-stack mitigation.
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

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("nested-stream-producer-small-stack-worker")
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
            .expect("deep producer chain should run depth-independently on a small worker stack");

        // Every forwarding level passes items through unchanged, so the
        // leaf's "a","b","c" must still aggregate to "abc".
        assert_eq!(value, json!("abc"));
    });
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
            &mut skiff_runtime_eval::heap_access::HeapAccess::private(heap),
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

#[tokio::test]
async fn runtime_program_http_stream_chunk_and_end_construct_canonical_wire_events() {
    for (executable, expected) in [
        (
            http_stream_chunk_helper_in_http_handler_executable(),
            json!({
                "tag": "chunk",
                "value": { "__skiffBytesBase64": "aGVsbG8gd29ybGQ=" }
            }),
        ),
        (
            http_stream_end_helper_in_http_handler_executable(),
            json!({ "tag": "end" }),
        ),
    ] {
        let program = Arc::new(program_with_executable_and_std_http_types(executable));
        let interpreter = Interpreter::with_program(program, runtime_factory());
        let mut frame = test_invocation("svc.main.run");
        set_request_http_arg(&mut frame, "request");

        let value = execute_test_program_route(&interpreter, &frame)
            .await
            .expect("HTTP stream event constructor should use its exact native signature");
        assert_eq!(value, expected);
    }
}

#[test]
fn test_host_operation_double_materializes_typed_request_without_consuming_actual_input() {
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
    let mut heap = RequestHeap::default();
    let input = http_client_request_runtime_value(&mut heap);
    let input_before = input.clone();
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
        .expect("test double should match the materialized bytes input");

    assert!(matches!(value, RuntimeValue::Heap(_)));
    assert_eq!(input, input_before);
    assert!(heap.stats().materialize_output_bytes > 0);
}

#[test]
fn test_host_operation_double_fails_closed_for_invalid_request_heap_handle() {
    let program = Arc::new(program_with_executable(run_executable()));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.request".to_string(),
            TestEffectDouble {
                expect_request: Some(json!({ "method": "POST" })),
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
    let invalid_input = RuntimeValue::Heap(HeapHandle::new(42, 0));
    let arg_plan = RuntimeTypePlan::from_descriptor(
        &json!({ "kind": "builtin", "name": "std.http.HttpClientRequest", "args": [] }),
    )
    .expect("arg plan should build");
    let return_plan = RuntimeTypePlan::from_descriptor(
        &json!({ "kind": "builtin", "name": "std.http.HttpClientResponse", "args": [] }),
    )
    .expect("return plan should build");

    let error = interpreter
        .dispatch_test_http_effect_invocation_double(
            "std.http.client.request",
            Some(&invalid_input),
            Some(&arg_plan),
            Some(&return_plan),
            &mut heap,
        )
        .expect("test double should dispatch")
        .expect_err("invalid request heap handle must fail closed");

    assert!(
        error.to_string().contains("invalid heap handle 42:0"),
        "unexpected error: {error}"
    );
}

#[test]
fn test_host_operation_double_does_not_alias_distinct_binding_key() {
    let program = Arc::new(program_with_executable(run_executable()));
    let interpreter = Interpreter::with_program_test_effect_doubles(
        program,
        HashMap::from([(
            "std.http.client.request".to_string(),
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
        "runtime HTTP effect doubles must match the exact bindingKey"
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
    let program = Arc::new(program_with_executables_and_std_builtins(vec![
        create_from_stream_route_executable(),
        bytes_stream_emit_then_typed_throw_producer_executable(),
    ]));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let frame = test_invocation("svc.main.run");

    let error = execute_test_program_route(&interpreter, &frame)
        .await
        .expect_err("producer error should win over missing file store consumer error");

    assert_json_decode_exception_identity(&error);
}

#[tokio::test]
async fn runtime_program_create_from_stream_items_use_request_heap_budget() {
    let program = Arc::new(program_with_executable_and_std_builtins(
        emit_response_stream_helper_executable(),
    ));
    let interpreter = Interpreter::with_program(program.clone(), runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(
        Arc::new(
            skiff_runtime_service_db::ServiceDbRuntime::new(
                "test".to_string(),
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
        RuntimeTypePlan::from_artifact_type_ref(&skiff_artifact_model::TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![skiff_artifact_model::TypeRefIr::builtin("bytes")],
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
    let payload = error
        .ordinary_payload()
        .expect("resource limit remains ordinary");
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
    assert!(
        error.is_cancellation_terminal(),
        "unexpected error: {error}"
    );
    assert_eq!(error.ordinary_payload(), None);
    assert_eq!(error.ordinary_catch_projection(), None);
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
                Some(&PlatformBuiltinErrorIdentity::JsonDecode.catch_identity())
            );
            assert!(exception.request().local_value().is_some());
        }
        other => panic!("expected uncaught std.json.DecodeError user exception, got {other:?}"),
    }
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

#[test]
fn request_local_rethrow_preserves_identity_source_stack_and_correlation() {
    let identity = local_execution_catch_identity(0);
    let source = test_instruction_site();
    let stack = vec![ExceptionStackFrame::Local {
        site: source.clone(),
    }];
    let correlation = ErrorCorrelation {
        trace_id: "trace-driver-local".to_string(),
        error_id: "trace-driver-local:local-error:1".to_string(),
    };
    let exception = RequestException::local(
        RuntimeValueCarrier::identified(RuntimeValue::from("denied"), identity.clone()),
        source.clone(),
        stack.clone(),
        correlation.clone(),
    )
    .expect("request-local exception");
    let mut heap = RequestHeap::default();
    let handle = heap
        .alloc_exception(exception.clone())
        .expect("request-local exception node");
    let exception_value = RuntimeValueCarrier::unidentified(RuntimeValue::Heap(handle));

    let rethrown = request_exception_for_rethrow(&exception_value, &heap)
        .expect("rethrow must use the existing request-local exception node");

    assert_eq!(rethrown.local_catch_identity(), Some(&identity));
    assert_eq!(rethrown.source(), &source);
    assert_eq!(rethrown.stack(), stack);
    assert_eq!(rethrown.correlation(), &correlation);
    assert_eq!(rethrown, exception);
    assert!(matches!(
        heap.get(handle).expect("same request-local exception node"),
        HeapNode::Exception(stored) if stored == &rethrown
    ));
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
                Some(&local_execution_catch_identity(1))
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
    let program = program_with_thread_db_target(db_negative_offset_executable());
    let service_db = Arc::new(
        skiff_runtime_service_db::ServiceDbRuntime::new(
            "test".to_string(),
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
    let program = program_with_thread_db_target(db_after_executable());
    let service_db = Arc::new(
        skiff_runtime_service_db::ServiceDbRuntime::new(
            "test".to_string(),
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
        implements: Vec::new(),
        source_span: None,
    }];
    program.packages = vec![runtime_package(
        "skiff.run/http-session",
        0,
        vec![Arc::new(package_file)],
        Default::default(),
    )];
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
    assert_eq!(value["mixed"]["target"], thread_db_target_json());
}

#[tokio::test]
async fn runtime_program_db_many_key_selector_is_rejected() {
    let program = Arc::new(program_with_thread_db_target(
        db_many_key_selector_executable(),
    ));
    let interpreter = Interpreter::with_program(program, runtime_factory());
    let mut frame = test_invocation("svc.main.run");
    frame.service_db = Some(
        Arc::new(
            skiff_runtime_service_db::ServiceDbRuntime::new(
                "test".to_string(),
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

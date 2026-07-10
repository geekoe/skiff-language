use std::{
    collections::BTreeMap,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::json;
use skiff_artifact_model::builtin_receiver_op_by_name;
use skiff_runtime_host::eval_capability_adapter as eval_capabilities;
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    runtime_value::{
        HeapNode, InterfaceCarrier, InterfaceValue, RemoteOperationSlot, RemoteOperationTable,
        RuntimeMap, RuntimeObject, RuntimeValue, RuntimeValueKey,
    },
};
use skiff_runtime_service_db::{DbRequestState, ServiceDbCapabilityHandle};

use super::*;
use crate::{
    capability_context::{ConfigCapabilityContext, DbCapabilityContext, FileCapabilitySource},
    config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
    config_view::RuntimeConfigView,
    eval::capabilities::StreamRuntime,
    eval::program::{
        ExecutableKind, FileAddr, FileDeclarations, FileLinkTargets, GatewayConfig, LinkOverlay,
        LinkedExecutableBody, LinkedTypeDescriptor, ParamIr, RuntimeActivation, RuntimeProgram,
        RuntimeTypeContext, ServiceMeta, SlotIr, SlotLayoutIr, TypeAddr, TypeDeclIr, UnitAddr,
    },
    eval::program_execution::ProgramExecutionInput,
    execution_budget::ExecutionBudget,
    host::{file_runtime::FileRuntime, OutboundRequestRegistry},
    request::{ExecutionControl, RequestEnvelope, RuntimeOperation},
};

fn receiver_op(root: &str, method: &str) -> skiff_artifact_model::BuiltinReceiverOp {
    builtin_receiver_op_by_name(root, method).expect("receiver op must exist")
}

fn runtime_factory() -> crate::eval::capabilities::EvalRuntimeFactory {
    eval_capabilities::runtime_factory()
}

fn receiver_builtin_target(root: &str, method: &str) -> serde_json::Value {
    json!({
        "kind": "receiverBuiltin",
        "op": serde_json::to_value(receiver_op(root, method)).unwrap()
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

#[test]
fn map_keys_receiver_returns_snapshot_in_canonical_order() {
    let mut heap = RequestHeap::default();
    let map_value = RuntimeValue::Heap(
        heap.alloc_map(RuntimeMap::from([
            (RuntimeValueKey::string("b"), RuntimeValue::Number(2.0)),
            (RuntimeValueKey::string("a"), RuntimeValue::Number(1.0)),
        ]))
        .unwrap(),
    );

    let keys = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&receiver_op("Map", "keys"), map_value.clone(), vec![])
        .expect("Map.keys should dispatch");
    let RuntimeValue::Heap(keys_handle) = keys.clone() else {
        panic!("Map.keys should return an array heap value");
    };
    assert_eq!(
        heap.get(keys_handle).unwrap(),
        &HeapNode::Array(vec![
            RuntimeValue::String("a".to_string()),
            RuntimeValue::String("b".to_string()),
        ])
    );

    ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(
            &receiver_op("Array", "push"),
            keys,
            vec![RuntimeValue::String("z".to_string())],
        )
        .expect("mutating returned keys array should be local to the array");
    let has_z = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(
            &receiver_op("Map", "has"),
            map_value,
            vec![RuntimeValue::String("z".to_string())],
        )
        .expect("Map.has should dispatch");
    assert_eq!(has_z, RuntimeValue::Bool(false));
}

#[tokio::test]
async fn runtime_program_single_binding_map_for_uses_key_snapshot() {
    let value = call_run_executable_with_args(single_binding_map_for_executable(), |heap| {
        vec![runtime_string_map(
            heap,
            [
                ("b", RuntimeValue::String("B".to_string())),
                ("a", RuntimeValue::String("A".to_string())),
                ("c", RuntimeValue::String("C".to_string())),
            ],
        )]
    })
    .await
    .expect("single-binding map for should execute");

    assert_eq!(value, RuntimeValue::String("abc".to_string()));
}

#[tokio::test]
async fn runtime_program_entry_map_for_uses_entry_snapshot() {
    let value = call_run_executable_with_args(entry_binding_map_for_executable(), |heap| {
        vec![runtime_string_map(
            heap,
            [
                ("b", RuntimeValue::String("B".to_string())),
                ("a", RuntimeValue::String("A".to_string())),
                ("c", RuntimeValue::String("C".to_string())),
            ],
        )]
    })
    .await
    .expect("entry-binding map for should execute");

    assert_eq!(value, RuntimeValue::String("aAbBcC".to_string()));
}

#[tokio::test]
async fn runtime_program_entry_for_rejects_non_map_iterable() {
    let error = call_run_executable_with_args(entry_binding_map_for_executable(), |heap| {
        vec![RuntimeValue::Heap(
            heap.alloc_array(vec![RuntimeValue::String("a".to_string())])
                .unwrap(),
        )]
    })
    .await
    .expect_err("entry binding for non-map should fail closed");

    assert!(matches!(
        error,
        RuntimeError::Decode(message) if message.contains("for entry binding requires Map")
    ));
}

#[tokio::test]
async fn local_const_receiver_explicit_self_rejects_extra_user_arg_instead_of_dropping_it() {
    let mut program = program_with_executables(vec![
        local_const_receiver_extra_arg_route(),
        explicit_self_receiver_echo_executable(),
    ]);
    Arc::make_mut(&mut program.service_files[0])
        .constants
        .push(crate::eval::program::ConstIr {
            name: "managedLlmService".to_string(),
            ty: builtin("Json"),
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
                        "kind": "literal",
                        "value": { "kind": "string", "value": "receiver" }
                    }
                ]
            })),
            source_span: None,
        });
    let activation = Arc::new(runtime_activation_from_program(&program));
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let request = test_request();
    let operation = test_operation();
    let cancelled = Arc::new(AtomicBool::new(false));
    let execution_budget = Arc::new(ExecutionBudget::disabled());
    let config = RuntimeConfigView::empty();
    let package_configs = Vec::new();
    let db_request_state = Arc::new(tokio::sync::Mutex::new(DbRequestState::default()));
    let file_runtime = Arc::new(FileRuntime::new(
        None,
        std::env::temp_dir().join("skiff-runtime-local-receiver-test-file-tmp"),
    ));
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let context = program_execution_context(
        &interpreter,
        &request,
        &operation,
        &activation,
        &cancelled,
        &execution_budget,
        &config,
        &package_configs,
        db_request_state,
        file_runtime,
        interpreter.stream_runtime.clone(),
        &outbound_requests,
    );
    let mut heap = context.request_heap();
    let caller_env = Env::new();
    let run_addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity("file:svc".to_string()),
        executable: 0,
    };

    let error = interpreter
        .call_program_executable(
            context,
            &mut heap,
            &caller_env,
            &run_addr,
            &run_addr,
            &BTreeMap::new(),
            Vec::new(),
        )
        .await
        .expect_err("extra user arg must not be discarded as duplicate self");

    assert!(
        error
            .to_string()
            .contains("callable readSelf expects 2 argument(s), got 3"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn self_type_impl_method_receiver_call_uses_first_arg_as_self() {
    let value = call_run_program_with_args(
        vec![
            self_type_receiver_route_executable(),
            self_type_receiver_echo_executable(),
        ],
        |_| Vec::new(),
    )
    .await
    .expect("selfType receiver call should execute");

    assert_eq!(value, RuntimeValue::String("receiver".to_string()));
}

#[test]
fn receiver_builtin_rejects_wrong_shape_and_map_builtin_still_works() {
    let mut heap = RequestHeap::default();
    let object = RuntimeObject::unshaped(RuntimeObjectFields::from([(
        "name".to_string(),
        RuntimeValue::String("plain".to_string()),
    )]));
    let object_value = RuntimeValue::Heap(heap.alloc_object(object).unwrap());

    let error = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&receiver_op("Map", "length"), object_value, vec![])
        .expect_err("wrong receiver shape should fail closed");
    assert!(
        matches!(error, RuntimeError::Decode(message) if message.contains("receiver:Map.length@1"))
    );

    let map_value = RuntimeValue::Heap(
        heap.alloc_map(RuntimeMap::from([(
            RuntimeValueKey::string("key"),
            RuntimeValue::Bool(true),
        )]))
        .unwrap(),
    );
    let value = ReceiverMethodDispatch::new(&mut heap)
        .dispatch_op(&receiver_op("Map", "length"), map_value, vec![])
        .expect("map length should remain a builtin receiver method");
    assert_eq!(value, RuntimeValue::Number(1.0));
}

#[tokio::test]
async fn interface_box_allocates_interface_heap_node() {
    let (value, heap) =
        call_run_program_with_args_and_heap(vec![interface_box_route_executable()], |_| Vec::new())
            .await
            .expect("interface box should execute");

    let RuntimeValue::Heap(handle) = value else {
        panic!("interface box should return heap wrapper");
    };
    let HeapNode::Interface(value) = heap.get(handle).expect("wrapper should resolve") else {
        panic!("expected interface heap node");
    };
    assert_eq!(value.interface(), "svc.main.Reader");
    let InterfaceCarrier::Local {
        method_table,
        payload,
        ..
    } = value.carrier()
    else {
        panic!("Core boxing must create local carrier");
    };
    assert_eq!(method_table.slots().len(), 1);
    assert!(matches!(payload, RuntimeValue::Heap(_)));
}

#[tokio::test]
async fn interface_method_dispatch_uses_payload_as_self() {
    let value = call_run_program_with_args(
        vec![
            interface_method_route_executable(),
            interface_read_name_impl_executable(),
        ],
        |_| Vec::new(),
    )
    .await
    .expect("interface method dispatch should execute");

    assert_eq!(value, RuntimeValue::String("Ada".to_string()));
}

#[tokio::test]
async fn interface_method_stream_dispatch_returns_deferred_stream_handle() {
    let value = call_run_program_with_args(
        vec![
            interface_stream_for_in_route_executable(),
            shaped_object_stream_impl_method(&builtin("Json")),
        ],
        |_| Vec::new(),
    )
    .await
    .expect("interface method stream dispatch should execute");

    assert_eq!(value, RuntimeValue::String("AB".to_string()));
}

#[tokio::test]
async fn heterogeneous_interface_array_for_in_dispatches_each_concrete_payload() {
    let value = call_run_program_with_args(
        vec![
            heterogeneous_interface_array_route_executable(),
            interface_read_field_impl_executable(
                "svc.main.HostProvider.read",
                builtin("HostProvider"),
                "prefix",
            ),
            interface_read_field_impl_executable(
                "svc.main.DbProvider.read",
                builtin("DbProvider"),
                "table",
            ),
        ],
        |_| Vec::new(),
    )
    .await
    .expect("heterogeneous any-interface array should dispatch each local carrier");

    assert_eq!(value, RuntimeValue::String("hosttools".to_string()));
}

#[tokio::test]
async fn interface_wrapper_self_equality_fails_closed_in_program() {
    let error = call_run_program_with_args(
        vec![
            interface_self_equality_route_executable(),
            interface_read_name_impl_executable(),
        ],
        |_| Vec::new(),
    )
    .await
    .expect_err("same wrapper equality must fail closed instead of using heap identity");

    assert!(
        error.to_string().contains("does not define equality")
            && error.to_string().contains("any interface svc.main.Reader"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn interface_box_remote_source_constructs_remote_carrier_without_payload() {
    let (value, heap) =
        call_run_program_with_args_and_heap(vec![remote_source_interface_box_executable()], |_| {
            Vec::new()
        })
        .await
        .expect("remote interface box source should construct a remote carrier");

    let RuntimeValue::Heap(handle) = value else {
        panic!("remote interface box should return a heap interface value");
    };
    let HeapNode::Interface(interface) = heap.get(handle).expect("interface handle should exist")
    else {
        panic!("remote interface box should allocate an interface value");
    };
    assert_eq!(interface.interface(), READER_INTERFACE_ABI_ID);
    let InterfaceCarrier::Remote {
        dependency_ref,
        public_instance_key,
        operations,
    } = interface.carrier()
    else {
        panic!("remote interface box should not allocate a local payload carrier");
    };
    assert_eq!(dependency_ref, "dep");
    assert_eq!(public_instance_key, "reader");
    assert_eq!(operations.interface_abi_id(), READER_INTERFACE_ABI_ID);
    assert_eq!(
        operations.slots()[0].operation_abi_id(),
        "operation:dep:reader.read"
    );
}

#[tokio::test]
async fn interface_method_remote_carrier_missing_dependency_fails_closed() {
    let error = call_run_executable_with_args(interface_method_arg_route_executable(), |heap| {
        let handle = heap
            .alloc_interface(InterfaceValue::new(
                "svc.main.Reader".to_string(),
                InterfaceCarrier::Remote {
                    dependency_ref: "dep".to_string(),
                    public_instance_key: "reader".to_string(),
                    operations: RemoteOperationTable::new(
                        "remote:reader".to_string(),
                        "svc.main.Reader".to_string(),
                        vec![RemoteOperationSlot::new(
                            0,
                            READER_READ_METHOD_ABI_ID.to_string(),
                            "operation:dep:reader.readName".to_string(),
                        )],
                    ),
                },
            ))
            .unwrap();
        vec![RuntimeValue::Heap(handle)]
    })
    .await
    .expect_err("remote carrier dispatch must fail closed without a declared dependency");

    assert!(
        error
            .to_string()
            .contains("service dependency alias dep is not declared"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn interface_method_non_interface_receiver_fails_closed() {
    let error = call_run_executable_with_args(interface_method_arg_route_executable(), |_| {
        vec![RuntimeValue::String("plain".to_string())]
    })
    .await
    .expect_err("plain receiver must not dispatch as interface");

    assert!(
        error.to_string().contains("not an interface value"),
        "unexpected error: {error}"
    );
}

async fn call_run_executable_with_args(
    executable: LinkedExecutable,
    build_args: impl FnOnce(&mut RequestHeap) -> Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    call_run_program_with_args(vec![executable], build_args).await
}

async fn call_run_program_with_args(
    executables: Vec<LinkedExecutable>,
    build_args: impl FnOnce(&mut RequestHeap) -> Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let (value, _) = call_run_program_with_args_and_heap(executables, build_args).await?;
    Ok(value)
}

async fn call_run_program_with_args_and_heap(
    executables: Vec<LinkedExecutable>,
    build_args: impl FnOnce(&mut RequestHeap) -> Vec<RuntimeValue>,
) -> Result<(RuntimeValue, RequestHeap)> {
    let program = program_with_executables(executables);
    let activation = Arc::new(runtime_activation_from_program(&program));
    let interpreter = Interpreter::with_program(Arc::new(program), runtime_factory());
    let request = test_request();
    let operation = test_operation();
    let cancelled = Arc::new(AtomicBool::new(false));
    let execution_budget = Arc::new(ExecutionBudget::disabled());
    let config = RuntimeConfigView::empty();
    let package_configs = Vec::new();
    let db_request_state = Arc::new(tokio::sync::Mutex::new(DbRequestState::default()));
    let file_runtime = Arc::new(FileRuntime::new(
        None,
        std::env::temp_dir().join("skiff-runtime-map-for-test-file-tmp"),
    ));
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let context = program_execution_context(
        &interpreter,
        &request,
        &operation,
        &activation,
        &cancelled,
        &execution_budget,
        &config,
        &package_configs,
        db_request_state,
        file_runtime,
        interpreter.stream_runtime.clone(),
        &outbound_requests,
    );
    let mut heap = context.request_heap();
    let args = build_args(&mut heap);
    let caller_env = Env::new();
    let run_addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity("file:svc".to_string()),
        executable: 0,
    };

    interpreter
        .call_program_executable(
            context,
            &mut heap,
            &caller_env,
            &run_addr,
            &run_addr,
            &BTreeMap::new(),
            args,
        )
        .await
        .map(|value| (value, heap))
}

fn runtime_string_map<'a>(
    heap: &mut RequestHeap,
    entries: impl IntoIterator<Item = (&'a str, RuntimeValue)>,
) -> RuntimeValue {
    RuntimeValue::Heap(
        heap.alloc_map(
            entries
                .into_iter()
                .map(|(key, value)| (RuntimeValueKey::string(key), value))
                .collect(),
        )
        .unwrap(),
    )
}

const READER_INTERFACE_ABI_ID: &str = "svc.main.Reader";
const READER_READ_METHOD_ABI_ID: &str = "method:svc.main.Reader.read";

fn reader_interface_ref() -> serde_json::Value {
    json!({
        "interfaceAbiId": READER_INTERFACE_ABI_ID,
        "canonicalTypeArgs": []
    })
}

fn reader_interface_method_target() -> serde_json::Value {
    json!({
        "kind": "interfaceMethod",
        "interface": reader_interface_ref(),
        "methodAbiId": READER_READ_METHOD_ABI_ID,
        "slot": 0
    })
}

fn local_reader_box_source(target_executable_index: u32) -> serde_json::Value {
    local_reader_box_source_for(builtin("Json"), target_executable_index)
}

fn local_reader_box_source_for(
    concrete_type: LinkedTypeRef,
    target_executable_index: u32,
) -> serde_json::Value {
    let concrete_type = serde_json::to_value(concrete_type).unwrap();
    let string_type = serde_json::to_value(builtin("String")).unwrap();
    json!({
        "kind": "local",
        "concreteType": concrete_type,
        "methodTable": {
            "interface": reader_interface_ref(),
            "concreteType": concrete_type,
            "slots": [
                {
                    "slot": 0,
                    "methodName": "read",
                    "methodAbiId": READER_READ_METHOD_ABI_ID,
                    "signature": {
                        "params": [],
                        "returnType": string_type
                    },
                    "target": {
                        "executableIndex": target_executable_index,
                        "receiverCallAbi": "explicitSelfFirst"
                    }
                }
            ]
        }
    })
}

fn reader_payload_construct_expressions() -> Vec<serde_json::Value> {
    vec![
        json!({
            "kind": "literal",
            "value": { "kind": "string", "value": "Ada" }
        }),
        json!({
            "kind": "construct",
            "typeRef": serde_json::to_value(builtin("Json")).unwrap(),
            "fields": {
                "name": { "expression": 0 }
            }
        }),
    ]
}

fn interface_box_route_executable() -> LinkedExecutable {
    let mut expressions = reader_payload_construct_expressions();
    expressions.push(json!({
        "kind": "interfaceBox",
        "value": { "expression": 1 },
        "interface": reader_interface_ref(),
        "source": local_reader_box_source(0)
    }));
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("Json")),
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
            "expressions": expressions
        })),
    }
}

fn interface_method_route_executable() -> LinkedExecutable {
    let mut expressions = reader_payload_construct_expressions();
    expressions.push(json!({
        "kind": "interfaceBox",
        "value": { "expression": 1 },
        "interface": reader_interface_ref(),
        "source": local_reader_box_source(1)
    }));
    expressions.push(json!({
        "kind": "call",
        "call": {
            "target": reader_interface_method_target(),
            "args": [
                { "expression": 2 }
            ]
        }
    }));
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("String")),
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
            "expressions": expressions
        })),
    }
}

fn interface_stream_for_in_route_executable() -> LinkedExecutable {
    let mut expressions = reader_payload_construct_expressions();
    expressions.push(json!({
        "kind": "interfaceBox",
        "value": { "expression": 1 },
        "interface": reader_interface_ref(),
        "source": local_reader_box_source(1)
    }));
    expressions.push(json!({
        "kind": "call",
        "call": {
            "target": reader_interface_method_target(),
            "args": [
                { "expression": 2 }
            ]
        }
    }));
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("String")),
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
                    "value": { "expression": 4 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 3 },
                    "body": "body"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 0 },
                    "value": { "expression": 7 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 8 }
                }
            ],
            "expressions": [
                expressions[0].clone(),
                expressions[1].clone(),
                expressions[2].clone(),
                expressions[3].clone(),
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 5 },
                    "right": { "expression": 6 }
                },
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn heterogeneous_interface_array_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("String")),
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
                    name: "provider".to_string(),
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
                    "value": { "expression": 7 }
                },
                {
                    "kind": "forIn",
                    "itemSlot": 1,
                    "iterable": { "expression": 6 },
                    "body": "body"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 0 },
                    "value": { "expression": 11 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 12 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "host" } },
                {
                    "kind": "construct",
                    "typeRef": serde_json::to_value(builtin("Json")).unwrap(),
                    "fields": {
                        "prefix": { "expression": 0 }
                    }
                },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 1 },
                    "interface": reader_interface_ref(),
                    "source": local_reader_box_source_for(builtin("HostProvider"), 1)
                },
                { "kind": "literal", "value": { "kind": "string", "value": "tools" } },
                {
                    "kind": "construct",
                    "typeRef": serde_json::to_value(builtin("Json")).unwrap(),
                    "fields": {
                        "table": { "expression": 3 }
                    }
                },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 4 },
                    "interface": reader_interface_ref(),
                    "source": local_reader_box_source_for(builtin("DbProvider"), 2)
                },
                {
                    "kind": "arrayLiteral",
                    "items": [
                        { "expression": 2 },
                        { "expression": 5 }
                    ]
                },
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "call",
                    "call": {
                        "target": reader_interface_method_target(),
                        "args": [
                            { "expression": 9 }
                        ]
                    }
                },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 8 },
                    "right": { "expression": 10 }
                },
                { "kind": "loadSlot", "slot": 0 }
            ]
        })),
    }
}

fn interface_self_equality_route_executable() -> LinkedExecutable {
    let mut expressions = reader_payload_construct_expressions();
    expressions.push(json!({
        "kind": "interfaceBox",
        "value": { "expression": 1 },
        "interface": reader_interface_ref(),
        "source": local_reader_box_source(1)
    }));
    expressions.extend([
        json!({ "kind": "loadSlot", "slot": 0 }),
        json!({ "kind": "loadSlot", "slot": 0 }),
        json!({
            "kind": "binary",
            "op": "equal",
            "left": { "expression": 3 },
            "right": { "expression": 4 }
        }),
    ]);
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("Bool")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "provider".to_string(),
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
                        { "statement": 1 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "let",
                    "slot": 0,
                    "value": { "expression": 2 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 5 }
                }
            ],
            "expressions": expressions
        })),
    }
}

fn remote_source_interface_box_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("Json")),
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
                    "kind": "literal",
                    "value": { "kind": "string", "value": "payload" }
                },
                {
                    "kind": "interfaceBox",
                    "value": { "expression": 0 },
                    "interface": reader_interface_ref(),
                    "source": {
                        "kind": "remote",
                        "dependencyRef": "dep",
                        "publicInstanceKey": "reader",
                        "operations": {
                            "interface": reader_interface_ref(),
                            "slots": [
                                {
                                    "slot": 0,
                                    "methodAbiId": READER_READ_METHOD_ABI_ID,
                                    "signature": {
                                        "params": [],
                                        "returnType": { "kind": "builtin", "name": "String" }
                                    },
                                    "operationAbiId": "operation:dep:reader.read"
                                }
                            ]
                        },
                        "calleeProtocolIdentity": "protocol:dep"
                    }
                }
            ]
        })),
    }
}

fn interface_method_arg_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "provider".to_string(),
            slot: 0,
            ty: builtin("Json"),
        }],
        return_type: Some(builtin("String")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "provider".to_string(),
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
                        "target": reader_interface_method_target(),
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                }
            ]
        })),
    }
}

fn interface_read_field_impl_executable(
    symbol: &str,
    self_type: LinkedTypeRef,
    field: &str,
) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("String")),
        self_type: Some(self_type),
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
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "field",
                    "object": { "expression": 0 },
                    "field": field
                }
            ]
        })),
    }
}

fn interface_read_name_impl_executable() -> LinkedExecutable {
    interface_read_field_impl_executable("svc.main.ReaderImpl.read", builtin("Json"), "name")
}

fn single_binding_map_for_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: builtin("Json"),
        }],
        return_type: Some(builtin("String")),
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
                    name: "out".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 2,
                    name: "key".to_string(),
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
                        { "statement": 4 }
                    ]
                },
                {
                    "label": "body",
                    "statements": [
                        { "statement": 2 },
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
                    "iterable": { "expression": 6 },
                    "body": "body"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 1 },
                    "value": { "expression": 3 }
                },
                {
                    "kind": "expr",
                    "value": { "expression": 5 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "loadSlot", "slot": 2 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 1 },
                    "right": { "expression": 2 }
                },
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "target": receiver_builtin_target("Map", "delete"),
                        "args": [
                            { "expression": 4 },
                            { "expression": 2 }
                        ]
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 }
            ]
        })),
    }
}

fn entry_binding_map_for_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: builtin("Json"),
        }],
        return_type: Some(builtin("String")),
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
                    name: "out".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 2,
                    name: "key".to_string(),
                    kind: "local".to_string(),
                },
                SlotIr {
                    index: 3,
                    name: "value".to_string(),
                    kind: "local".to_string(),
                },
            ],
            frame_size: 4,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 4 }
                    ]
                },
                {
                    "label": "body",
                    "statements": [
                        { "statement": 2 },
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
                    "valueSlot": 3,
                    "iterable": { "expression": 10 },
                    "body": "body"
                },
                {
                    "kind": "assign",
                    "target": { "kind": "slot", "slot": 1 },
                    "value": { "expression": 5 }
                },
                {
                    "kind": "expr",
                    "value": { "expression": 9 }
                },
                {
                    "kind": "return",
                    "value": { "expression": 11 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "loadSlot", "slot": 2 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 1 },
                    "right": { "expression": 2 }
                },
                { "kind": "loadSlot", "slot": 3 },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 3 },
                    "right": { "expression": 4 }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "string", "value": "c" } },
                { "kind": "literal", "value": { "kind": "string", "value": "Z" } },
                {
                    "kind": "call",
                    "call": {
                        "target": receiver_builtin_target("Map", "set"),
                        "args": [
                            { "expression": 6 },
                            { "expression": 7 },
                            { "expression": 8 }
                        ]
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 }
            ]
        })),
    }
}

fn local_const_receiver_extra_arg_route() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("Json")),
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
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "first-user-arg" }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "second-user-arg" }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": local_const_receiver_target(1),
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

fn explicit_self_receiver_echo_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "readSelf".to_string(),
        type_params: Vec::new(),
        params: vec![
            ParamIr {
                name: "self".to_string(),
                slot: 0,
                ty: builtin("Json"),
            },
            ParamIr {
                name: "input".to_string(),
                slot: 1,
                ty: builtin("Json"),
            },
        ],
        return_type: Some(builtin("Json")),
        self_type: Some(builtin("Json")),
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
                { "kind": "loadSlot", "slot": 1 }
            ]
        })),
    }
}

fn self_type_receiver_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(builtin("Json")),
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
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "receiver" }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "input" }
                },
                {
                    "kind": "call",
                    "call": {
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
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

fn self_type_receiver_echo_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "readSelf".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "input".to_string(),
            slot: 1,
            ty: builtin("Json"),
        }],
        return_type: Some(builtin("Json")),
        self_type: Some(builtin("Json")),
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

fn shaped_object_impl_method(store_type_ref: &LinkedTypeRef) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "svc.main.RecordingStore.recordedName".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "self".to_string(),
            slot: 0,
            ty: store_type_ref.clone(),
        }],
        return_type: Some(builtin("String")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "self".to_string(),
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
                    "kind": "field",
                    "object": { "expression": 0 },
                    "field": "name"
                }
            ]
        })),
    }
}

fn shaped_object_stream_impl_method(store_type_ref: &LinkedTypeRef) -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: "svc.main.RecordingStore.streamItems".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "self".to_string(),
            slot: 0,
            ty: store_type_ref.clone(),
        }],
        return_type: Some(stream_of(builtin("string"))),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "self".to_string(),
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
                        { "statement": 0 },
                        { "statement": 1 }
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
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "A" } },
                { "kind": "literal", "value": { "kind": "string", "value": "B" } }
            ]
        })),
    }
}

fn add_store_type(program: &mut RuntimeProgram, addr: TypeAddr) {
    let declaration = TypeDeclIr {
        name: "svc.main.RecordingStore".to_string(),
        descriptor: LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([("name".to_string(), builtin("String"))]),
        },
        type_params: Vec::new(),
        discriminator: None,
        implements: Vec::new(),
        source_span: None,
    };
    Arc::make_mut(&mut program.service_files[0])
        .types
        .push(declaration.clone());
    program.types.descriptors.insert(addr, declaration);
}

fn program_execution_context<'a>(
    interpreter: &Interpreter,
    request: &'a RequestEnvelope,
    operation: &'a RuntimeOperation,
    activation: &'a Arc<RuntimeActivation>,
    cancelled: &'a Arc<AtomicBool>,
    execution_budget: &'a Arc<ExecutionBudget>,
    config: &'a RuntimeConfigView,
    package_configs: &'a [RuntimeConfigView],
    db_request_state: Arc<tokio::sync::Mutex<DbRequestState>>,
    file_runtime: Arc<FileRuntime>,
    stream_runtime: StreamRuntime,
    outbound_requests: &'a Arc<OutboundRequestRegistry>,
) -> ProgramExecutionContext<'a> {
    let concrete_execution = ExecutionControl::new(cancelled, execution_budget);
    let execution = eval_capabilities::execution_control(concrete_execution);
    let db = eval_capabilities::db_context(DbCapabilityContext::from_handle(
        ServiceDbCapabilityHandle::with_state(None, db_request_state),
    ));
    let actor = eval_capabilities::actor_from_request(
        "runtime-program",
        "svc",
        "v1",
        request,
        operation,
        None,
        outbound_requests,
        cancelled.as_ref(),
        execution.cancel_flag(),
    );
    let effects = eval_capabilities::effects(eval_capabilities::effect_dispatch_context_from_request(
        request,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        cancelled.clone(),
        None,
    ));
    let file = eval_capabilities::file_source(FileCapabilitySource::new(file_runtime))
        .context_for_request(db.clone());
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: eval_capabilities::config_context(ConfigCapabilityContext::new(
            config,
            package_configs,
        )),
        db: db.clone(),
        file,
        file_source_stream: crate::eval::capabilities::FileSourceStreamContext::new(
            stream_runtime.clone(),
            execution.clone(),
        ),
        time: crate::eval::capabilities::TimeCapabilityContext::new(execution.clone()),
        websocket: eval_capabilities::websocket_from_request("svc", None, None),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options,
            stream_runtime,
            interpreter.test_effect_double_context(),
        ),
        runtime_activation: activation.clone(),
        actor: actor.clone(),
        spawn: actor,
        outbound: eval_capabilities::outbound(
            eval_capabilities::outbound_service_context_from_request(
                request,
                execution_budget.clone(),
                execution.cancel_flag(),
                RequestHeapLimits::default(),
                None,
                outbound_requests.clone(),
                &activation.service_dependencies,
                &activation.timeout,
            ),
        ),
        request_heap_limits: RequestHeapLimits::default(),
    })
}

fn program_with_executables(executables: Vec<LinkedExecutable>) -> RuntimeProgram {
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
        package_configs: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: Vec::new(),
        routes: Default::default(),
        spawn_routes: Default::default(),
        operations: Default::default(),
        operation_receivers: Default::default(),
        db: Vec::new(),
        actors: Vec::new(),
        link_overlay: LinkOverlay::default(),
        gateway: GatewayConfig::default(),
        types: RuntimeTypeContext::default(),
    }
}

fn runtime_activation_from_program(program: &RuntimeProgram) -> RuntimeActivation {
    RuntimeActivation {
        service: program.service.clone(),
        version: program.version.clone(),
        package_configs: program.package_configs.clone(),
        service_dependencies: program.service_dependencies.clone(),
        timeout: program.timeout.clone(),
        operation_route_bindings: program.operation_route_bindings.clone(),
        db: program.db.clone(),
        actors: program.actors.clone(),
        gateway: program.gateway.clone(),
    }
}

fn test_request() -> RequestEnvelope {
    RequestEnvelope {
        request_id: "request-program".to_string(),
        mode: "unary".to_string(),
        target: "svc.main.run".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        build_id: "build:program".to_string(),
        service_protocol_identity: String::new(),
        contract_identity: None,
        activation_identity: None,
        http_adapter: None,
        websocket_adapter: None,
        binary_http: None,
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn test_operation() -> RuntimeOperation {
    RuntimeOperation {
        operation_abi_id: None,
        operation: "run".to_string(),
        target: "svc.main.run".to_string(),
        mode: "unary".to_string(),
        parameters: Vec::new(),
        service_protocol_identity: None,
        extra: serde_json::Map::new(),
    }
}

fn service_type_addr(type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Service,
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn builtin(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn stream_of(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Stream".to_string(),
        args: vec![item],
    }
}

fn executable_body(value: serde_json::Value) -> LinkedExecutableBody {
    serde_json::from_value(value).unwrap()
}

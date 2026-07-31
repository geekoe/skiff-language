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

use super::super::*;
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

use super::program::*;
use super::runtime::*;
use super::stream_executables::*;
use skiff_runtime_native::dispatch::NativeDispatch;

pub(crate) fn executable_body(value: Value) -> LinkedExecutableBody {
    serde_json::from_value(value).expect("typed executable body should deserialize")
}

pub(crate) fn expression(value: Value) -> LinkedExprIr {
    serde_json::from_value(value).expect("typed expression should deserialize")
}

pub(crate) fn statement(value: Value) -> LinkedStmtIr {
    serde_json::from_value(value).expect("typed statement should deserialize")
}

pub(crate) fn old_db_builtin_executable() -> LinkedExecutable {
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

pub(crate) fn db_negative_offset_executable() -> LinkedExecutable {
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
                        "target": thread_db_target_json(),
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

pub(crate) fn db_after_executable() -> LinkedExecutable {
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
                        "target": thread_db_target_json(),
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

pub(crate) fn db_query_value_executable() -> LinkedExecutable {
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

pub(crate) fn db_many_key_selector_executable() -> LinkedExecutable {
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
                        "target": thread_db_target_json(),
                        "selector": { "kind": "key", "value": { "expression": 0 } },
                        "resultType": { "kind": "builtin", "name": "Json" }
                    }
                }
            ]
        })),
    }
}

pub(crate) fn db_call_expr_without_type<const N: usize>(op: &str, args: [Value; N]) -> Value {
    let args = args.into_iter().collect::<Vec<_>>();
    json!({
        "kind": "call",
        "call": {
            "site": test_instruction_site(),
            "target": {
                "kind": "builtin",
                "op": op
            },
            "args": args
        }
    })
}

pub(crate) fn push_expr(expressions: &mut Vec<Value>, expression: Value) -> usize {
    let index = expressions.len();
    expressions.push(expression);
    index
}

pub(crate) fn expr_ref_json(index: usize) -> Value {
    json!({ "expression": index })
}

pub(crate) fn db_query_value_expr(query: Value) -> Value {
    json!({
        "kind": "dbQuery",
        "target": thread_db_target_json(),
        "query": query,
        "resultType": { "kind": "builtin", "name": "Json" }
    })
}

pub(crate) fn db_query(predicates: Vec<Value>) -> Value {
    if predicates.is_empty() {
        json!({})
    } else {
        json!({ "where": predicates })
    }
}

pub(crate) fn db_predicate_compare(field: &str, op: &str, value: Value) -> Value {
    json!({
        "kind": "compare",
        "field": db_field_path_json(field),
        "op": op,
        "value": value
    })
}

pub(crate) fn db_predicate_conditional(condition: Value, predicate: Value) -> Value {
    json!({ "kind": "conditional", "condition": condition, "predicate": predicate })
}

pub(crate) fn db_field_path_json(field: &str) -> Value {
    let segments = field.split('.').map(str::to_string).collect::<Vec<_>>();
    json!({ "text": field, "segments": segments })
}

pub(crate) fn thread_db_target_json() -> Value {
    json!({
        "targetId": thread_db_object_target_id(0),
        "typeRef": { "kind": "dbObjectSymbol", "symbol": { "modulePath": "svc.main", "symbol": "Thread" } },
        "typeName": "Thread"
    })
}

pub(crate) fn thread_db_object_target_id(index: usize) -> DbObjectTargetId {
    let file_ir_identity = format!("test-file-Thread-{index}");
    DbObjectTargetId {
        package_artifact_ref: PackageArtifactRef {
            package_id: format!("test.local/provider-Thread-{index}"),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new(format!("test-build-Thread-{index}")),
            package_local_abi_identity: PackageLocalAbiIdentity::new(format!(
                "test-abi-Thread-{index}"
            )),
        },
        file_ir_ref: FileIrRef {
            source_ast_hash: Some(format!("source:{file_ir_identity}")),
            file_ir_identity,
            module_path: "svc.main".to_string(),
            artifact_path: None,
        },
        type_index: index,
    }
}

pub(crate) fn literal_string_expr(value: &str) -> Value {
    json!({ "kind": "literal", "value": { "kind": "string", "value": value } })
}

pub(crate) fn literal_number_expr(value: i64) -> Value {
    json!({ "kind": "literal", "value": { "kind": "number", "value": value } })
}

pub(crate) fn literal_bool_expr(value: bool) -> Value {
    json!({ "kind": "literal", "value": { "kind": "bool", "value": value } })
}

pub(crate) fn parameter_slot_def_executable() -> LinkedExecutable {
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

pub(crate) fn self_local_call_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn receiver_builtin_array_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn bytes_concat_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
                        "target": receiver_builtin_target("bytes", "toUtf8String"),
                        "args": [{ "expression": 5 }]
                    }
                }
            ]
        })),
    }
}

pub(crate) fn bytes_from_utf8_invalid_arg_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn time_sleep_executable(ms: i64) -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn read_self_executable() -> LinkedExecutable {
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

pub(crate) fn package_call_executable() -> LinkedExecutable {
    package_call_executable_with_package_ref(json!({
        "kind": "packageId",
        "packageId": "example.com/pkg"
    }))
}

pub(crate) fn package_call_executable_with_package_ref(package_ref: Value) -> LinkedExecutable {
    package_call_executable_with_symbol(json!({
        "package": package_ref,
        "symbolPath": "pkg.echo"
    }))
}

pub(crate) fn package_call_executable_with_symbol(_symbol: Value) -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn telemetry_emit_native_direct_call_executable() -> LinkedExecutable {
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
                        site: test_instruction_site(),
                        args: vec![
                            ExprRefIr { expression: 0 },
                            ExprRefIr { expression: 1 },
                            ExprRefIr { expression: 3 },
                        ],
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                        actor_metadata: None,
                    },
                },
            ],
        },
    }
}

pub(crate) fn resource_text_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "text",
        "std.resource.text",
        path,
        None,
        builtin_type("string"),
    )
}

pub(crate) fn catch_resource_text_native_executable(path: &str) -> LinkedExecutable {
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
            "blocks": [{
                "label": "entry",
                "statements": [{
                    "statement": 0
                }]
            }],
            "statements": [{
                "kind": "return",
                "value": {
                    "expression": 2
                }
            }],
            "expressions": [
                {
                    "kind": "literal",
                    "value": {
                        "kind": "string",
                        "value": path
                    }
                },
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "native",
                            "target": {
                                "namespace": "std.resource",
                                "symbol": "text",
                                "bindingKey": "std.resource.text"
                            }
                        },
                        "args": [{
                            "expression": 0
                        }]
                    }
                },
                {
                    "kind": "catch",
                    "tryExpression": {
                        "expression": 1
                    },
                    "catchSlot": 0,
                    "catchType": {
                        "kind": "address",
                        "addr": serde_json::to_value(
                            std_http_type_addr(STD_RESOURCE_ERROR_TYPE_INDEX)
                        ).unwrap()
                    },
                    "body": {
                        "expression": 0
                    }
                }
            ]
        })),
    }
}

pub(crate) fn resource_bytes_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "bytes",
        "std.resource.bytes",
        path,
        None,
        builtin_type("bytes"),
    )
}

pub(crate) fn resource_info_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "info",
        "std.resource.info",
        path,
        None,
        std_http_type_ref(STD_RESOURCE_INFO_TYPE_INDEX),
    )
}

pub(crate) fn resource_exists_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "exists",
        "std.resource.exists",
        path,
        None,
        builtin_type("bool"),
    )
}

pub(crate) fn resource_json_object_native_executable(path: &str) -> LinkedExecutable {
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

pub(crate) fn resource_json_stream_native_executable(path: &str) -> LinkedExecutable {
    resource_native_executable(
        "json",
        "std.resource.json",
        path,
        Some(json!({
            "T0": {
                "kind": "builtin",
                "name": "Stream",
                "args": [{ "kind": "builtin", "name": "string" }]
            }
        })),
        linked_stream_type(builtin_type("string")),
    )
}

pub(crate) fn resource_native_executable(
    symbol: &str,
    binding_key: &str,
    path: &str,
    type_args: Option<Value>,
    return_type: LinkedTypeRef,
) -> LinkedExecutable {
    let mut call = json!({
        "site": test_instruction_site(),
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

pub(crate) fn service_calls_package_resource_text_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn resource_table(path: &str, bytes: &[u8]) -> PublicationResourceTable {
    let mut table = PublicationResourceTable::default();
    table.insert(path.to_string(), loaded_resource(path, bytes));
    table
}

pub(crate) fn loaded_resource(path: &str, bytes: &[u8]) -> LoadedPublicationResource {
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

pub(crate) fn builtin_type(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

pub(crate) fn assert_resource_exception(error: &RuntimeError) {
    let RuntimeError::UserException(exception) = runtime_error_leaf(error) else {
        panic!("expected request-local std.resource.ResourceError, got {error:?}");
    };
    assert_resource_request_exception(exception.request());
}

pub(crate) fn assert_resource_request_exception(exception: &RequestException) {
    let expected_site = test_instruction_site();
    assert_eq!(
        exception.local_catch_identity(),
        Some(&local_execution_catch_identity_for_addr(
            std_http_type_addr(STD_RESOURCE_ERROR_TYPE_INDEX)
        ))
    );
    assert!(exception.local_value().is_some());
    assert_eq!(exception.source(), &expected_site);
    assert_eq!(
        exception.stack(),
        [ExceptionStackFrame::Local {
            site: expected_site,
        }]
    );
}

pub(crate) fn assert_resource_error_invalid_artifact(error: &RuntimeError) {
    let RuntimeError::InvalidArtifact(message) = runtime_error_leaf(error) else {
        panic!("expected invalid ResourceError projection artifact, got {error:?}");
    };
    assert!(
        message.contains("std.resource.ResourceError"),
        "invalid artifact should identify the required ResourceError type: {message}"
    );
}

pub(crate) fn assert_resource_json_decode_exception(error: &RuntimeError) {
    let RuntimeError::UserException(exception) = runtime_error_leaf(error) else {
        panic!("expected request-local std.json.DecodeError, got {error:?}");
    };
    let expected_site = test_instruction_site();
    assert_eq!(
        exception.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::JsonDecode.catch_identity())
    );
    assert!(exception.request().local_value().is_some());
    assert_eq!(exception.request().source(), &expected_site);
    assert_eq!(
        exception.request().stack(),
        [ExceptionStackFrame::Local {
            site: expected_site,
        }]
    );
}

pub(crate) fn assert_json_decode_exception_identity(error: &RuntimeError) {
    let RuntimeError::UserException(exception) = runtime_error_leaf(error) else {
        panic!("expected request-local std.json.DecodeError, got {error:?}");
    };
    assert_eq!(
        exception.actual_payload_type(),
        Some(&PlatformBuiltinErrorIdentity::JsonDecode.catch_identity())
    );
    assert!(exception.request().local_value().is_some());
}

pub(crate) fn stream_route_slots() -> SlotLayoutIr {
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

pub(crate) fn match_executable() -> LinkedExecutable {
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

pub(crate) fn type_pattern_match_executable() -> LinkedExecutable {
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

pub(crate) fn catch_builtin_decode_error_throw_with_catch_type_executable(
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
                    "kind": "construct",
                    "typeRef": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    },
                    "fields": {
                        "target": { "expression": 0 },
                        "message": { "expression": 1 }
                    }
                },
                {
                    "kind": "throw",
                    "site": test_instruction_site(),
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

pub(crate) fn catch_literal_with_catch_type_executable(catch_type_name: &str) -> LinkedExecutable {
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

pub(crate) fn catch_throw_with_type_addrs_executable(
    throw_type_addr: TypeAddr,
    catch_type_addr: TypeAddr,
) -> LinkedExecutable {
    let catch_expression = json!({
        "kind": "catch",
        "tryExpression": { "expression": 2 },
        "catchSlot": 0,
        "catchType": {
            "kind": "address",
            "addr": serde_json::to_value(catch_type_addr).unwrap()
        },
        "body": { "expression": 4 }
    });

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
                    "kind": "construct",
                    "typeRef": {
                        "kind": "address",
                        "addr": serde_json::to_value(&throw_type_addr).unwrap()
                    },
                    "fields": {
                        "message": { "expression": 0 }
                    }
                },
                {
                    "kind": "throw",
                    "site": test_instruction_site(),
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

pub(crate) fn assert_executable(condition: bool) -> LinkedExecutable {
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

pub(crate) fn package_echo_executable() -> LinkedExecutable {
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

pub(crate) fn package_stream_chain_route_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(linked_builtin_type("string")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                test_slot(0, "source", "local"),
                test_slot(1, "forwarded", "local"),
                test_slot(2, "item", "local"),
                test_slot(3, "result", "local"),
            ],
            frame_size: 4,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                { "label": "entry", "statements": [
                    { "statement": 0 }, { "statement": 1 }, { "statement": 2 },
                    { "statement": 3 }, { "statement": 4 }
                ] },
                { "label": "append", "statements": [{ "statement": 5 }] }
            ],
            "statements": [
                { "kind": "let", "slot": 0, "value": { "expression": 0 } },
                { "kind": "let", "slot": 1, "value": { "expression": 2 } },
                { "kind": "let", "slot": 3, "value": { "expression": 4 } },
                {
                    "kind": "forIn", "itemSlot": 2,
                    "iterable": { "expression": 3 }, "body": "append"
                },
                { "kind": "return", "value": { "expression": 8 } },
                {
                    "kind": "assign", "target": { "kind": "slot", "slot": 3 },
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::service(0, 1)).unwrap()
                        },
                        "args": []
                    }
                },
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(ExecutableAddr::package(0, 0, 0)).unwrap()
                        },
                        "args": [{ "expression": 1 }]
                    }
                },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "literal", "value": { "kind": "string", "value": "" } },
                { "kind": "loadSlot", "slot": 3 },
                { "kind": "loadSlot", "slot": 2 },
                {
                    "kind": "binary", "op": "add",
                    "left": { "expression": 5 }, "right": { "expression": 6 }
                },
                { "kind": "loadSlot", "slot": 3 }
            ]
        })),
    }
}

pub(crate) fn package_string_stream_forwarder_executable(
    executable_index: usize,
    nested_index: Option<usize>,
) -> LinkedExecutable {
    let (iterable_expression, item_expression, expressions) = match nested_index {
        Some(nested_index) => (
            1,
            2,
            json!([
                { "kind": "loadSlot", "slot": 0 },
                {
                    "kind": "call",
                    "call": {
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "executable",
                            "addr": serde_json::to_value(
                                ExecutableAddr::package(0, 0, nested_index)
                            ).unwrap()
                        },
                        "args": [{ "expression": 0 }]
                    }
                },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]),
        ),
        None => (
            0,
            1,
            json!([
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "loadSlot", "slot": 1 },
                { "kind": "literal", "value": { "kind": "null" } }
            ]),
        ),
    };
    let return_expression = expressions
        .as_array()
        .expect("forwarder expressions should be an array")
        .len()
        - 1;
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: format!("forwarders.forward{executable_index}"),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "source".to_string(),
            slot: 0,
            ty: linked_stream_type(linked_builtin_type("string")),
        }],
        return_type: Some(linked_stream_type(linked_builtin_type("string"))),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                test_slot(0, "source", "param"),
                test_slot(1, "item", "local"),
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: executable_body(json!({
            "blocks": [
                { "label": "entry", "statements": [{ "statement": 1 }, { "statement": 2 }] },
                { "label": "forward", "statements": [{ "statement": 0 }] }
            ],
            "statements": [
                { "kind": "emit", "operation": "emit", "value": { "expression": item_expression } },
                {
                    "kind": "forIn", "itemSlot": 1,
                    "iterable": { "expression": iterable_expression }, "body": "forward"
                },
                { "kind": "return", "value": { "expression": return_expression } }
            ],
            "expressions": expressions
        })),
    }
}

pub(crate) fn test_slot(index: usize, name: &str, kind: &str) -> SlotIr {
    SlotIr {
        index,
        name: name.to_string(),
        kind: kind.to_string(),
    }
}

pub(crate) fn package_file_unit(
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
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![executable],
        external_refs: Default::default(),
    }
}

pub(crate) fn package_call_config_reader_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn config_require_string_executable(path: &str) -> LinkedExecutable {
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
                        "site": test_instruction_site(),
                        "target": {
                            "kind": "builtin",
                            "op": "config.require"
                        },
                        "args": [
                            { "expression": 0 }
                        ],
                        "typeArgs": {
                            "T0": { "kind": "builtin", "name": "string" }
                        }
                    }
                }
            ]
        })),
    }
}

pub(crate) fn package_generic_json_decode_call_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn package_generic_config_require_call_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn generic_json_decode_native_wrapper_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn generic_config_require_wrapper_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn json_decode_native_missing_type_args_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn json_decode_native_missing_binding_key_executable() -> LinkedExecutable {
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

pub(crate) fn json_encode_native_missing_type_args_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn json_decode_native_missing_t0_type_arg_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn json_decode_native_unresolved_type_arg_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn json_decode_native_target_metadata_executable() -> LinkedExecutable {
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

pub(crate) fn json_native_direct_type_args_with_nullable_json_object_return_executable(
) -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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

pub(crate) fn run_executable() -> LinkedExecutable {
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

pub(crate) fn explicit_self_route_executable() -> LinkedExecutable {
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

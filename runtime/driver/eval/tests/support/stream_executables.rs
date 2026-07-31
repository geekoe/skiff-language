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

use super::executables::*;
use super::program::*;
use super::runtime::*;
use skiff_runtime_native::dispatch::NativeDispatch;

pub(crate) fn for_in_value_block_executable() -> LinkedExecutable {
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

pub(crate) fn local_stream_aggregate_route_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn local_stream_first_item_route_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn local_const_receiver_stream_first_item_route_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn local_string_stream_producer_executable() -> LinkedExecutable {
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

pub(crate) fn forwarding_string_stream_producer_executable(next_index: usize) -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn local_http_sse_response_stream_producer_executable() -> LinkedExecutable {
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

pub(crate) fn outer_string_stream_from_sse_producer_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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

pub(crate) fn sse_tag_string_stream_converter_executable() -> LinkedExecutable {
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

pub(crate) fn local_const_receiver_stream_producer_executable() -> LinkedExecutable {
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

pub(crate) fn stream_variable_json_object_length_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn create_from_stream_route_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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
                        "site": test_instruction_site(),
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

pub(crate) fn bytes_stream_emit_then_typed_throw_producer_executable() -> LinkedExecutable {
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
                    "kind": "expr",
                    "value": { "expression": 5 }
                }
            ],
            "expressions": [
                { "kind": "literal", "value": { "kind": "string", "value": "ok" } },
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
                        "args": [
                            { "expression": 0 }
                        ]
                    }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "fixture.stream.producer" }
                },
                {
                    "kind": "literal",
                    "value": { "kind": "string", "value": "typed producer terminal" }
                },
                {
                    "kind": "construct",
                    "typeRef": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    },
                    "fields": {
                        "target": { "expression": 2 },
                        "message": { "expression": 3 }
                    }
                },
                {
                    "kind": "throw",
                    "site": test_instruction_site(),
                    "value": { "expression": 4 },
                    "payloadType": {
                        "kind": "builtin",
                        "name": "std.json.DecodeError"
                    }
                }
            ]
        })),
    }
}

pub(crate) fn emit_response_stream_helper_executable() -> LinkedExecutable {
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

pub(crate) fn emit_response_stream_call_ir() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Native {
            target: NativeTarget {
                namespace: "std.http".to_string(),
                symbol: "emitResponseStream".to_string(),
                binding_key: Some("std.http.stream.emitResponse".to_string()),
                metadata: BTreeMap::new(),
            },
        },
        site: test_instruction_site(),
        args: vec![ExprRefIr { expression: 0 }],
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

pub(crate) fn create_from_stream_call_ir() -> CallIr {
    CallIr {
        target: LinkedCallTarget::Native {
            target: NativeTarget {
                namespace: "std.file".to_string(),
                symbol: "createFromStream".to_string(),
                binding_key: Some("std.file.createFromStream".to_string()),
                metadata: BTreeMap::new(),
            },
        },
        site: test_instruction_site(),
        args: vec![ExprRefIr { expression: 0 }, ExprRefIr { expression: 1 }],
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

pub(crate) fn http_stream_chunk_value(heap: &mut RequestHeap, bytes: &[u8]) -> RuntimeValue {
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

pub(crate) fn local_native_stream_wrapper_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn local_native_sse_forwarding_stream_producer_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn http_stream_effect_in_http_handler_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn http_stream_start_helper_in_http_handler_executable() -> LinkedExecutable {
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
                        "site": test_instruction_site(),
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

pub(crate) fn http_stream_chunk_helper_in_http_handler_executable() -> LinkedExecutable {
    http_stream_event_helper_executable(
        "streamChunk",
        "std.http.stream.chunk",
        vec![json!({
            "kind": "field",
            "object": { "expression": 0 },
            "field": "body"
        })],
        vec![json!({ "expression": 1 })],
    )
}

pub(crate) fn http_stream_end_helper_in_http_handler_executable() -> LinkedExecutable {
    http_stream_event_helper_executable("streamEnd", "std.http.stream.end", vec![], vec![])
}

pub(crate) fn http_stream_event_helper_executable(
    symbol: &str,
    binding_key: &str,
    extra_expressions: Vec<Value>,
    args: Vec<Value>,
) -> LinkedExecutable {
    let mut expressions = vec![json!({ "kind": "loadSlot", "slot": 0 })];
    expressions.extend(extra_expressions);
    let call_index = expressions.len();
    expressions.push(json!({
        "kind": "call",
        "call": {
            "site": test_instruction_site(),
            "target": {
                "kind": "native",
                "target": {
                    "namespace": "std.http",
                    "symbol": symbol,
                    "bindingKey": binding_key
                }
            },
            "args": args
        }
    }));
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
            "blocks": [{
                "label": "entry",
                "statements": [{ "statement": 0 }]
            }],
            "statements": [{
                "kind": "return",
                "value": { "expression": call_index }
            }],
            "expressions": expressions
        })),
    }
}

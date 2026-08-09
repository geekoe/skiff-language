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
use super::stream_executables::*;
use skiff_runtime_native::dispatch::NativeDispatch;

pub(crate) fn runtime_factory() -> crate::eval::capabilities::EvalRuntimeFactory {
    eval_capability_adapter::runtime_factory()
}

pub(crate) fn test_instruction_site() -> skiff_artifact_model::InstructionSourceSite {
    skiff_artifact_model::InstructionSourceSite::Synthetic {
        reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

pub(crate) fn local_execution_catch_identity(type_index: usize) -> CatchIdentity {
    local_execution_catch_identity_for_addr(service_type_addr(type_index))
}

pub(crate) fn local_execution_catch_identity_for_addr(addr: TypeAddr) -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr,
            type_arguments: Vec::new(),
        },
    ))
}

pub(crate) fn linked_builtin_type(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

pub(crate) fn receiver_builtin_target(root: &str, method: &str) -> serde_json::Value {
    let op = builtin_receiver_op_by_name(root, method).expect("receiver op must exist");
    json!({
        "kind": "receiverBuiltin",
        "op": serde_json::to_value(op).unwrap()
    })
}

pub(crate) fn local_const_receiver_target(executable_index: usize) -> serde_json::Value {
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

pub(crate) fn runtime_scalar_json(value: &RuntimeValue) -> Option<Value> {
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

pub(crate) struct ProgramTestInvocation {
    pub(crate) request: RequestEnvelope,
    pub(crate) operation: RuntimeOperation,
    pub(crate) route_addr: ExecutableAddr,
    pub(crate) receiver_const: Option<ConstAddr>,
    pub(crate) runtime_id: String,
    pub(crate) service_id: String,
    pub(crate) cancellation: CancellationToken,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) service_http_response_max_bytes: usize,
    pub(crate) config: RuntimeConfigView,
    pub(crate) package_configs: Vec<RuntimeConfigView>,
    pub(crate) service_db: Option<skiff_runtime_service_db::ServiceDbCapabilityFactory>,
    pub(crate) file_runtime: Arc<crate::host::file_runtime::FileRuntime>,
    pub(crate) db_request_state: Arc<tokio::sync::Mutex<skiff_runtime_service_db::DbRequestState>>,
    pub(crate) execution_budget: Arc<crate::execution_budget::ExecutionBudget>,
    pub(crate) request_heap_limits: RequestHeapLimits,
    pub(crate) router_sender:
        Option<tokio::sync::mpsc::UnboundedSender<crate::host::RouterWriterMessage>>,
    pub(crate) outbound_requests: Arc<crate::host::OutboundRequestRegistry>,
    pub(crate) actor_factory: eval_capability_adapter::TestActorCapabilityFactory,
    pub(crate) telemetry: Option<crate::telemetry::RequestTelemetryContext>,
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
        self.telemetry.clone()
    }
}

pub(crate) fn test_invocation(target: &str) -> ProgramTestInvocation {
    let operation_abi_id = format!("operation:{target}");
    let cancellation = CancellationToken::new();
    let cancelled = cancellation.cancel_flag();
    let mut request_extra = serde_json::Map::new();
    request_extra.insert(
        "trace".to_string(),
        json!({
            "traceId": "trace-program"
        }),
    );
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
            ingress_selector: None,
            http_adapter: None,
            binary_http: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: request_extra,
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
        actor_factory: eval_capability_adapter::TestActorCapabilityFactory::default(),
        telemetry: None,
    }
}

pub(crate) fn concrete_execution_control(
    frame: &ProgramTestInvocation,
) -> crate::request::ExecutionControl<'_> {
    crate::request::ExecutionControl::new(frame.cancellation.clone(), &frame.execution_budget)
}

pub(crate) fn test_execution_control(
    frame: &ProgramTestInvocation,
) -> crate::eval::capabilities::ExecutionControl<'_> {
    eval_capability_adapter::execution_control(concrete_execution_control(frame))
}

pub(crate) fn test_db_context(
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

pub(crate) async fn execute_test_program_route(
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

pub(crate) fn program_invocation_context<'a>(
    interpreter: &Interpreter,
    frame: &'a ProgramTestInvocation,
) -> ProgramInvocationContext<'a> {
    let execution = frame.execution_control();
    let (actor, request) = frame.actor_factory.actor_from_request(
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
        actor: actor.clone(),
        request,
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

pub(crate) fn set_request_string_arg(frame: &mut ProgramTestInvocation, name: &str, value: &str) {
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

pub(crate) fn set_request_http_arg(frame: &mut ProgramTestInvocation, name: &str) {
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

pub(crate) fn std_http_request_descriptor_for_payload() -> Value {
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

pub(crate) fn std_http_header_descriptor_for_payload() -> Value {
    json!({
        "kind": "record",
        "fields": {
            "name": { "kind": "builtin", "name": "string", "args": [] },
            "value": { "kind": "builtin", "name": "string", "args": [] }
        }
    })
}

pub(crate) fn http_request_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
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

pub(crate) fn http_client_request_runtime_value(heap: &mut RequestHeap) -> RuntimeValue {
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

pub(crate) fn db_metadata(mut value: Value) -> Vec<DbMetadataIr> {
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

pub(crate) fn thread_db_metadata() -> Vec<DbProviderTargetMetadata> {
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
    .into_iter()
    .enumerate()
    .map(|(index, metadata)| {
        let type_name = metadata.type_name.clone();
        let target_id = thread_db_object_target_id(index);
        DbProviderTargetMetadata {
            target: DbCapabilityTarget::new(
                DbCapabilityTargetId {
                    package_artifact_ref: target_id.package_artifact_ref,
                    file_ir_ref: target_id.file_ir_ref,
                    type_index: target_id.type_index,
                },
                type_name,
            ),
            metadata,
        }
    })
    .collect()
}

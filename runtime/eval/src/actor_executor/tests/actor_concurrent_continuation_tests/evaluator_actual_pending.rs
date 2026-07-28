use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
    task::{Wake, Waker},
};

use bytes::Bytes;
use serde_json::{json, Value};
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, InstructionSourceSite, InterfaceInstantiationRef, LiteralIr,
    OperationAbiRef, PublicationAbiUnit, PublicationOperationAbi, PublicationOperationKind,
    PublicationPublicInstanceExport, SourceCallMethodIndexEntry, SyntheticInstructionSiteReason,
    TypeRefIr,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    file::{immutable_file_wire, FileCreateOptions, ImmutableFileRef},
    json::RuntimeBoundaryCodec,
    payload::{PayloadBoundary, PayloadBoundaryKind},
    plan::BoundaryUse,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorInvocationOutcome, ActorInvocationRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken, CapabilityError,
    CapabilityFuture, CapabilityResult, DbCapabilityContext, FileCapabilityApi,
    FileCapabilityContext, FileCapabilityFuture, FileCapabilitySource, FileCapabilitySourceApi,
    FileChunkSource, FileSourceStreamApi, FileSourceStreamContext, OutboundRequestCancelSendError,
    OutboundRequestCancelSender, OutboundRequestRegistry, OutboundResponse,
    OutboundResponseReceiver, OutboundStartedRequest, OwnedActorCapabilityContext,
    RequestEffectDoubleControl, SpawnSubmitControlRequest, StreamCancelSignal, StreamInternalItem,
    StreamLifetimeGuard, StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi,
    StreamRuntimeError, StreamRuntimeResult, StreamSink, StreamSinkApi,
};
use skiff_runtime_linked_program::{
    anonymous_type_decl, CallIr, DbQueryIr, DbTargetIr, LinkedCallTarget, LinkedExecutableBody,
    LinkedInterfaceInstantiationRef, LinkedRemoteOperationSlotPlanIr,
    LinkedRemoteOperationTablePlanIr, LinkedTypeDescriptor, NativeTarget, PackageSymbolKey,
    PackageUnit, ServiceDependencyConstraint, ServiceDependencySymbolRef, ServiceMeta, SlotIr,
    SlotLayoutIr, TypeAddr,
};
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    runtime_value::ActorRef,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::*;
use crate::{
    capabilities::{EvalCapabilityFuture, OutboundServiceApi, OutboundServiceContext},
    env::Env,
    eval_context::EvalContext,
    program_execution::ProgramExecutionInput,
};

struct EvaluatorFixture {
    actor: Fixture,
    interpreter: Interpreter,
    file: Arc<LinkedFileUnit>,
}

impl EvaluatorFixture {
    fn new(
        expressions: Vec<LinkedExprIr>,
        statements: Vec<LinkedStmtIr>,
        slots: SlotLayoutIr,
    ) -> Self {
        let actor = fixture(integer(), true);
        let mut file = (*actor_file(integer(), true)).clone();
        let executable = &mut file.executables[0];
        executable.return_type = None;
        executable.slots = slots;
        executable.body = LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: (0..statements.len())
                    .map(|statement| StmtRefIr {
                        statement: statement as u32,
                    })
                    .collect(),
            }],
            statements,
            expressions,
        };
        let file = Arc::new(file);
        let interpreter = interpreter_with_std_types(Arc::clone(&file));
        Self {
            actor,
            interpreter,
            file,
        }
    }

    fn executable(&self) -> &LinkedExecutable {
        &self.file.executables[0]
    }

    async fn actor_frame(&self) -> (ActorExecutionFrame, RequestHeap) {
        execution_frame(&self.actor).await
    }

    fn eval_context<'a>(
        &'a self,
        frame: ActorExecutionFrame,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context(&self.interpreter).with_actor_execution_frame(frame),
            heap,
            env,
            addr,
            &self.file,
            self.executable(),
        )
        .expect("evaluator context")
    }

    fn eval_context_with<'a>(
        &'a self,
        context: ProgramExecutionContext<'static>,
        frame: ActorExecutionFrame,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
    ) -> EvalContext<'a> {
        EvalContext::new(
            &self.interpreter,
            context.with_actor_execution_frame(frame),
            heap,
            env,
            addr,
            &self.file,
            self.executable(),
        )
        .expect("evaluator context")
    }
}

fn program_context_with(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    outbound: OutboundServiceContext,
    file: FileCapabilityContext,
    db: DbCapabilityContext,
) -> ProgramExecutionContext<'static> {
    program_context_with_stream(
        interpreter,
        actor,
        outbound,
        file,
        db,
        interpreter.stream_runtime.clone(),
    )
}

fn program_context_with_stream(
    interpreter: &Interpreter,
    actor: ActorCapabilityContext<'static>,
    outbound: OutboundServiceContext,
    file: FileCapabilityContext,
    db: DbCapabilityContext,
    stream_runtime: StreamRuntime,
) -> ProgramExecutionContext<'static> {
    let execution = test_runtime::execution_control();
    let effects = test_runtime::effects_context();
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db,
        file,
        file_source_stream: FileSourceStreamContext::from_api(RuntimeFileSourceStream {
            stream_runtime: stream_runtime.clone(),
        }),
        time: TimeCapabilityContext::new(execution),
        websocket: test_runtime::websocket_context(),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            stream_runtime,
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: "skiff.run/counter".to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: "1.0.0".to_string(),
            package_configs: Vec::new(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            db: Vec::new(),
            actors: Vec::new(),
            gateway: Default::default(),
        }),
        actor: actor.clone(),
        spawn: actor,
        outbound,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

#[derive(Clone)]
struct RuntimeFileSourceStream {
    stream_runtime: StreamRuntime,
}

impl FileSourceStreamApi for RuntimeFileSourceStream {
    fn stream_runtime_handle(&self) -> StreamRuntime {
        self.stream_runtime.clone()
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            match self.stream_runtime.next(stream).await? {
                StreamPoll::Item(item) => Ok(Some(item)),
                StreamPoll::End => Ok(None),
                StreamPoll::InternalItem(item) => {
                    let (value, heap) = item.into_parts();
                    let value = match value {
                        RuntimeValue::Heap(handle) => heap
                            .local_carrier_cell(handle)
                            .map_err(|error| {
                                skiff_runtime_capability_context::FileCapabilityError::decode(
                                    error.to_string(),
                                )
                            })?
                            .map(|carrier| carrier.into_value())
                            .unwrap_or(RuntimeValue::Heap(handle)),
                        value => value,
                    };
                    crate::runtime_ops::runtime_to_wire(&value, &heap)
                        .map(Some)
                        .map_err(|error| {
                            skiff_runtime_capability_context::FileCapabilityError::decode(
                                error.to_string(),
                            )
                        })
                }
            }
        })
    }
}

fn default_program_context(interpreter: &Interpreter) -> ProgramExecutionContext<'static> {
    program_context_with(
        interpreter,
        test_runtime::actor_context(),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    )
}

fn interpreter_with_std_types(file: Arc<LinkedFileUnit>) -> Interpreter {
    let duration = anonymous_type_decl(
        "std.time.Duration",
        LinkedTypeDescriptor::Alias { target: integer() },
    );
    let nullable_string = LinkedTypeRef::Nullable {
        inner: Box::new(string_type()),
    };
    let immutable_file = anonymous_type_decl(
        "std.file.ImmutableFile",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("contentType".to_string(), nullable_string.clone()),
                ("id".to_string(), string_type()),
                ("sha256".to_string(), string_type()),
                ("size".to_string(), integer()),
            ]),
        },
    );
    let create_options = anonymous_type_decl(
        "std.file.CreateOptions",
        LinkedTypeDescriptor::Record {
            fields: BTreeMap::from([
                ("contentType".to_string(), nullable_string.clone()),
                ("purpose".to_string(), nullable_string),
            ]),
        },
    );
    let declarations = [
        ("std.time.Duration", duration),
        ("std.file.ImmutableFile", immutable_file),
        ("std.file.CreateOptions", create_options),
    ];
    let std_file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-std".to_string(),
        source_ast_hash: "source:f445h-e4r-std".to_string(),
        module_path: "std".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: declarations
            .iter()
            .map(|(_, declaration)| declaration.clone())
            .collect(),
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: ExternalRefTable::default(),
    });
    let mut overlay = LinkOverlay::default();
    overlay
        .package_slots_by_id
        .insert("skiff.run/std".to_string(), 0);
    overlay
        .package_slots_by_dependency_ref
        .insert("std".to_string(), 0);
    let mut types = RuntimeTypeContext::default();
    for (type_index, (symbol, declaration)) in declarations.into_iter().enumerate() {
        let addr = TypeAddr {
            unit: UnitAddr::Package(0),
            file: FileAddr::LoadedFileIndex(0),
            type_index,
        };
        types.descriptors.insert(addr.clone(), declaration);
        types
            .exported_types
            .insert_package(PackageSymbolKey::new(0, symbol), addr);
    }
    let program = Arc::new(EvalRuntimeProgram::new(
        "skiff.run/counter",
        vec![file],
        vec![Arc::new(PackageUnit::empty(
            "skiff.run/std",
            "1.0.0",
            "skiff.run/std:build:f445h-e4r",
            "skiff.run/std:abi:f445h-e4r",
        ))],
        vec![vec![std_file]],
        PublicationResourceTable::default(),
        vec![PublicationResourceTable::default()],
        Default::default(),
        overlay,
        types,
    ));
    Interpreter::with_program(program, test_runtime::runtime_factory())
}

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn call(target: LinkedCallTarget, args: Vec<u32>) -> CallIr {
    CallIr {
        target,
        site: site(),
        args: args
            .into_iter()
            .map(|expression| ExprRefIr { expression })
            .collect(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

fn native_target(namespace: &str, symbol: &str, binding_key: &str) -> LinkedCallTarget {
    LinkedCallTarget::Native {
        target: NativeTarget {
            namespace: namespace.to_string(),
            symbol: symbol.to_string(),
            binding_key: Some(binding_key.to_string()),
            metadata: BTreeMap::new(),
        },
    }
}

fn native_executable(target: LinkedCallTarget, args: Vec<LiteralIr>) -> EvaluatorFixture {
    let call_index = args.len() as u32;
    let mut expressions = args
        .into_iter()
        .map(|value| LinkedExprIr::Literal { value })
        .collect::<Vec<_>>();
    expressions.push(LinkedExprIr::Call {
        call: call(target, (0..call_index).collect()),
    });
    EvaluatorFixture::new(
        expressions,
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr {
                    expression: call_index,
                },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

const OUTBOUND_SERVICE_ID: &str = "skiff.test/evaluator-provider";
const OUTBOUND_PROTOCOL: &str = "protocol:f445h-e4r";
const OUTBOUND_BUILD: &str = "build:f445h-e4r";
const OUTBOUND_ALIAS: &str = "provider";
const LEGACY_OPERATION_ABI: &str = "operation:f445h-e4r:legacy";
const REMOTE_OPERATION_ABI: &str = "operation:f445h-e4r:remote";
const REMOTE_INTERFACE_ABI: &str = "interface:f445h-e4r:remote";
const REMOTE_METHOD_ABI: &str = "method:f445h-e4r:remote";

#[derive(Clone)]
struct RecordingOutbound {
    state: Arc<RecordingOutboundState>,
}

struct RecordingOutboundState {
    dependencies: Vec<ServiceDependencyConstraint>,
    registry: OutboundRequestRegistry,
    starts: AtomicUsize,
    response_sender: Mutex<Option<tokio::sync::mpsc::UnboundedSender<OutboundResponse>>>,
    buffered: Mutex<Option<OutboundResponse>>,
}

#[derive(Debug)]
struct PullSetupRuntime;

impl StreamRuntimeApi for PullSetupRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!("serverStream setup only creates a pull source")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!("serverStream setup only creates a pull source")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        skiff_runtime_boundary::stream::stream_value("f445h-e4r-server-stream")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!("serverStream setup does not create buffered streams")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("serverStream setup does not poll its returned stream")
    }

    fn cancel(&self, _value: &Value) {}
}

fn pull_setup_runtime() -> StreamRuntime {
    StreamRuntime::new(PullSetupRuntime)
}

impl RecordingOutbound {
    fn pending(dependencies: Vec<ServiceDependencyConstraint>) -> Self {
        Self {
            state: Arc::new(RecordingOutboundState {
                dependencies,
                registry: OutboundRequestRegistry::default(),
                starts: AtomicUsize::new(0),
                response_sender: Mutex::new(None),
                buffered: Mutex::new(None),
            }),
        }
    }

    fn buffered(
        dependencies: Vec<ServiceDependencyConstraint>,
        response: OutboundResponse,
    ) -> Self {
        let this = Self::pending(dependencies);
        *this.state.buffered.lock().expect("outbound buffered lock") = Some(response);
        this
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn send(&self, response: OutboundResponse) {
        self.state
            .response_sender
            .lock()
            .expect("outbound response sender lock")
            .as_ref()
            .expect("outbound request must have started")
            .send(response)
            .expect("outbound response receiver must remain live");
    }
}

impl OutboundServiceApi for RecordingOutbound {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        &self.state.dependencies
    }

    fn test_effects_enabled(&self) -> bool {
        false
    }

    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        HashMap::new()
    }

    fn request_heap(&self) -> RequestHeap {
        RequestHeap::default()
    }

    fn effective_timeout_ms(&self, _operation_timeout_ms: Option<u64>) -> Option<u64> {
        None
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        RuntimeError::Cancelled
    }

    fn start_request(
        &self,
        _start: crate::capabilities::OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> crate::error::Result<OutboundStartedRequest> {
        let ordinal = self.state.starts.fetch_add(1, Ordering::AcqRel) + 1;
        let request_id = format!("f445h-e4r-outbound-{ordinal}");
        let (sender, response_rx) = tokio::sync::mpsc::unbounded_channel();
        let cancel_sender: OutboundRequestCancelSender =
            Arc::new(|_, _| Ok::<(), OutboundRequestCancelSendError>(()));
        let lease = self
            .state
            .registry
            .insert_with_lease(
                request_id.clone(),
                sender.clone(),
                Some(cancel_sender),
                "f445h_e4r_wait_dropped",
            )
            .expect("outbound request lease");
        *self
            .state
            .response_sender
            .lock()
            .expect("outbound response sender lock") = Some(sender.clone());
        if let Some(response) = self
            .state
            .buffered
            .lock()
            .expect("outbound buffered lock")
            .take()
        {
            sender.send(response).expect("buffered outbound response");
        }
        Ok(OutboundStartedRequest {
            request_id,
            response_rx,
            lease,
        })
    }

    fn receive_response<'a>(
        &'a self,
        _lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        target: &'a str,
        receiver: &'a mut OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> EvalCapabilityFuture<'a, OutboundResponse> {
        Box::pin(async move {
            receiver
                .recv()
                .await
                .ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "response channel closed".to_string(),
                })
        })
    }

    fn cancel_signal(&self) -> CancellationToken {
        CancellationToken::new()
    }
}

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new())
}

fn encoded_string_response(value: &str) -> OutboundResponse {
    let plan = string_plan();
    let mut heap = RequestHeap::default();
    let value = RuntimeBoundaryCodec::new(
        &plan,
        BoundaryUse::NativeReturn,
        "f445h-e4r outbound response",
    )
    .from_wire_json(&json!(value), &mut heap)
    .expect("outbound response should materialize");
    let payload = encode_payload_plan(
        &value,
        &plan,
        &PayloadBoundary::cross_service(
            PayloadBoundaryKind::InboundServiceCall,
            OUTBOUND_SERVICE_ID,
        ),
        &mut heap,
    )
    .expect("outbound response should encode");
    OutboundResponse::End { payload }
}

fn operation_ref(remote: bool) -> OperationAbiRef {
    if remote {
        OperationAbiRef {
            operation_abi_id: REMOTE_OPERATION_ABI.to_string(),
            kind: PublicationOperationKind::PublicInstanceMethod,
            public_path: "reader.read".to_string(),
            public_instance_key: Some("reader".to_string()),
            interface: Some(InterfaceInstantiationRef {
                interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
                canonical_type_args: Vec::new(),
            }),
            method_abi_id: Some(REMOTE_METHOD_ABI.to_string()),
            display_name: "reader.read".to_string(),
        }
    } else {
        OperationAbiRef {
            operation_abi_id: LEGACY_OPERATION_ABI.to_string(),
            kind: PublicationOperationKind::PublicFunction,
            public_path: "read".to_string(),
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            display_name: "read".to_string(),
        }
    }
}

fn outbound_dependency(mode: &str, remote: bool) -> ServiceDependencyConstraint {
    let operation = operation_ref(remote);
    let return_type = if mode == "serverStream" {
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        }
    } else {
        TypeRefIr::builtin("string")
    };
    let operation_abi = PublicationOperationAbi {
        operation: operation.clone(),
        public_signature: CanonicalPublicCallableSignature {
            params: Vec::new(),
            return_type,
            may_suspend: true,
        },
        schema_closure: Vec::new(),
        stream_effect_throw_config: BTreeMap::new(),
    };
    let mut publication = PublicationAbiUnit::empty(OUTBOUND_SERVICE_ID, "1.0.0", "abi:f445h-e4r");
    publication.operation_exports.push(operation.clone());
    publication.operation_abi.push(operation_abi);
    if remote {
        publication
            .public_instances
            .push(PublicationPublicInstanceExport {
                public_instance_key: "reader".to_string(),
                interfaces: vec![InterfaceInstantiationRef {
                    interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
                    canonical_type_args: Vec::new(),
                }],
                source_call_method_index: vec![SourceCallMethodIndexEntry {
                    method_name: "read".to_string(),
                    operation: operation.clone(),
                }],
                method_operations: vec![operation],
            });
    }
    ServiceDependencyConstraint {
        id: OUTBOUND_SERVICE_ID.to_string(),
        version: "1.0.0".to_string(),
        alias: OUTBOUND_ALIAS.to_string(),
        build_id: OUTBOUND_BUILD.to_string(),
        service_protocol_identity: OUTBOUND_PROTOCOL.to_string(),
        publication_abi: publication,
    }
}

fn legacy_outbound_fixture(_mode: &str) -> EvaluatorFixture {
    let symbol = ServiceDependencySymbolRef {
        dependency_ref: OUTBOUND_ALIAS.to_string(),
        operation: operation_ref(false),
    };
    EvaluatorFixture::new(
        vec![LinkedExprIr::Call {
            call: call(
                LinkedCallTarget::ServiceDependencySymbol { symbol },
                Vec::new(),
            ),
        }],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

fn remote_outbound_fixture() -> EvaluatorFixture {
    let interface = LinkedInterfaceInstantiationRef {
        interface_abi_id: REMOTE_INTERFACE_ABI.to_string(),
        canonical_type_args: Vec::new(),
    };
    let operations = LinkedRemoteOperationTablePlanIr {
        interface: interface.clone(),
        slots: vec![LinkedRemoteOperationSlotPlanIr {
            slot: 0,
            method_abi_id: REMOTE_METHOD_ABI.to_string(),
            signature: skiff_runtime_linked_program::LinkedInterfaceMethodSlotSignatureIr {
                params: Vec::new(),
                return_type: string_type(),
            },
            operation_abi_id: REMOTE_OPERATION_ABI.to_string(),
        }],
    };
    EvaluatorFixture::new(
        vec![
            LinkedExprIr::Literal {
                value: LiteralIr::Null,
            },
            LinkedExprIr::InterfaceBox {
                value: ExprRefIr { expression: 0 },
                interface: interface.clone(),
                source: skiff_runtime_linked_program::LinkedBoxSourceIr::Remote {
                    dependency_ref: OUTBOUND_ALIAS.to_string(),
                    public_instance_key: "reader".to_string(),
                    operations,
                    callee_protocol_identity: OUTBOUND_PROTOCOL.to_string(),
                },
            },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::InterfaceMethod {
                        interface,
                        method_abi_id: REMOTE_METHOD_ABI.to_string(),
                        slot: 0,
                    },
                    vec![1],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

enum ActorReply {
    Ready(CapabilityResult<ActorInvocationOutcome>),
    Pending(oneshot::Receiver<CapabilityResult<ActorInvocationOutcome>>),
}

#[derive(Clone)]
struct RecordingActor {
    state: Arc<RecordingActorState>,
}

struct RecordingActorState {
    reply: Mutex<Option<ActorReply>>,
    starts: AtomicUsize,
    drops_before_completion: AtomicUsize,
}

impl RecordingActor {
    fn ready(outcome: CapabilityResult<ActorInvocationOutcome>) -> Self {
        Self {
            state: Arc::new(RecordingActorState {
                reply: Mutex::new(Some(ActorReply::Ready(outcome))),
                starts: AtomicUsize::new(0),
                drops_before_completion: AtomicUsize::new(0),
            }),
        }
    }

    fn pending() -> (
        Self,
        oneshot::Sender<CapabilityResult<ActorInvocationOutcome>>,
    ) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                state: Arc::new(RecordingActorState {
                    reply: Mutex::new(Some(ActorReply::Pending(receiver))),
                    starts: AtomicUsize::new(0),
                    drops_before_completion: AtomicUsize::new(0),
                }),
            },
            sender,
        )
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }
}

struct PendingActorWait {
    state: Arc<RecordingActorState>,
    completed: bool,
}

impl Drop for PendingActorWait {
    fn drop(&mut self) {
        if !self.completed {
            self.state
                .drops_before_completion
                .fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl ActorCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "runtime:f445h-e4r"
    }

    fn service_id(&self) -> &str {
        "skiff.run/counter"
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:f445h-e4r"
    }

    fn request_target(&self) -> &str {
        "actor.f445h-e4r"
    }

    fn request_build_id(&self) -> &str {
        "build:f445h-e4r"
    }

    fn spawn_service_protocol_identity(&self) -> &str {
        "spawn-protocol:f445h-e4r"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "request-protocol:f445h-e4r"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        None
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }

    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn submit_spawn<'a>(
        &'a self,
        _request: SpawnSubmitControlRequest,
        _args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome> {
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        let reply = self
            .state
            .reply
            .lock()
            .expect("Actor reply lock")
            .take()
            .expect("Actor invocation starts once");
        match reply {
            ActorReply::Ready(outcome) => Box::pin(async move { outcome }),
            ActorReply::Pending(receiver) => {
                let state = Arc::clone(&self.state);
                Box::pin(async move {
                    let mut guard = PendingActorWait {
                        state,
                        completed: false,
                    };
                    let outcome = receiver.await.map_err(|_| {
                        CapabilityError::provider_unavailable(
                            "actor.f445h-e4r",
                            "reply channel closed",
                        )
                    })?;
                    guard.completed = true;
                    outcome
                })
            }
        }
    }
}

fn actor_dispatch_fixture() -> EvaluatorFixture {
    EvaluatorFixture::new(
        vec![
            LinkedExprIr::LoadSlot { slot: 0 },
            LinkedExprIr::Literal {
                value: LiteralIr::Number {
                    value: serde_json::Number::from(4),
                },
            },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActorDispatch {
                        plan: skiff_runtime_linked_program::LinkedActorMethodDispatchPlan {
                            declaration_owner: owner(),
                            actor_abi_identity: abi(),
                            actor_implementation_identity: implementation(),
                            method_identity: method_identity(),
                        },
                    },
                    vec![0, 1],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "receiver".to_string(),
                kind: "parameter".to_string(),
            }],
            frame_size: 1,
        },
    )
}

fn actor_dispatch_env(fixture: &EvaluatorFixture) -> Env {
    let mut env = Env::for_program_executable(
        fixture.executable(),
        Some(fixture.file.module_path.clone()),
        0,
    )
    .expect("Actor dispatch env");
    env.declare_binding(
        "receiver",
        Some(0),
        RuntimeValue::ActorRef(ActorRef::new(
            "skiff.run/counter",
            "actors.Counter",
            "builtin:string",
            ACTOR_BOOTSTRAP_ENCODING_V1,
            br#""counter-remote""#.to_vec(),
            "sha256:f445h-e4r-counter",
            Some(7),
        )),
    )
    .expect("Actor receiver binding");
    env
}

fn actor_return(value: i64) -> ActorInvocationOutcome {
    ActorInvocationOutcome::Returned(
        canonical_json_bytes(&json!(value)).expect("Actor return payload"),
    )
}

#[derive(Clone)]
struct RecordingFile {
    state: Arc<RecordingFileState>,
}

struct RecordingFileState {
    starts: AtomicUsize,
    completions: AtomicUsize,
    drops_before_completion: AtomicUsize,
}

impl RecordingFile {
    fn new() -> Self {
        Self {
            state: Arc::new(RecordingFileState {
                starts: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                drops_before_completion: AtomicUsize::new(0),
            }),
        }
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn completions(&self) -> usize {
        self.state.completions.load(Ordering::Acquire)
    }

    fn drops_before_completion(&self) -> usize {
        self.state.drops_before_completion.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct RecordingFileSource {
    file: RecordingFile,
}

impl FileCapabilitySourceApi for RecordingFileSource {
    fn context_for_request(&self, _db_context: DbCapabilityContext) -> FileCapabilityContext {
        FileCapabilityContext::new(self.file.clone())
    }
}

struct PendingFileWait {
    state: Arc<RecordingFileState>,
    completed: bool,
}

impl Drop for PendingFileWait {
    fn drop(&mut self) {
        if !self.completed {
            self.state
                .drops_before_completion
                .fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl FileCapabilityApi for RecordingFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(RecordingFileSource { file: self.clone() })
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, ()> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "not used",
            ))
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        mut next_chunk: FileChunkSource<'a>,
    ) -> FileCapabilityFuture<'a, Value> {
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let mut guard = PendingFileWait {
                state: Arc::clone(&state),
                completed: false,
            };
            let mut size = 0usize;
            while let Some(chunk) = next_chunk().await? {
                size += chunk.len();
            }
            state.completions.fetch_add(1, Ordering::AcqRel);
            guard.completed = true;
            Ok(immutable_file_wire(ImmutableFileRef {
                id: "file:f445h-e4r".to_string(),
                size: size as i64,
                sha256: "sha256:f445h-e4r".to_string(),
                content_type: None,
            }))
        })
    }
}

fn bytes_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "bytes".to_string(),
        args: Vec::new(),
    }
}

fn create_from_stream_fixture() -> EvaluatorFixture {
    let actor = fixture(integer(), true);
    let mut file = (*actor_file(integer(), true)).clone();
    let producer_addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(FILE_ID.to_string()),
        executable: 1,
    };
    file.executables[0].return_type = None;
    file.executables[0].slots = SlotLayoutIr::default();
    file.executables[0].body = LinkedExecutableBody {
        blocks: vec![BlockIr {
            label: "entry".to_string(),
            statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
        }],
        statements: vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 2 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        expressions: vec![
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::Executable {
                        addr: producer_addr,
                    },
                    Vec::new(),
                ),
            },
            LinkedExprIr::Literal {
                value: LiteralIr::Null,
            },
            LinkedExprIr::Call {
                call: call(
                    native_target("std.file", "createFromStream", "std.file.createFromStream"),
                    vec![0, 1],
                ),
            },
        ],
    };
    file.executables.push(LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "bytesProducer".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![bytes_type()],
        }),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }, StmtRefIr { statement: 1 }],
            }],
            statements: vec![
                LinkedStmtIr::Emit {
                    operation: "emit".to_string(),
                    value: ExprRefIr { expression: 1 },
                },
                LinkedStmtIr::Return { value: None },
            ],
            expressions: vec![
                LinkedExprIr::Literal {
                    value: LiteralIr::String {
                        value: "6869".to_string(),
                    },
                },
                LinkedExprIr::Call {
                    call: call(
                        native_target("std.bytes", "fromHex", "core.bytes.fromHex"),
                        vec![0],
                    ),
                },
            ],
        },
    });
    let file = Arc::new(file);
    let interpreter = interpreter_with_std_types(Arc::clone(&file));
    EvaluatorFixture {
        actor,
        interpreter,
        file,
    }
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn first_poll<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(NoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

#[tokio::test]
async fn f445h_e4r_spine_native_ready_first_poll_keeps_actor_segment() {
    let fixture = native_executable(
        native_target("std.time", "sleep", "std.time.sleep"),
        vec![LiteralIr::Number {
            value: serde_json::Number::from(0),
        }],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let flow = fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("zero sleep");

    assert!(matches!(flow, crate::env::Flow::Return(_)));
    assert!(
        frame.has_execution_lease(),
        "first-Ready native wait must keep the current segment"
    );
    frame.finish(heap).expect("finish ready native frame");
}

#[tokio::test]
async fn f445h_e4r_spine_native_pending_releases_and_reacquires_actor_segment() {
    let fixture = native_executable(
        native_target("std.time", "sleep", "std.time.sleep"),
        vec![LiteralIr::Number {
            value: serde_json::Number::from(50),
        }],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(
        !frame.has_execution_lease(),
        "first-Pending native wait must release the current segment"
    );
    tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("sleep completes")
        .expect("pending native call succeeds");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "native completion must reacquire before finalize"
    );
    frame.finish(heap).expect("finish pending native frame");
}

#[tokio::test]
async fn f445h_e4r_spine_websocket_send_sync_error_keeps_actor_segment() {
    let fixture = native_executable(
        native_target(
            "std.websocket",
            "sendTextToConnection",
            "std.websocket.sendTextToConnection",
        ),
        vec![
            LiteralIr::String {
                value: "connection".to_string(),
            },
            LiteralIr::String {
                value: "hello".to_string(),
            },
        ],
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect_err("test websocket capability is deliberately unavailable");

    assert!(
        frame.has_execution_lease(),
        "synchronous WebSocket send must not cut the Actor segment"
    );
    frame.finish(heap).expect("finish websocket frame");
}

#[tokio::test]
async fn f445h_e4r_spine_remote_interface_ready_keeps_actor_segment() {
    let dependency = outbound_dependency("unary", true);
    let outbound = RecordingOutbound::buffered(vec![dependency], encoded_string_response("ready"));
    let fixture = remote_outbound_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );

    fixture
        .eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("buffered remote interface response");

    assert_eq!(outbound.starts(), 1);
    assert!(
        frame.has_execution_lease(),
        "first-Ready remote wait must keep the Actor segment"
    );
    frame.finish(heap).expect("finish remote Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_remote_interface_pending_reacquires_before_finalize() {
    let dependency = outbound_dependency("unary", true);
    let outbound = RecordingOutbound::pending(vec![dependency]);
    let fixture = remote_outbound_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(outbound.starts(), 1);
    assert!(!frame.has_execution_lease());
    outbound.send(encoded_string_response("pending"));
    execution.await.expect("pending remote interface response");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "remote completion must reacquire before response decode/finalize"
    );
    frame.finish(heap).expect("finish remote Pending frame");
}

#[tokio::test]
async fn f445h_e4r_spine_legacy_unary_pending_and_server_stream_ready() {
    let unary_dependency = outbound_dependency("unary", false);
    let unary_outbound = RecordingOutbound::pending(vec![unary_dependency]);
    let unary = legacy_outbound_fixture("unary");
    let (unary_frame, mut unary_heap) = unary.actor_frame().await;
    let mut unary_env = Env::new();
    let addr = executable_addr();
    let unary_context = program_context_with(
        &unary.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(unary_outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut unary_eval = unary.eval_context_with(
        unary_context,
        unary_frame.clone(),
        &mut unary_heap,
        &mut unary_env,
        &addr,
    );
    let mut unary_execution = Box::pin(unary_eval.exec_program_executable());

    assert!(matches!(
        first_poll(unary_execution.as_mut()),
        Poll::Pending
    ));
    assert!(!unary_frame.has_execution_lease());
    unary_outbound.send(encoded_string_response("legacy-pending"));
    unary_execution.await.expect("legacy unary response");
    drop(unary_eval);
    assert!(unary_frame.has_execution_lease());
    unary_frame
        .finish(unary_heap)
        .expect("finish legacy unary frame");

    let stream_dependency = outbound_dependency("serverStream", false);
    let stream_outbound = RecordingOutbound::pending(vec![stream_dependency]);
    let stream = legacy_outbound_fixture("serverStream");
    let (stream_frame, mut stream_heap) = stream.actor_frame().await;
    let mut stream_env = Env::new();
    let stream_context = program_context_with_stream(
        &stream.interpreter,
        test_runtime::actor_context(),
        OutboundServiceContext::new(stream_outbound.clone()),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
        pull_setup_runtime(),
    );

    stream
        .eval_context_with(
            stream_context,
            stream_frame.clone(),
            &mut stream_heap,
            &mut stream_env,
            &addr,
        )
        .exec_program_executable()
        .await
        .expect("serverStream setup is synchronous");

    assert_eq!(stream_outbound.starts(), 1);
    assert!(
        stream_frame.has_execution_lease(),
        "serverStream setup must not cut the Actor segment"
    );
    stream_frame
        .finish(stream_heap)
        .expect("finish serverStream frame");
}

#[tokio::test]
async fn f445h_e4r_spine_actor_dispatch_ready_keeps_actor_segment() {
    let actor = RecordingActor::ready(Ok(actor_return(11)));
    let fixture = actor_dispatch_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = actor_dispatch_env(&fixture);
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        ActorCapabilityContext::new(actor.clone()),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );

    fixture
        .eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("first-Ready Actor dispatch");

    assert_eq!(actor.starts(), 1);
    assert!(
        frame.has_execution_lease(),
        "first-Ready Actor dispatch must keep the current segment"
    );
    frame.finish(heap).expect("finish Actor Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_actor_dispatch_pending_reacquires_before_finalize() {
    let (actor, release) = RecordingActor::pending();
    let fixture = actor_dispatch_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = actor_dispatch_env(&fixture);
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        ActorCapabilityContext::new(actor.clone()),
        test_runtime::outbound_context(),
        test_runtime::file_context(),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(actor.starts(), 1);
    assert!(!frame.has_execution_lease());
    release
        .send(Ok(actor_return(12)))
        .expect("release pending Actor invocation");
    execution.await.expect("pending Actor dispatch");
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "Actor completion must reacquire before return decode/finalize"
    );
    frame.finish(heap).expect("finish Actor Pending frame");
}

#[tokio::test]
async fn f445h_e4r_spine_db_query_is_first_poll_ready_and_keeps_actor_segment() {
    let fixture = EvaluatorFixture::new(
        vec![LinkedExprIr::DbQuery {
            target: DbTargetIr {
                type_ref: string_type(),
                type_name: "Thread".to_string(),
            },
            query: DbQueryIr::default(),
            projection: None,
            result_type: None,
        }],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    );
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let mut eval = fixture.eval_context_with(
        default_program_context(&fixture.interpreter),
        frame.clone(),
        &mut heap,
        &mut env,
        &addr,
    );
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(
        first_poll(execution.as_mut()),
        Poll::Ready(Ok(crate::env::Flow::Return(_)))
    ));
    drop(execution);
    drop(eval);
    assert!(
        frame.has_execution_lease(),
        "DbQuery only materializes query IR and must stay synchronous"
    );
    frame.finish(heap).expect("finish DbQuery frame");
}

#[tokio::test]
async fn f445h_e4r_spine_create_from_stream_pending_reacquires_and_finalizes_once() {
    let file = RecordingFile::new();
    let fixture = create_from_stream_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        test_runtime::outbound_context(),
        FileCapabilityContext::new(file.clone()),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(file.starts(), 1);
    assert!(!frame.has_execution_lease());
    tokio::time::timeout(Duration::from_secs(1), execution)
        .await
        .expect("createFromStream completes")
        .expect("createFromStream finalizes");
    drop(eval);
    assert_eq!(file.starts(), 1);
    assert_eq!(file.completions(), 1);
    assert_eq!(file.drops_before_completion(), 0);
    assert!(
        frame.has_execution_lease(),
        "file result must reacquire before native return finalization"
    );
    frame
        .finish(heap)
        .expect("finish createFromStream success frame");
}

#[tokio::test]
async fn f445h_e4r_spine_create_from_stream_pending_drop_settles_once() {
    let file = RecordingFile::new();
    let fixture = create_from_stream_fixture();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    let addr = executable_addr();
    let context = program_context_with(
        &fixture.interpreter,
        test_runtime::actor_context(),
        test_runtime::outbound_context(),
        FileCapabilityContext::new(file.clone()),
        DbCapabilityContext::unavailable(),
    );
    let mut eval = fixture.eval_context_with(context, frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(file.starts(), 1);
    assert!(!frame.has_execution_lease());
    drop(execution);
    drop(eval);
    assert_eq!(file.completions(), 0);
    assert_eq!(
        file.drops_before_completion(),
        1,
        "dropping the evaluator future must drop the prepared file wait once"
    );
    drop(heap);
    drop(frame);
}

fn emit_fixture(value: &str) -> EvaluatorFixture {
    EvaluatorFixture::new(
        vec![LinkedExprIr::Literal {
            value: LiteralIr::String {
                value: value.to_string(),
            },
        }],
        vec![
            LinkedStmtIr::Emit {
                operation: "emit".to_string(),
                value: ExprRefIr { expression: 0 },
            },
            LinkedStmtIr::Return { value: None },
        ],
        SlotLayoutIr::default(),
    )
}

#[tokio::test]
async fn f445h_e4r_spine_emit_detached_ready_keeps_actor_segment() {
    let fixture = emit_fixture("ready");
    let (_stream, sink) = fixture.interpreter.stream_runtime.channel_stream();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("buffered detached emit");

    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish ready emit frame");
}

#[tokio::test]
async fn f445h_e4r_spine_emit_detached_pending_cuts_actor_segment_once() {
    let fixture = emit_fixture("pending");
    let (stream, sink) = fixture.interpreter.stream_runtime.channel_stream();
    sink.send(json!("prefill")).await.expect("prefill stream");
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert!(!frame.has_execution_lease());
    assert!(matches!(
        fixture
            .interpreter
            .stream_runtime
            .next(&stream)
            .await
            .expect("consume prefill"),
        skiff_runtime_capability_context::StreamPoll::Item(value) if value == json!("prefill")
    ));
    execution.await.expect("pending detached emit completes");
    drop(eval);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish pending emit frame");
}

#[derive(Debug)]
struct ProjectingSink {
    pending: Mutex<Option<oneshot::Receiver<()>>>,
    sends: Arc<AtomicUsize>,
    cancellation: StreamSink,
}

impl ProjectingSink {
    fn ready() -> (StreamSink, Arc<AtomicUsize>) {
        Self::new(None)
    }

    fn pending() -> (StreamSink, Arc<AtomicUsize>, oneshot::Sender<()>) {
        let (sender, receiver) = oneshot::channel();
        let (sink, sends) = Self::new(Some(receiver));
        (sink, sends, sender)
    }

    fn new(receiver: Option<oneshot::Receiver<()>>) -> (StreamSink, Arc<AtomicUsize>) {
        let runtime = test_runtime::runtime_factory().stream_runtime();
        let (_, cancellation) = runtime.channel_stream();
        let sends = Arc::new(AtomicUsize::new(0));
        (
            StreamSink::new(Self {
                pending: Mutex::new(receiver),
                sends: Arc::clone(&sends),
                cancellation,
            }),
            sends,
        )
    }
}

impl StreamSinkApi for ProjectingSink {
    fn project_runtime_item(
        &self,
        item: RuntimeValue,
        _source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        Ok(Some(StreamInternalItem::new(item, RequestHeap::default())))
    }

    fn send_internal_with_cancellation<'a>(
        &'a self,
        _item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        self.sends.fetch_add(1, Ordering::AcqRel);
        let pending = self.pending.lock().expect("pending sink mutex").take();
        Box::pin(async move {
            if let Some(pending) = pending {
                pending
                    .await
                    .map_err(|_| StreamRuntimeError::decode("projected send gate dropped"))?;
            }
            Ok(())
        })
    }

    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn fail<'a>(
        &'a self,
        _error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn is_cancelled(&self) -> bool {
        false
    }

    fn is_same_stream(&self, _other: &StreamSink) -> bool {
        false
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        self.cancellation.cancel_signal()
    }
}

#[tokio::test]
async fn f445h_e4r_spine_emit_projected_ready_keeps_actor_segment() {
    let fixture = emit_fixture("projected-ready");
    let (sink, sends) = ProjectingSink::ready();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    fixture
        .eval_context(frame.clone(), &mut heap, &mut env, &addr)
        .exec_program_executable()
        .await
        .expect("projected Ready emit");

    assert_eq!(sends.load(Ordering::Acquire), 1);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish projected Ready frame");
}

#[tokio::test]
async fn f445h_e4r_spine_emit_projected_pending_reacquires_before_completion() {
    let fixture = emit_fixture("projected-pending");
    let (sink, sends, release) = ProjectingSink::pending();
    let (frame, mut heap) = fixture.actor_frame().await;
    let mut env = Env::new();
    env.stream_sink = Some(sink);
    let addr = executable_addr();
    let mut eval = fixture.eval_context(frame.clone(), &mut heap, &mut env, &addr);
    let mut execution = Box::pin(eval.exec_program_executable());

    assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
    assert_eq!(sends.load(Ordering::Acquire), 1);
    assert!(!frame.has_execution_lease());
    release.send(()).expect("release projected send");
    execution.await.expect("projected Pending emit");
    drop(eval);
    assert!(frame.has_execution_lease());
    frame.finish(heap).expect("finish projected Pending frame");
}

mod callback_matrix {
    use super::*;

    use skiff_artifact_model as artifact;
    use skiff_runtime_activation::{
        ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
        CallbackLifetime, RequestActivationContext,
    };
    use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
    use skiff_runtime_model::runtime_value::{
        InterfaceCarrier, InterfaceMethodSignature, InterfaceMethodSlot, InterfaceMethodTable,
        InterfaceMethodTarget, InterfaceMethodType, InterfaceReceiverCallAbi, InterfaceValue,
    };
    use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

    use crate::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

    const CALLBACK_PACKAGE_ID: &str = "example.f445h.callback-owner";
    const CALLBACK_INTERFACE_ABI: &str = "interface:f445h-e4r:callback";
    const CALLBACK_METHOD_ABI: &str = "method:f445h-e4r:callback";
    const CALLBACK_SCHEMA_ID: &str = "schema:f445h-e4r:callback";
    const CALLBACK_STABLE_KEY: &str = "api.Callback";

    struct CallbackFixture {
        evaluator: EvaluatorFixture,
        target: RuntimeAssemblyEvalTarget,
        carrier: skiff_runtime_model::runtime_value::CallbackCapabilityCarrier,
        caller_addr: ExecutableAddr,
    }

    fn callback_caller() -> EvaluatorFixture {
        let interface = LinkedInterfaceInstantiationRef {
            interface_abi_id: CALLBACK_INTERFACE_ABI.to_string(),
            canonical_type_args: Vec::new(),
        };
        EvaluatorFixture::new(
            vec![
                LinkedExprIr::LoadSlot { slot: 0 },
                LinkedExprIr::Call {
                    call: call(
                        LinkedCallTarget::InterfaceMethod {
                            interface,
                            method_abi_id: CALLBACK_METHOD_ABI.to_string(),
                            slot: 0,
                        },
                        vec![0],
                    ),
                },
            ],
            vec![
                LinkedStmtIr::Expr {
                    value: ExprRefIr { expression: 1 },
                },
                LinkedStmtIr::Return { value: None },
            ],
            SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "receiver".to_string(),
                    kind: "parameter".to_string(),
                }],
                frame_size: 1,
            },
        )
    }

    fn callback_owner_file(delay_ms: u64) -> artifact::FileIrUnit {
        let mut file =
            artifact::FileIrUnit::empty("callback.owner", "source:f445h-e4r-callback-owner");
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::Function,
            symbol: "callerAddressAnchor".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: artifact::TypeRefIr::builtin("void"),
            self_type: None,
            slots: artifact::SlotLayout::default(),
            may_suspend: false,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![artifact::StmtRefIr { statement: 0 }],
                }],
                statements: vec![artifact::StmtIr::Return { value: None }],
                expressions: Vec::new(),
            },
            source_span: None,
        });
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::ImplMethod,
            symbol: "invoke".to_string(),
            type_params: Vec::new(),
            params: vec![artifact::ParamIr {
                name: "self".to_string(),
                slot: 0,
                ty: artifact::TypeRefIr::builtin("string"),
            }],
            return_type: artifact::TypeRefIr::builtin("string"),
            self_type: Some(artifact::TypeRefIr::builtin("string")),
            slots: artifact::SlotLayout {
                slots: vec![artifact::SlotIr {
                    index: 0,
                    name: "self".to_string(),
                    kind: artifact::SlotKind::SelfValue,
                }],
                frame_size: 1,
            },
            may_suspend: true,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![
                        artifact::StmtRefIr { statement: 0 },
                        artifact::StmtRefIr { statement: 1 },
                    ],
                }],
                statements: vec![
                    artifact::StmtIr::Expr {
                        value: artifact::ExprRefIr { expression: 1 },
                    },
                    artifact::StmtIr::Return {
                        value: Some(artifact::ExprRefIr { expression: 2 }),
                    },
                ],
                expressions: vec![
                    artifact::ExprIr::Literal {
                        value: artifact::LiteralIr::Number {
                            value: serde_json::Number::from(delay_ms),
                        },
                    },
                    artifact::ExprIr::Call {
                        call: artifact::CallIr {
                            target: artifact::CallTargetIr::Native {
                                target: artifact::NativeTarget {
                                    namespace: "std.time".to_string(),
                                    symbol: "sleep".to_string(),
                                    binding_key: Some("std.time.sleep".to_string()),
                                    metadata: BTreeMap::new(),
                                },
                            },
                            site: site(),
                            args: vec![artifact::ExprRefIr { expression: 0 }],
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::new(),
                        },
                    },
                    artifact::ExprIr::Literal {
                        value: artifact::LiteralIr::String {
                            value: "callback-complete".to_string(),
                        },
                    },
                ],
            },
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("callback owner file identity");
        file
    }

    pub(super) fn file_ref(file: &artifact::FileIrUnit) -> artifact::FileIrRef {
        artifact::FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }
    }

    pub(super) fn private_package(
        package_id: &str,
        file: &artifact::FileIrUnit,
    ) -> artifact::PackageArtifact {
        artifact::PackageArtifact {
            schema_version: artifact::PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: artifact::PackageBuildId::new("unassigned"),
            files: vec![file_ref(file)],
            static_resources: Vec::new(),
            package_local_abi: artifact::PackageLocalAbi {
                local_abi_identity: artifact::PackageLocalAbiIdentity::new("unassigned"),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: artifact::PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity:
                    skiff_artifact_identity::package_schema_index_identity(
                        package_id,
                        &BTreeMap::new(),
                    )
                    .expect("empty callback Package schema index"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: artifact::PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: artifact::PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
        }
    }

    fn std_duration_package() -> (artifact::PackageArtifact, artifact::FileIrUnit) {
        let descriptor = artifact::TypeDescriptorIr::Representation {
            representation: artifact::TypeRefIr::builtin("integer"),
        };
        let mut file = artifact::FileIrUnit::empty("std.time", "source:f445h-e4r-std-duration");
        file.declarations.types.insert(
            "Duration".to_string(),
            artifact::TypeDeclarationIr {
                type_index: 0,
                symbol: "std.time.Duration".to_string(),
                source_span: None,
            },
        );
        file.type_table.push(artifact::TypeDeclIr {
            name: "Duration".to_string(),
            descriptor: descriptor.clone(),
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("std Duration file identity");

        let mut package = private_package("skiff.run/std", &file);
        package.package_local_abi.public_symbols.insert(
            "std.time.Duration".to_string(),
            artifact::PackageLocalAbiSymbol::Type {
                local_type_id: "type:skiff.run/std:top-level:std.time.Duration".to_string(),
                descriptor: descriptor.clone(),
                is_alias: false,
                is_interface: false,
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        package.implementation_links.types.insert(
            "std.time.Duration".to_string(),
            artifact::TypeExport {
                file: file_ref(&file),
                type_index: 0,
                symbol: "std.time.Duration".to_string(),
                is_interface: false,
                descriptor: Some(descriptor),
                type_params: Vec::new(),
                interface_methods: Vec::new(),
            },
        );
        skiff_artifact_identity::assign_package_artifact_identities(&mut package)
            .expect("std Duration package identities");
        (package, file)
    }

    pub(super) fn package_ref(package: &artifact::PackageArtifact) -> artifact::PackageArtifactRef {
        artifact::PackageArtifactRef {
            package_id: package.package_id.clone(),
            package_version: package.package_version.clone(),
            package_build_id: package.package_build_id.clone(),
            package_local_abi_identity: package.package_local_abi.local_abi_identity.clone(),
        }
    }

    fn activation(
        assembly_identity: artifact::AssemblyIdentity,
        package_build_id: artifact::PackageBuildId,
    ) -> Arc<ActivationContext> {
        ActivationContext::new(
            ActivationIdentity {
                assembly_identity,
                assembly_generation: 1,
                runtime_replica_id: "replica:f445h-e4r-callback".to_string(),
                deployment: artifact::ServiceDeploymentRef {
                    service_id: CALLBACK_PACKAGE_ID.to_string(),
                    contract_version: "1.0.0".to_string(),
                    deployment_revision: artifact::DeploymentRevision::new("f445h-e4r-callback-r1"),
                    deployment_artifact_identity: artifact::DeploymentArtifactIdentity::new(
                        "deployment:f445h-e4r-callback",
                    ),
                },
            },
            package_build_id,
            ActivationOwnedBindings {
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                policy: artifact::DeploymentPolicy {
                    timeout_ms: Some(1_000),
                    resources: artifact::ResourcePolicy {
                        cpu_millis: 100,
                        memory_bytes: 1_048_576,
                    },
                    activation: artifact::ActivationPolicy {
                        max_concurrency: 1,
                        idle_timeout_ms: None,
                    },
                    principal: "test".to_string(),
                },
            },
            Vec::new(),
        )
        .expect("callback activation")
    }

    struct Resolver {
        activation: Arc<ActivationContext>,
    }

    impl RuntimeAssemblyEvalResolver for Resolver {
        fn activation(&self, id: &ActivationId) -> Option<Arc<ActivationContext>> {
            (self.activation.activation_id() == id).then(|| Arc::clone(&self.activation))
        }

        fn activation_by_opaque_id(&self, id: &str) -> Option<Arc<ActivationContext>> {
            (self.activation.activation_id().as_str() == id).then(|| Arc::clone(&self.activation))
        }

        fn contract(
            &self,
            _contract: &artifact::ServiceContractRef,
        ) -> Option<Arc<artifact::ServiceContract>> {
            None
        }

        fn admitted_schema_records(
            &self,
            _contract: &artifact::ServiceContractRef,
        ) -> Option<crate::AdmittedPackageSchemaRecords> {
            None
        }

        fn operation_target(
            &self,
            _activation_id: &ActivationId,
            _operation: &artifact::ContractOperationId,
        ) -> Option<artifact::OperationTargetRef> {
            None
        }
    }

    fn callback_schema() -> (
        artifact::PackageSchemaTypeRef,
        BTreeMap<String, artifact::BoundaryCallbackOperation>,
        PackageSchemaRecords,
    ) {
        let reference = artifact::PackageSchemaTypeRef {
            package_id: CALLBACK_PACKAGE_ID.to_string(),
            stable_schema_key: CALLBACK_STABLE_KEY.to_string(),
            package_schema_type_id: artifact::PackageSchemaTypeId::new(CALLBACK_SCHEMA_ID),
        };
        let operations = BTreeMap::from([(
            "invoke".to_string(),
            artifact::BoundaryCallbackOperation {
                parameters: Vec::new(),
                return_type: artifact::ContractTypeRef::builtin("string"),
            },
        )]);
        let records = BTreeMap::from([(
            reference.package_schema_type_id.clone(),
            Arc::new(artifact::PackageSchemaTypeRecord {
                package_id: reference.package_id.clone(),
                stable_schema_key: reference.stable_schema_key.clone(),
                package_schema_type_id: reference.package_schema_type_id.clone(),
                canonical_descriptor: artifact::PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: artifact::ContractTypeDescriptor::CallbackInterface {
                        operations: operations.clone(),
                    },
                },
            }),
        )]);
        (reference, operations, records)
    }

    fn fixture(delay_ms: u64) -> CallbackFixture {
        let owner_file = callback_owner_file(delay_ms);
        let mut owner_package = private_package(CALLBACK_PACKAGE_ID, &owner_file);
        skiff_artifact_identity::assign_package_artifact_identities(&mut owner_package)
            .expect("callback owner package identities");
        let owner_ref = package_ref(&owner_package);
        let (std_package, std_file) = std_duration_package();
        let std_ref = package_ref(&std_package);
        let assembly = artifact::RuntimeAssembly {
            schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: artifact::AssemblyIdentity::new("assembly:f445h-e4r-callback"),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![owner_ref.clone(), std_ref.clone()],
            package_link_plan: artifact::CanonicalPackageLinkPlan {
                code_slots: vec![
                    artifact::PackageCodeSlot {
                        package: owner_ref.clone(),
                    },
                    artifact::PackageCodeSlot { package: std_ref },
                ],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let image = crate::test_support::link_package_fixture(
            assembly.clone(),
            vec![
                (owner_package, vec![owner_file]),
                (std_package, vec![std_file]),
            ],
        );
        let activation = activation(
            assembly.assembly_identity,
            owner_ref.package_build_id.clone(),
        );
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(Resolver {
            activation: Arc::clone(&activation),
        });
        let request =
            RequestActivationContext::begin(Arc::clone(&activation)).expect("callback request");
        let target =
            RuntimeAssemblyEvalTarget::new(image, request, resolver).expect("callback eval target");

        let (schema_type, operations, schema) = callback_schema();
        let callback_addr = ExecutableAddr::package(0, 0, 1);
        let local_interface = InterfaceValue::new(
            CALLBACK_INTERFACE_ABI.to_string(),
            InterfaceCarrier::Local {
                concrete_type: "callback.owner.State".to_string(),
                method_table: InterfaceMethodTable::new(
                    "table:f445h-e4r-callback".to_string(),
                    CALLBACK_INTERFACE_ABI.to_string(),
                    vec![InterfaceMethodSlot::from_admitted_metadata(
                        0,
                        "invoke".to_string(),
                        CALLBACK_METHOD_ABI.to_string(),
                        InterfaceMethodSignature::new(
                            vec![InterfaceMethodType::builtin("Self")],
                            InterfaceMethodType::builtin("string"),
                        ),
                        InterfaceMethodTarget::LocalExecutable {
                            executable: callback_addr,
                            receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                        },
                    )],
                ),
                payload: RuntimeValue::String("callback-owner".to_string()),
            },
        );
        let adapter = InProcessCallbackAdapter::from_local_interface(
            schema_type.clone(),
            &local_interface,
            &operations,
            &schema,
            &RequestHeap::default(),
        )
        .expect("callback adapter");
        let contract = serde_json::to_string(&schema_type).expect("callback contract identity");
        let carrier = activation
            .callback_capabilities()
            .register(
                &activation,
                target.request_activation(),
                contract,
                "callback:f445h-e4r",
                CallbackLifetime::Request,
                Arc::new(adapter),
            )
            .expect("register callback");

        CallbackFixture {
            evaluator: callback_caller(),
            target,
            carrier,
            caller_addr: ExecutableAddr::package(0, 0, 0),
        }
    }

    fn caller_env(fixture: &CallbackFixture, heap: &mut RequestHeap) -> Env {
        let interface = heap
            .alloc_interface(InterfaceValue::new(
                CALLBACK_INTERFACE_ABI.to_string(),
                InterfaceCarrier::CallbackCapability(fixture.carrier.clone()),
            ))
            .expect("callback receiver");
        let mut env = Env::for_program_executable(
            fixture.evaluator.executable(),
            Some(fixture.evaluator.file.module_path.clone()),
            1,
        )
        .expect("callback caller env");
        env.declare_binding("receiver", Some(0), RuntimeValue::Heap(interface))
            .expect("callback receiver binding");
        env
    }

    #[tokio::test]
    async fn f445h_e4r_spine_callback_ready_keeps_actor_segment() {
        let fixture = fixture(0);
        let (frame, mut heap) = fixture.evaluator.actor_frame().await;
        let mut env = caller_env(&fixture, &mut heap);
        let context = default_program_context(&fixture.evaluator.interpreter)
            .with_runtime_assembly_target(fixture.target.clone());
        let mut eval = fixture.evaluator.eval_context_with(
            context,
            frame.clone(),
            &mut heap,
            &mut env,
            &fixture.caller_addr,
        );
        let mut execution = Box::pin(eval.exec_program_executable());

        assert!(matches!(
            first_poll(execution.as_mut()),
            Poll::Ready(Ok(crate::env::Flow::Return(_)))
        ));
        drop(execution);
        drop(eval);
        assert!(
            frame.has_execution_lease(),
            "first-Ready callback must remain in the current Actor segment"
        );
        frame.finish(heap).expect("finish Ready callback frame");
    }

    #[tokio::test]
    async fn f445h_e4r_spine_callback_pending_reacquires_before_finalize() {
        let fixture = fixture(20);
        let (frame, mut heap) = fixture.evaluator.actor_frame().await;
        let mut env = caller_env(&fixture, &mut heap);
        let context = default_program_context(&fixture.evaluator.interpreter)
            .with_runtime_assembly_target(fixture.target.clone());
        let mut eval = fixture.evaluator.eval_context_with(
            context,
            frame.clone(),
            &mut heap,
            &mut env,
            &fixture.caller_addr,
        );
        let mut execution = Box::pin(eval.exec_program_executable());

        assert!(matches!(first_poll(execution.as_mut()), Poll::Pending));
        assert!(
            !frame.has_execution_lease(),
            "first-Pending callback must release the Actor segment"
        );
        tokio::time::timeout(Duration::from_secs(1), execution)
            .await
            .expect("callback completes")
            .expect("callback finalizes");
        drop(eval);
        assert!(
            frame.has_execution_lease(),
            "callback completion must reacquire before caller-heap finalize"
        );
        frame.finish(heap).expect("finish Pending callback frame");
    }
}

mod canonical_emit_matrix {
    use super::*;

    use std::{collections::BTreeSet, fmt, sync::atomic::AtomicU64};

    use skiff_artifact_model as artifact;
    use skiff_runtime_activation::{
        ActivationContext, ActivationId, ActivationIdentity, ActivationOwnedBindings,
        ActivationServiceBinding, RequestActivationContext,
    };
    use skiff_runtime_capability_context::StreamCancelSignalApi;

    use crate::{RuntimeAssemblyEvalResolver, RuntimeAssemblyEvalTarget};

    const CALLER_PACKAGE: &str = "example.f445h.emit-caller";
    const PROVIDER_PACKAGE: &str = "example.f445h.emit-provider";
    const SERVICE_ID: &str = "example.f445h.emit-service";
    const OPERATION_ID: &str = "operation:f445h-e4r:canonical-emit";

    #[derive(Debug)]
    enum ProbeEvent {
        Item(Value),
        Internal(StreamInternalItem),
        End,
        Fail(StreamRuntimeError),
    }

    #[derive(Default)]
    struct ProbeCounts {
        send_starts: AtomicUsize,
        send_completions: AtomicUsize,
    }

    struct ProbeState {
        id: u64,
        sender: tokio::sync::mpsc::Sender<ProbeEvent>,
        receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<ProbeEvent>>,
        counts: Arc<ProbeCounts>,
        cancelled: Arc<AtomicBool>,
        lifetime: Mutex<Option<StreamLifetimeGuard>>,
    }

    #[derive(Clone)]
    struct ProbeRuntime {
        state: Arc<ProbeState>,
    }

    impl fmt::Debug for ProbeRuntime {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ProbeRuntime")
                .field("id", &self.state.id)
                .finish()
        }
    }

    impl ProbeRuntime {
        fn new() -> (StreamRuntime, Arc<ProbeCounts>) {
            static NEXT_ID: AtomicU64 = AtomicU64::new(1);
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            let counts = Arc::new(ProbeCounts::default());
            let runtime = Self {
                state: Arc::new(ProbeState {
                    id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                    sender,
                    receiver: tokio::sync::Mutex::new(receiver),
                    counts: Arc::clone(&counts),
                    cancelled: Arc::new(AtomicBool::new(false)),
                    lifetime: Mutex::new(None),
                }),
            };
            (StreamRuntime::new(runtime), counts)
        }

        fn stream(&self, lifetime: Option<StreamLifetimeGuard>) -> (Value, StreamSink) {
            *self
                .state
                .lifetime
                .lock()
                .expect("probe stream lifetime lock") = lifetime;
            (
                skiff_runtime_boundary::stream::stream_value(&self.state.id.to_string()),
                StreamSink::new(ProbeSink {
                    state: Arc::clone(&self.state),
                }),
            )
        }

        async fn next_event(&self) -> StreamRuntimeResult<StreamPoll> {
            match self.state.receiver.lock().await.recv().await {
                Some(ProbeEvent::Item(value)) => Ok(StreamPoll::Item(value)),
                Some(ProbeEvent::Internal(item)) => Ok(StreamPoll::InternalItem(item)),
                Some(ProbeEvent::End) | None => Ok(StreamPoll::End),
                Some(ProbeEvent::Fail(error)) => Err(error),
            }
        }
    }

    impl StreamRuntimeApi for ProbeRuntime {
        fn channel_stream(&self) -> (Value, StreamSink) {
            self.stream(None)
        }

        fn channel_stream_with_lifetime(
            &self,
            lifetime: StreamLifetimeGuard,
        ) -> (Value, StreamSink) {
            self.stream(Some(lifetime))
        }

        fn pull_stream_with_cancellation(
            &self,
            _source: Box<dyn StreamPullSource>,
            _cancellation: CancellationToken,
        ) -> Value {
            panic!("canonical Emit probe does not create pull streams")
        }

        fn buffered_stream(&self, _items: Vec<Value>) -> Value {
            panic!("canonical Emit probe does not create buffered streams")
        }

        fn next_with_cancel<'a>(
            &'a self,
            _value: &'a Value,
            _signals: &'a [StreamCancelSignal],
            _cancel_flags: &'a [Arc<AtomicBool>],
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
            Box::pin(self.next_event())
        }

        fn next_with_cancellation<'a>(
            &'a self,
            _value: &'a Value,
            _signals: &'a [StreamCancelSignal],
            _cancel_tokens: Vec<CancellationToken>,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
            Box::pin(self.next_event())
        }

        fn next<'a>(
            &'a self,
            _value: &'a Value,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
            Box::pin(self.next_event())
        }

        fn cancel(&self, _value: &Value) {
            self.state.cancelled.store(true, Ordering::Release);
            self.state
                .lifetime
                .lock()
                .expect("probe stream lifetime lock")
                .take();
        }
    }

    #[derive(Clone)]
    struct ProbeSink {
        state: Arc<ProbeState>,
    }

    impl fmt::Debug for ProbeSink {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter
                .debug_struct("ProbeSink")
                .field("id", &self.state.id)
                .finish()
        }
    }

    impl ProbeSink {
        fn send_event<'a>(
            &'a self,
            event: ProbeEvent,
            count_send: bool,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            Box::pin(async move {
                if count_send {
                    self.state.counts.send_starts.fetch_add(1, Ordering::AcqRel);
                }
                self.state
                    .sender
                    .send(event)
                    .await
                    .map_err(|_| StreamRuntimeError::decode("probe stream receiver dropped"))?;
                if count_send {
                    self.state
                        .counts
                        .send_completions
                        .fetch_add(1, Ordering::AcqRel);
                }
                Ok(())
            })
        }
    }

    impl StreamSinkApi for ProbeSink {
        fn send_internal_with_cancellation<'a>(
            &'a self,
            item: StreamInternalItem,
            _signals: &'a [StreamCancelSignal],
            _cancel_tokens: Vec<CancellationToken>,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            self.send_event(ProbeEvent::Internal(item), true)
        }

        fn send<'a>(
            &'a self,
            item: Value,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            self.send_event(ProbeEvent::Item(item), true)
        }

        fn send_with_cancel<'a>(
            &'a self,
            item: Value,
            _cancel_flags: &'a [Arc<AtomicBool>],
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            self.send_event(ProbeEvent::Item(item), true)
        }

        fn send_with_cancellation<'a>(
            &'a self,
            item: Value,
            _signals: &'a [StreamCancelSignal],
            _cancel_tokens: Vec<CancellationToken>,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            self.send_event(ProbeEvent::Item(item), true)
        }

        fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                let _ = self.state.sender.send(ProbeEvent::End).await;
                self.state
                    .lifetime
                    .lock()
                    .expect("probe stream lifetime lock")
                    .take();
            })
        }

        fn fail<'a>(
            &'a self,
            error: StreamRuntimeError,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                let _ = self.state.sender.send(ProbeEvent::Fail(error)).await;
                self.state
                    .lifetime
                    .lock()
                    .expect("probe stream lifetime lock")
                    .take();
            })
        }

        fn is_cancelled(&self) -> bool {
            self.state.cancelled.load(Ordering::Acquire)
        }

        fn is_same_stream(&self, other: &StreamSink) -> bool {
            other
                .downcast_ref::<Self>()
                .is_some_and(|other| other.state.id == self.state.id)
        }

        fn cancel_flag(&self) -> Arc<AtomicBool> {
            Arc::clone(&self.state.cancelled)
        }

        fn cancel_signal(&self) -> StreamCancelSignal {
            StreamCancelSignal::new(NeverCancelled)
        }
    }

    #[derive(Debug)]
    struct NeverCancelled;

    impl StreamCancelSignalApi for NeverCancelled {
        fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(std::future::pending())
        }
    }

    struct ServiceFixture {
        evaluator: EvaluatorFixture,
        target: RuntimeAssemblyEvalTarget,
        caller_addr: ExecutableAddr,
    }

    fn provider_file(items: usize) -> artifact::FileIrUnit {
        let mut file =
            artifact::FileIrUnit::empty("emit.provider", "source:f445h-e4r-canonical-emit");
        let expressions = (0..items)
            .map(|index| artifact::ExprIr::Literal {
                value: artifact::LiteralIr::String {
                    value: format!("canonical-{index}"),
                },
            })
            .collect::<Vec<_>>();
        let mut statements = (0..items)
            .map(|index| artifact::StmtIr::Emit {
                operation: "emit".to_string(),
                value: artifact::ExprRefIr {
                    expression: index as u32,
                },
            })
            .collect::<Vec<_>>();
        statements.push(artifact::StmtIr::Return { value: None });
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::Function,
            symbol: "stream".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: artifact::TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![artifact::TypeRefIr::builtin("string")],
            },
            self_type: None,
            slots: artifact::SlotLayout::default(),
            may_suspend: true,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: (0..statements.len())
                        .map(|statement| artifact::StmtRefIr {
                            statement: statement as u32,
                        })
                        .collect(),
                }],
                statements,
                expressions,
            },
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("canonical Emit provider file identity");
        file
    }

    fn caller_file(service_call: &artifact::ServiceCallRef) -> artifact::FileIrUnit {
        let mut file =
            artifact::FileIrUnit::empty("emit.caller", "source:f445h-e4r-canonical-caller");
        file.external_refs
            .service_call_refs
            .push(service_call.clone());
        file.executables.push(artifact::ExecutableIr {
            kind: artifact::ExecutableKind::Function,
            symbol: "anchor".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: artifact::TypeRefIr::Builtin {
                name: "Stream".to_string(),
                args: vec![artifact::TypeRefIr::builtin("string")],
            },
            self_type: None,
            slots: artifact::SlotLayout::default(),
            may_suspend: true,
            body: artifact::ExecutableBody {
                blocks: vec![artifact::BlockIr {
                    label: "entry".to_string(),
                    statements: vec![artifact::StmtRefIr { statement: 0 }],
                }],
                statements: vec![artifact::StmtIr::Return {
                    value: Some(artifact::ExprRefIr { expression: 0 }),
                }],
                expressions: vec![artifact::ExprIr::Call {
                    call: artifact::CallIr {
                        target: artifact::CallTargetIr::ServiceCall {
                            service_call_ref_index: artifact::ServiceCallRefIndex::new(0),
                        },
                        site: site(),
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::new(),
                    },
                }],
            },
            source_span: None,
        });
        skiff_artifact_identity::assign_file_ir_identity(&mut file)
            .expect("canonical Emit caller file identity");
        file
    }

    fn detached_plan(
        owner: artifact::BoundaryValueOwner,
        lifetime: artifact::BoundaryValueLifetime,
    ) -> artifact::BoundaryValuePlan {
        artifact::BoundaryValuePlan::Linkable {
            carrier: artifact::BoundaryValueCarrier::DetachedValueGraph,
            encoding: artifact::BoundaryValueEncoding::CanonicalValue,
            owner,
            lifetime,
        }
    }

    fn service_contract() -> artifact::ServiceContract {
        let operation = artifact::ContractOperationId::new(OPERATION_ID);
        artifact::ServiceContract {
            schema_version: artifact::SERVICE_CONTRACT_SCHEMA_VERSION.to_string(),
            service_id: SERVICE_ID.to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: artifact::ServiceProtocolIdentity::new(
                "protocol:f445h-e4r-canonical-emit",
            ),
            operations: BTreeMap::from([(
                operation.clone(),
                artifact::BoundaryOperationDescriptor {
                    operation_id: operation.clone(),
                    stable_key: "stream".to_string(),
                    contract: artifact::BoundaryOperationContract {
                        parameters: Vec::new(),
                        return_value: artifact::BoundaryReturn {
                            ty: artifact::ContractTypeRef::builtin("void"),
                            value_plan: detached_plan(
                                artifact::BoundaryValueOwner::Provider,
                                artifact::BoundaryValueLifetime::Call,
                            ),
                        },
                        stream: artifact::BoundaryStreamContract::ServerStream {
                            item_type: artifact::ContractTypeRef::builtin("string"),
                            item_value_plan: detached_plan(
                                artifact::BoundaryValueOwner::Provider,
                                artifact::BoundaryValueLifetime::Stream,
                            ),
                        },
                        callbacks: artifact::BoundaryCallbackContract::None,
                        effect_guarantee: artifact::BoundaryEffectGuarantee {
                            detached_parameters: true,
                            detached_return: true,
                            detached_error: true,
                            no_caller_reachable_mutation: true,
                            no_caller_value_escape: true,
                            no_same_heap_identity: true,
                        },
                    },
                },
            )]),
            package_type_requirements: Vec::new(),
            diagnostic_text: artifact::ContractDiagnosticText {
                service: "canonical Emit fixture".to_string(),
                operations: BTreeMap::from([(operation, "stream".to_string())]),
                types: BTreeMap::new(),
            },
        }
    }

    fn contract_ref(contract: &artifact::ServiceContract) -> artifact::ServiceContractRef {
        artifact::ServiceContractRef {
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            service_protocol_identity: contract.service_protocol_identity.clone(),
        }
    }

    fn contract_requirement(
        contract: &artifact::ServiceContractRef,
    ) -> artifact::ContractRequirement {
        artifact::ContractRequirement {
            alias: "emit".to_string(),
            service_id: contract.service_id.clone(),
            contract_version: contract.contract_version.clone(),
            expected_protocol_identity: contract.service_protocol_identity.clone(),
        }
    }

    fn activation_bindings() -> ActivationOwnedBindings {
        ActivationOwnedBindings {
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            policy: artifact::DeploymentPolicy {
                timeout_ms: Some(1_000),
                resources: artifact::ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_048_576,
                },
                activation: artifact::ActivationPolicy {
                    max_concurrency: 1,
                    idle_timeout_ms: None,
                },
                principal: "test".to_string(),
            },
        }
    }

    fn activation_identity(
        assembly_identity: artifact::AssemblyIdentity,
        service_id: &str,
        revision: &str,
    ) -> ActivationIdentity {
        ActivationIdentity {
            assembly_identity,
            assembly_generation: 1,
            runtime_replica_id: "replica:f445h-e4r-canonical-emit".to_string(),
            deployment: artifact::ServiceDeploymentRef {
                service_id: service_id.to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: artifact::DeploymentRevision::new(revision),
                deployment_artifact_identity: artifact::DeploymentArtifactIdentity::new(format!(
                    "deployment:f445h-e4r:{revision}"
                )),
            },
        }
    }

    struct Resolver {
        activations: BTreeMap<ActivationId, Arc<ActivationContext>>,
        contract: Arc<artifact::ServiceContract>,
        contract_ref: artifact::ServiceContractRef,
        operation: artifact::ContractOperationId,
        provider: ActivationId,
        target: artifact::OperationTargetRef,
    }

    impl RuntimeAssemblyEvalResolver for Resolver {
        fn activation(&self, id: &ActivationId) -> Option<Arc<ActivationContext>> {
            self.activations.get(id).cloned()
        }

        fn activation_by_opaque_id(&self, id: &str) -> Option<Arc<ActivationContext>> {
            self.activations
                .values()
                .find(|activation| activation.activation_id().as_str() == id)
                .cloned()
        }

        fn contract(
            &self,
            contract: &artifact::ServiceContractRef,
        ) -> Option<Arc<artifact::ServiceContract>> {
            (contract == &self.contract_ref).then(|| Arc::clone(&self.contract))
        }

        fn admitted_schema_records(
            &self,
            contract: &artifact::ServiceContractRef,
        ) -> Option<crate::AdmittedPackageSchemaRecords> {
            (contract == &self.contract_ref).then(|| Arc::new(BTreeMap::new()))
        }

        fn operation_target(
            &self,
            activation: &ActivationId,
            operation: &artifact::ContractOperationId,
        ) -> Option<artifact::OperationTargetRef> {
            (activation == &self.provider && operation == &self.operation)
                .then(|| self.target.clone())
        }
    }

    fn fixture(items: usize) -> ServiceFixture {
        let contract = Arc::new(service_contract());
        let contract_ref = contract_ref(&contract);
        let operation = artifact::ContractOperationId::new(OPERATION_ID);
        let service_call = artifact::ServiceCallRef {
            service_requirement_slot: 0,
            contract_operation_id: operation.clone(),
            expected_protocol_identity: contract_ref.service_protocol_identity.clone(),
        };
        let caller_file = caller_file(&service_call);
        let provider_file = provider_file(items);

        let requirement = contract_requirement(&contract_ref);
        let mut caller_package =
            super::callback_matrix::private_package(CALLER_PACKAGE, &caller_file);
        caller_package
            .contract_requirements
            .push(requirement.clone());
        caller_package
            .service_requirements
            .push(artifact::ServiceRequirement {
                contract_requirement: requirement,
                service_binding_slot: 0,
                used_operations: BTreeSet::from([operation.clone()]),
            });
        caller_package.service_call_refs.push(service_call);
        skiff_artifact_identity::assign_package_artifact_identities(&mut caller_package)
            .expect("canonical Emit caller package identities");
        let caller_ref = super::callback_matrix::package_ref(&caller_package);

        let mut provider_package =
            super::callback_matrix::private_package(PROVIDER_PACKAGE, &provider_file);
        skiff_artifact_identity::assign_package_artifact_identities(&mut provider_package)
            .expect("canonical Emit provider package identities");
        let provider_ref = super::callback_matrix::package_ref(&provider_package);
        let assembly_identity =
            artifact::AssemblyIdentity::new("assembly:f445h-e4r-canonical-emit");
        let assembly = artifact::RuntimeAssembly {
            schema_version: artifact::RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: assembly_identity.clone(),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: vec![caller_ref.clone(), provider_ref.clone()],
            package_link_plan: artifact::CanonicalPackageLinkPlan {
                code_slots: vec![
                    artifact::PackageCodeSlot {
                        package: caller_ref.clone(),
                    },
                    artifact::PackageCodeSlot {
                        package: provider_ref.clone(),
                    },
                ],
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
        let image = crate::test_support::link_package_fixture(
            assembly,
            vec![
                (caller_package, vec![caller_file.clone()]),
                (provider_package, vec![provider_file.clone()]),
            ],
        );
        let target = artifact::OperationTargetRef {
            file_ref: super::callback_matrix::file_ref(&provider_file),
            executable_index: 0,
            callable_abi_id: OPERATION_ID.to_string(),
            callable_kind: artifact::OperationCallableKind::PublicFunction,
        };
        let provider = ActivationContext::new(
            activation_identity(
                assembly_identity.clone(),
                SERVICE_ID,
                "canonical-emit-provider-r1",
            ),
            provider_ref.package_build_id.clone(),
            activation_bindings(),
            Vec::new(),
        )
        .expect("canonical Emit provider activation");
        let binding = ActivationServiceBinding::new(
            artifact::ServiceRequirementKey {
                caller_package_build_id: caller_ref.package_build_id.clone(),
                service_requirement_slot: 0,
            },
            provider.activation_id().clone(),
            contract_ref.clone(),
            vec![operation.clone()],
        )
        .expect("canonical Emit service binding");
        let caller = ActivationContext::new(
            activation_identity(
                assembly_identity,
                CALLER_PACKAGE,
                "canonical-emit-caller-r1",
            ),
            caller_ref.package_build_id.clone(),
            activation_bindings(),
            vec![binding],
        )
        .expect("canonical Emit caller activation");
        let activations = BTreeMap::from([
            (caller.activation_id().clone(), Arc::clone(&caller)),
            (provider.activation_id().clone(), Arc::clone(&provider)),
        ]);
        let resolver: Arc<dyn RuntimeAssemblyEvalResolver> = Arc::new(Resolver {
            activations,
            contract,
            contract_ref,
            operation,
            provider: provider.activation_id().clone(),
            target,
        });
        let instruction = image
            .resolve_activation_relative_service_call(
                &caller_ref.package_build_id,
                &caller_file.file_ir_identity,
                artifact::ServiceCallRefIndex::new(0),
            )
            .expect("canonical Emit service instruction");
        let request =
            RequestActivationContext::begin(caller).expect("canonical Emit request activation");
        let target = RuntimeAssemblyEvalTarget::new(image, request, resolver)
            .expect("canonical Emit eval target");
        let evaluator = EvaluatorFixture::new(
            vec![LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActivationRelativeService {
                        instruction: instruction.clone(),
                    },
                    Vec::new(),
                ),
            }],
            vec![LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 0 }),
            }],
            SlotLayoutIr::default(),
        );
        ServiceFixture {
            evaluator,
            target,
            caller_addr: ExecutableAddr::package(0, 0, 0),
        }
    }

    async fn start_stream(
        fixture: &ServiceFixture,
        runtime: StreamRuntime,
    ) -> (Value, ActorExecutionFrame) {
        let (frame, mut heap) = fixture.evaluator.actor_frame().await;
        let mut env = Env::new();
        let context = program_context_with_stream(
            &fixture.evaluator.interpreter,
            test_runtime::actor_context(),
            test_runtime::outbound_context(),
            test_runtime::file_context(),
            DbCapabilityContext::unavailable(),
            runtime,
        )
        .with_websocket_capability_rebinder(test_runtime::websocket_rebinder())
        .with_runtime_assembly_target(fixture.target.clone());
        let mut eval = fixture.evaluator.eval_context_with(
            context,
            frame.clone(),
            &mut heap,
            &mut env,
            &fixture.caller_addr,
        );
        let value = eval
            .eval_program_expr_ref(ExprRefIr { expression: 0 })
            .await
            .expect("canonical service stream");
        let value = crate::runtime_ops::runtime_to_wire(value.value(), &*eval.heap)
            .expect("canonical stream wire handle");
        drop(eval);
        assert!(
            frame.has_execution_lease(),
            "frozen activation setup must reacquire its Actor segment"
        );
        frame
            .clone()
            .finish(heap)
            .expect("finish canonical Emit caller frame");
        (value, frame)
    }

    async fn wait_for_counts(counts: &ProbeCounts, starts: usize, completions: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if counts.send_starts.load(Ordering::Acquire) == starts
                    && counts.send_completions.load(Ordering::Acquire) == completions
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("canonical Emit counters reach expected state");
    }

    #[tokio::test]
    async fn f445h_e4r_spine_emit_canonical_wire_ready_completes_first_poll() {
        let fixture = fixture(1);
        let (runtime, counts) = ProbeRuntime::new();
        let (stream, _frame) = start_stream(&fixture, runtime.clone()).await;

        wait_for_counts(&counts, 1, 1).await;
        assert!(matches!(
            runtime.next(&stream).await.expect("canonical item"),
            StreamPoll::Item(Value::String(value)) if value == "canonical-0"
        ));
        assert!(matches!(
            runtime.next(&stream).await.expect("canonical end"),
            StreamPoll::End
        ));
    }

    #[tokio::test]
    async fn f445h_e4r_spine_emit_canonical_wire_pending_resumes_same_send_once() {
        let fixture = fixture(2);
        let (runtime, counts) = ProbeRuntime::new();
        let (stream, _frame) = start_stream(&fixture, runtime.clone()).await;

        wait_for_counts(&counts, 2, 1).await;
        assert!(matches!(
            runtime.next(&stream).await.expect("first canonical item"),
            StreamPoll::Item(Value::String(value)) if value == "canonical-0"
        ));
        wait_for_counts(&counts, 2, 2).await;
        assert!(matches!(
            runtime.next(&stream).await.expect("second canonical item"),
            StreamPoll::Item(Value::String(value)) if value == "canonical-1"
        ));
        assert!(matches!(
            runtime.next(&stream).await.expect("canonical end"),
            StreamPoll::End
        ));
    }
}

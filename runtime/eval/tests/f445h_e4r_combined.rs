use std::{
    collections::{BTreeMap, HashMap},
    fs,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Condvar, Mutex, OnceLock,
    },
    task::{Context, Poll, Wake, Waker},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use bytes::Bytes;
use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, InstructionSourceSite,
    PackageSchemaIndex, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    SyntheticInstructionSiteReason, ACTOR_RUNTIME_ABI_VERSION_V1,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::{assembly::resolve_runtime_assembly, storage::CanonicalArtifactStore};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_boundary::{
    file::{FileCreateOptions, ImmutableFileRef},
    stream::stream_value,
};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorInvocationOutcome, ActorInvocationRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken, CapabilityError,
    CapabilityFuture, ConfigCapabilityApi, ConfigCapabilityContext, DbCapabilityContext,
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControl, ExecutionControlApi,
    ExecutionControlError, ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError,
    ExecutionScopeTerminal, FileCapabilityApi, FileCapabilityContext, FileCapabilityFuture,
    FileCapabilitySource, FileCapabilitySourceApi, FileChunkSource, FileSourceStreamApi,
    FileSourceStreamContext, HttpCapabilityFuture, HttpClientCapabilityApi,
    HttpClientCapabilityContext, OwnedActorCapabilityContext, OwnedConfigCapabilityContext,
    OwnedExecutionControl, OwnedExecutionControlApi, OwnedWebsocketCapabilityContext,
    SpawnSubmitControlRequest, StreamCancelSignal, StreamCancelSignalApi, StreamLifetimeGuard,
    StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError,
    StreamRuntimeResult, StreamSink, StreamSinkApi, TelemetryCapabilityApi,
    TelemetryCapabilityContext, TimeCapabilityContext, WebsocketCapabilityApi,
    WebsocketCapabilityContext as SharedWebsocketCapabilityContext,
};
use skiff_runtime_eval::{
    actor_executor::{ActorMethodExecutionRequest, ActorMethodExecutor},
    actor_instance::{
        ActorActivationRequest, ActorIncarnationKey, ActorInstanceFence, ActorInstanceHandle,
        ActorInstanceStore, ActorLogicalKey, ACTOR_BOOTSTRAP_ENCODING_V1,
    },
    capabilities::{
        EffectDispatchApi, EffectDispatchContext, EvalCapabilityFuture, EvalRuntimeFactory,
        EvalRuntimeFactoryApi, HttpRuntimeOptions, OutboundServiceApi, OutboundServiceContext,
        TestEffectDouble, TestEffectDoubleContext, TestEffectDoubleContextApi,
        WebsocketCapabilityContext as EvalWebsocketCapabilityContext,
    },
    env::Env,
    error::{Result as EvalResult, RuntimeError},
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram, Interpreter,
};
use skiff_runtime_linked_program::{
    anonymous_type_decl, ActivationRelativeServiceCall, BlockIr, CallIr, ExecutableAddr,
    ExecutableKind, ExprRefIr, ExternalRefTable, FileAddr, FileDeclarations, FileLinkTargets,
    HydratedPackageCode, LinkOverlay, LinkedActorDeclaration, LinkedActorDeclarationOwner,
    LinkedActorMethodImplementation, LinkedActorPublicMethod, LinkedCallTarget,
    LinkedConcurrentLaneIr, LinkedConcurrentPlanIr, LinkedExecutable, LinkedExecutableBody,
    LinkedExprIr, LinkedFileUnit, LinkedFunctionTypeParamIr, LinkedStmtIr, LinkedTypeDescriptor,
    LinkedTypeRef, LiteralIr, NativeTarget, PackageSymbolKey, PackageUnit,
    PublicationResourceTable, RuntimeTypeContext, ServiceDependencyConstraint, ServiceMeta, SlotIr,
    SlotLayoutIr, SourceMapDto, StmtRefIr, TypeAddr, UnitAddr,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::RuntimeTypePlan,
};
use skiff_test_runner::canonical_std_seed::seed_canonical_std;

const SERVICE_ID: &str = "skiff.test/f445h-e4r-combined";
const ACTOR_FILE_ID: &str = "file:f445h-e4r-combined-actor";
const VERSION: &str = "1.0.0";

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn integer() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "integer".to_string(),
        args: Vec::new(),
    }
}

fn string_type() -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "string".to_string(),
        args: Vec::new(),
    }
}

fn number(value: u64) -> LinkedExprIr {
    LinkedExprIr::Literal {
        value: LiteralIr::Number {
            value: serde_json::Number::from(value),
        },
    }
}

fn call(target: LinkedCallTarget, args: &[u32]) -> CallIr {
    CallIr {
        target,
        site: site(),
        args: args
            .iter()
            .copied()
            .map(|expression| ExprRefIr { expression })
            .collect(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
    }
}

fn native_sleep_call(argument: u32) -> LinkedExprIr {
    LinkedExprIr::Call {
        call: call(
            LinkedCallTarget::Native {
                target: NativeTarget {
                    namespace: "std.time".to_string(),
                    symbol: "sleep".to_string(),
                    binding_key: Some("std.time.sleep".to_string()),
                    metadata: BTreeMap::new(),
                },
            },
            &[argument],
        ),
    }
}

#[derive(Debug, Default)]
struct ProbeStreamState {
    next_calls: AtomicUsize,
    last_cancel_token_count: AtomicUsize,
    cleanup_cancels: AtomicUsize,
    cancelled: AtomicBool,
}

#[derive(Clone, Debug, Default)]
struct ProbeStreamRuntime {
    state: Arc<ProbeStreamState>,
}

impl ProbeStreamRuntime {
    fn wait_with_tokens<'a>(
        &'a self,
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.state.next_calls.fetch_add(1, Ordering::AcqRel);
        self.state
            .last_cancel_token_count
            .store(cancel_tokens.len(), Ordering::Release);
        Box::pin(async move {
            loop {
                if self.state.cancelled.load(Ordering::Acquire)
                    || cancel_tokens.iter().any(CancellationToken::is_cancelled)
                {
                    return Err(StreamRuntimeError::cancelled());
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
    }
}

impl StreamRuntimeApi for ProbeStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        (
            stream_value("f445h-e4r-combined-channel"),
            StreamSink::new(NoopStreamSink::default()),
        )
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        self.channel_stream()
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        stream_value("f445h-e4r-combined-pull")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        stream_value("f445h-e4r-combined-buffered")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(
            cancel_flags
                .iter()
                .cloned()
                .map(CancellationToken::from_flag)
                .collect(),
        )
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(cancel_tokens)
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        self.wait_with_tokens(Vec::new())
    }

    fn cancel(&self, _value: &Value) {
        self.state.cancelled.store(true, Ordering::Release);
        self.state.cleanup_cancels.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Debug)]
struct NoopStreamSignal {
    token: CancellationToken,
}

impl StreamCancelSignalApi for NoopStreamSignal {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(self.token.wait_cancelled())
    }
}

#[derive(Clone, Debug)]
struct NoopStreamSink {
    cancellation: CancellationToken,
}

impl Default for NoopStreamSink {
    fn default() -> Self {
        Self {
            cancellation: CancellationToken::new(),
        }
    }
}

impl StreamSinkApi for NoopStreamSink {
    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if cancel_flags.iter().any(|flag| flag.load(Ordering::Acquire)) {
                Err(StreamRuntimeError::cancelled())
            } else {
                Ok(())
            }
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            if cancel_tokens.iter().any(CancellationToken::is_cancelled) {
                Err(StreamRuntimeError::cancelled())
            } else {
                Ok(())
            }
        })
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
        self.cancellation.is_cancelled()
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other.downcast_ref::<Self>().is_some()
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.cancellation.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(NoopStreamSignal {
            token: self.cancellation.clone(),
        })
    }
}

#[derive(Clone)]
struct HarnessRuntimeFactory {
    stream: ProbeStreamRuntime,
}

impl EvalRuntimeFactoryApi for HarnessRuntimeFactory {
    fn stream_runtime(&self) -> StreamRuntime {
        StreamRuntime::new(self.stream.clone())
    }

    fn reusable_test_effect_doubles(
        &self,
        _doubles: HashMap<String, TestEffectDouble>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(NoTestEffects)
    }

    fn one_shot_test_effect_double_sequences(
        &self,
        _doubles: HashMap<String, Vec<TestEffectDouble>>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(NoTestEffects)
    }
}

struct NoTestEffects;

impl TestEffectDoubleContextApi for NoTestEffects {
    fn next_test_effect_double(&self, _target: &str) -> Option<TestEffectDouble> {
        None
    }

    fn dispatch_test_effect_double(
        &self,
        _target: &str,
        _input: Option<&Value>,
    ) -> Option<EvalResult<Value>> {
        None
    }

    fn dispatch_test_stable_target_double(
        &self,
        _target: &str,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }

    fn dispatch_test_host_operation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }

    fn dispatch_test_http_effect_invocation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<EvalResult<RuntimeValue>> {
        None
    }
}

#[derive(Clone)]
struct HarnessControl {
    request_cancellation: CancellationToken,
    cancellation_flag: Arc<AtomicBool>,
    scope: ExecutionScope,
    instruction_units: Arc<AtomicU64>,
}

impl HarnessControl {
    fn request() -> Self {
        let request_cancellation = CancellationToken::new();
        let cancellation_flag = request_cancellation.cancel_flag();
        Self {
            scope: ExecutionScope::request(request_cancellation.clone(), None),
            request_cancellation,
            cancellation_flag,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    fn child(deadline: Instant) -> (Self, ExecutionScope) {
        let root = Self::request();
        let scope = root
            .scope
            .derive(deadline, site())
            .expect("combined child scope");
        (
            Self {
                request_cancellation: root.request_cancellation,
                cancellation_flag: root.cancellation_flag,
                scope: scope.clone(),
                instruction_units: root.instruction_units,
            },
            scope,
        )
    }

    fn terminal_error(&self) -> Option<ExecutionControlError> {
        match self.scope.terminal_at(Instant::now()) {
            Some(ExecutionScopeTerminal::AncestorCancelled) => {
                Some(ExecutionControlError::Cancelled)
            }
            Some(
                ExecutionScopeTerminal::LocalDeadlineExceeded(_)
                | ExecutionScopeTerminal::InheritedDeadlineExceeded(_),
            ) => Some(ExecutionControlError::BudgetExceeded(
                ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::DeadlineExceeded,
                    instruction_count: self.instruction_units.load(Ordering::Acquire),
                    limit: None,
                    elapsed_ms: 0.0,
                },
            )),
            None => None,
        }
    }
}

impl ExecutionControlApi for HarnessControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation_flag)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.request_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        let scope = self.scope.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(Self {
            request_cancellation: self.request_cancellation.clone(),
            cancellation_flag: Arc::clone(&self.cancellation_flag),
            scope,
            instruction_units: Arc::clone(&self.instruction_units),
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        self.terminal_error().map_or(Ok(()), Err)
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::AcqRel);
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        FileSourceStreamContext::from_api(HarnessFileSourceStream { stream_runtime })
    }
}

impl OwnedExecutionControlApi for HarnessControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancellation_flag.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.request_cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

#[derive(Default)]
struct BlockingConfigState {
    released: Mutex<bool>,
    release: Condvar,
}

#[derive(Clone)]
struct HarnessConfig {
    entered: Option<mpsc::Sender<()>>,
    blocking: Option<Arc<BlockingConfigState>>,
}

impl HarnessConfig {
    fn ordinary() -> Self {
        Self {
            entered: None,
            blocking: None,
        }
    }

    fn blocking(entered: mpsc::Sender<()>, blocking: Arc<BlockingConfigState>) -> HarnessConfig {
        Self {
            entered: Some(entered),
            blocking: Some(blocking),
        }
    }
}

impl ConfigCapabilityApi for HarnessConfig {
    fn owned(&self) -> OwnedConfigCapabilityContext {
        ConfigCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ConfigCapabilityContext<'_> {
        ConfigCapabilityContext::new(self.clone())
    }

    fn read_config_target(
        &self,
        _current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        _type_arg: Option<&RuntimeTypePlan>,
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        if target == "config.require" && args.first().and_then(Value::as_str) == Some("barrier") {
            if let Some(entered) = &self.entered {
                let _ = entered.send(());
            }
            if let Some(blocking) = &self.blocking {
                let mut released = blocking
                    .released
                    .lock()
                    .expect("combined activation barrier lock");
                while !*released {
                    released = blocking
                        .release
                        .wait(released)
                        .expect("combined activation barrier wait");
                }
            }
        }
        Ok(Value::String("released".to_string()))
    }
}

#[derive(Clone)]
struct HarnessFileSource;

impl FileCapabilitySourceApi for HarnessFileSource {
    fn context_for_request(&self, _db_context: DbCapabilityContext) -> FileCapabilityContext {
        FileCapabilityContext::new(HarnessFile)
    }
}

#[derive(Clone)]
struct HarnessFile;

fn unavailable_file<'a, T>() -> FileCapabilityFuture<'a, T> {
    Box::pin(async {
        Err(skiff_runtime_capability_context::FileCapabilityError::file(
            "combined test file capability is unavailable",
        ))
    })
}

impl FileCapabilityApi for HarnessFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(HarnessFileSource)
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a ImmutableFileRef,
    ) -> FileCapabilityFuture<'a, ()> {
        unavailable_file()
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        _next_chunk: FileChunkSource<'a>,
    ) -> FileCapabilityFuture<'a, Value> {
        unavailable_file()
    }
}

#[derive(Clone)]
struct HarnessFileSourceStream {
    stream_runtime: StreamRuntime,
}

impl FileSourceStreamApi for HarnessFileSourceStream {
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
                StreamPoll::InternalItem(_) => Err(
                    skiff_runtime_capability_context::FileCapabilityError::decode(
                        "combined test does not project internal file stream items",
                    ),
                ),
            }
        })
    }
}

#[derive(Clone)]
struct HarnessActor;

impl ActorCapabilityApi for HarnessActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "combined-runtime"
    }

    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    fn service_version(&self) -> &str {
        VERSION
    }

    fn request_id(&self) -> &str {
        "combined-request"
    }

    fn request_target(&self) -> &str {
        SERVICE_ID
    }

    fn request_build_id(&self) -> &str {
        "combined-build"
    }

    fn spawn_service_protocol_identity(&self) -> &str {
        ""
    }

    fn request_service_protocol_identity(&self) -> &str {
        ""
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
        unsupported_capability()
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        unsupported_capability()
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        unsupported_capability()
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool> {
        unsupported_capability()
    }

    fn submit_spawn<'a>(
        &'a self,
        _request: SpawnSubmitControlRequest,
        _args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()> {
        unsupported_capability()
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome> {
        unsupported_capability()
    }
}

fn unsupported_capability<'a, T>() -> CapabilityFuture<'a, T> {
    Box::pin(async {
        Err(CapabilityError::unsupported(
            "combined test capability is unavailable",
        ))
    })
}

#[derive(Clone)]
struct HarnessWebsocket;

impl WebsocketCapabilityApi for HarnessWebsocket {
    fn owned(&self) -> OwnedWebsocketCapabilityContext {
        SharedWebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> SharedWebsocketCapabilityContext<'_> {
        SharedWebsocketCapabilityContext::new(self.clone())
    }

    fn service_id(&self) -> &str {
        SERVICE_ID
    }

    fn websocket_entry_id(&self) -> Option<&str> {
        None
    }

    fn send_connection_text_to_business_identity(
        &self,
        _business_identity: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_binary_to_business_identity(
        &self,
        _business_identity: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_text_to_connection(
        &self,
        _connection_id: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }

    fn send_connection_binary_to_connection(
        &self,
        _connection_id: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "combined test websocket capability is unavailable",
        ))
    }
}

struct HarnessEffects;

impl EffectDispatchApi for HarnessEffects {
    fn telemetry_context(&self) -> TelemetryCapabilityContext {
        TelemetryCapabilityContext::new(HarnessTelemetry)
    }

    fn http_client_context(
        &self,
        _http_options: HttpRuntimeOptions,
        _stream_runtime: StreamRuntime,
        _test_effect_doubles: TestEffectDoubleContext,
    ) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(HarnessHttp)
    }
}

struct HarnessTelemetry;

impl TelemetryCapabilityApi for HarnessTelemetry {
    fn emit_native(
        &self,
        _target: &str,
        _args: &[Value],
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        Err(CapabilityError::unsupported(
            "combined test telemetry capability is unavailable",
        ))
    }
}

struct HarnessHttp;

impl HttpClientCapabilityApi for HarnessHttp {
    fn with_stream_runtime(&self, _stream_runtime: StreamRuntime) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(Self)
    }

    fn dispatch_http_request<'a>(&'a self, _input: &'a Value) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        _input: &'a Value,
        _expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        _input: &'a Value,
        _expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value> {
        unsupported_capability()
    }
}

struct HarnessOutbound {
    cancellation: CancellationToken,
}

impl OutboundServiceApi for HarnessOutbound {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        &[]
    }

    fn test_effects_enabled(&self) -> bool {
        false
    }

    fn test_effect_doubles(
        &self,
    ) -> HashMap<String, Vec<skiff_runtime_capability_context::RequestEffectDoubleControl>> {
        HashMap::new()
    }

    fn request_heap(&self) -> RequestHeap {
        RequestHeap::default()
    }

    fn effective_timeout_ms(&self, operation_timeout_ms: Option<u64>) -> Option<u64> {
        operation_timeout_ms
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        RuntimeError::Unsupported("combined outbound capability is unavailable".to_string())
    }

    fn start_request(
        &self,
        _start: skiff_runtime_capability_context::OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> EvalResult<skiff_runtime_capability_context::OutboundStartedRequest> {
        Err(RuntimeError::Unsupported(
            "combined outbound capability is unavailable".to_string(),
        ))
    }

    fn receive_response<'a>(
        &'a self,
        _lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        _target: &'a str,
        _receiver: &'a mut skiff_runtime_capability_context::OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> EvalCapabilityFuture<'a, skiff_runtime_capability_context::OutboundResponse> {
        Box::pin(async {
            Err(RuntimeError::Unsupported(
                "combined outbound capability is unavailable".to_string(),
            ))
        })
    }

    fn cancel_signal(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

fn execution_context(
    interpreter: &Interpreter,
    control: HarnessControl,
    config: HarnessConfig,
) -> ProgramExecutionContext<'static> {
    let execution = ExecutionControl::new(control);
    let effects = EffectDispatchContext::new(HarnessEffects);
    let actor = ActorCapabilityContext::new(HarnessActor);
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: ConfigCapabilityContext::new(config),
        db: DbCapabilityContext::unavailable(),
        file: FileCapabilityContext::new(HarnessFile),
        file_source_stream: FileSourceStreamContext::from_api(HarnessFileSourceStream {
            stream_runtime: interpreter.stream_runtime.clone(),
        }),
        time: TimeCapabilityContext::new(execution),
        websocket: EvalWebsocketCapabilityContext::new(HarnessWebsocket),
        effects: effects.clone(),
        http_client: effects.http_client_context(
            interpreter.http_options.clone(),
            interpreter.stream_runtime.clone(),
            interpreter.test_effect_double_context(),
        ),
        test_effect_doubles: interpreter.test_effect_double_context(),
        runtime_activation: Arc::new(RuntimeActivation {
            service: ServiceMeta {
                id: SERVICE_ID.to_string(),
                display_name: None,
                metadata: BTreeMap::new(),
            },
            version: VERSION.to_string(),
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
        outbound: OutboundServiceContext::new(HarnessOutbound {
            cancellation: CancellationToken::new(),
        }),
        request_heap_limits: RequestHeapLimits::default(),
    })
}

fn interpreter_for(file: Arc<LinkedFileUnit>) -> (Arc<Interpreter>, ProbeStreamRuntime) {
    let duration = anonymous_type_decl(
        "std.time.Duration",
        LinkedTypeDescriptor::Alias { target: integer() },
    );
    let std_file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-combined-std".to_string(),
        source_ast_hash: "source:f445h-e4r-combined-std".to_string(),
        module_path: "std".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: vec![duration.clone()],
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
    let duration_addr = TypeAddr {
        unit: UnitAddr::Package(0),
        file: FileAddr::LoadedFileIndex(0),
        type_index: 0,
    };
    let mut types = RuntimeTypeContext::default();
    types.descriptors.insert(duration_addr.clone(), duration);
    types
        .exported_types
        .insert_package(PackageSymbolKey::new(0, "std.time.Duration"), duration_addr);
    let program = Arc::new(EvalRuntimeProgram {
        service_id: SERVICE_ID.to_string(),
        service_files: vec![file],
        packages: vec![Arc::new(PackageUnit::empty(
            "skiff.run/std",
            VERSION,
            "skiff.run/std:build:f445h-e4r-combined",
            "skiff.run/std:abi:f445h-e4r-combined",
        ))],
        package_files: vec![vec![std_file]],
        service_resources: PublicationResourceTable::default(),
        package_resources: vec![PublicationResourceTable::default()],
        spawn_routes: HashMap::new(),
        link_overlay: overlay,
        types,
    });
    let stream = ProbeStreamRuntime::default();
    let interpreter = Interpreter::with_program(
        program,
        EvalRuntimeFactory::new(HarnessRuntimeFactory {
            stream: stream.clone(),
        }),
    );
    (Arc::new(interpreter), stream)
}

fn actor_owner() -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(ACTOR_FILE_ID.to_string()),
        actor_symbol: "CombinedActor".to_string(),
    }
}

fn actor_abi() -> ActorAbiIdentity {
    ActorAbiIdentity::new("skiff-actor-abi-v1:sha256:f445h-e4r-combined")
}

fn actor_implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new("skiff-actor-implementation-v1:sha256:f445h-e4r-combined")
}

fn method_identity(name: &str) -> ActorMethodIdentity {
    ActorMethodIdentity::new(format!("skiff-actor-method-v1:sha256:f445h-e4r-{name}"))
}

fn ready_pending_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.readyPending",
        vec![
            number(0),
            native_sleep_call(0),
            number(20),
            native_sleep_call(2),
            number(11),
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 1 },
            },
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 3 },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 4 }),
            },
        ],
        Vec::new(),
    )
}

fn timeout_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.timeout",
        vec![
            number(1),
            LinkedExprIr::Timeout {
                duration_ms: 1_000,
                value: ExprRefIr { expression: 0 },
                site: site(),
            },
        ],
        vec![
            LinkedStmtIr::Timeout {
                duration_ms: 1_000,
                body: "timeout_body".to_string(),
                site: site(),
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            },
        ],
        vec![BlockIr {
            label: "timeout_body".to_string(),
            statements: Vec::new(),
        }],
    )
}

fn concurrent_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.concurrent",
        vec![
            number(2),
            LinkedExprIr::ConcurrentValue {
                plan: LinkedConcurrentPlanIr {
                    lanes: vec![LinkedConcurrentLaneIr::Tail {
                        source_order: 0,
                        dependencies: Vec::new(),
                        tail: ExprRefIr { expression: 0 },
                        site: site(),
                    }],
                    site: site(),
                },
            },
        ],
        vec![
            LinkedStmtIr::Concurrent {
                plan: LinkedConcurrentPlanIr {
                    lanes: vec![LinkedConcurrentLaneIr::Serial {
                        source_order: 0,
                        dependencies: Vec::new(),
                        body: "serial_body".to_string(),
                        site: site(),
                    }],
                    site: site(),
                },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 1 }),
            },
        ],
        vec![BlockIr {
            label: "serial_body".to_string(),
            statements: Vec::new(),
        }],
    )
}

fn activation_executable(instruction: ActivationRelativeServiceCall) -> LinkedExecutable {
    let mut config_call = call(
        LinkedCallTarget::Builtin {
            op: "config.require".to_string(),
        },
        &[0],
    );
    config_call
        .type_args
        .insert("T0".to_string(), string_type());
    executable(
        "CombinedActor.activation",
        vec![
            LinkedExprIr::Literal {
                value: LiteralIr::String {
                    value: "barrier".to_string(),
                },
            },
            LinkedExprIr::Call { call: config_call },
            LinkedExprIr::Call {
                call: call(
                    LinkedCallTarget::ActivationRelativeService { instruction },
                    &[],
                ),
            },
        ],
        vec![
            LinkedStmtIr::Expr {
                value: ExprRefIr { expression: 1 },
            },
            LinkedStmtIr::Return {
                value: Some(ExprRefIr { expression: 2 }),
            },
        ],
        Vec::new(),
    )
}

fn competitor_executable() -> LinkedExecutable {
    executable(
        "CombinedActor.competitor",
        vec![number(3)],
        vec![LinkedStmtIr::Return {
            value: Some(ExprRefIr { expression: 0 }),
        }],
        Vec::new(),
    )
}

fn executable(
    symbol: &str,
    expressions: Vec<LinkedExprIr>,
    statements: Vec<LinkedStmtIr>,
    mut extra_blocks: Vec<BlockIr>,
) -> LinkedExecutable {
    let mut blocks = vec![BlockIr {
        label: "entry".to_string(),
        statements: (0..statements.len())
            .map(|statement| StmtRefIr {
                statement: statement as u32,
            })
            .collect(),
    }];
    blocks.append(&mut extra_blocks);
    LinkedExecutable {
        kind: ExecutableKind::ImplMethod,
        symbol: symbol.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: Some(integer()),
        self_type: None,
        slots: SlotLayoutIr::default(),
        may_suspend: true,
        body: LinkedExecutableBody {
            blocks,
            statements,
            expressions,
        },
    }
}

fn actor_file(include_activation: bool) -> Arc<LinkedFileUnit> {
    let mut names = vec!["readyPending", "timeout", "concurrent"];
    let mut executables = vec![
        ready_pending_executable(),
        timeout_executable(),
        concurrent_executable(),
    ];
    if include_activation {
        names.extend(["activation", "competitor"]);
        executables.push(activation_executable(linked_activation_instruction()));
        executables.push(competitor_executable());
    }
    let public_methods = names
        .iter()
        .enumerate()
        .map(|(index, name)| LinkedActorPublicMethod {
            method_identity: method_identity(name),
            name: (*name).to_string(),
            parameters: Vec::<LinkedFunctionTypeParamIr>::new(),
            return_type: integer(),
            may_suspend: true,
            implementation: LinkedActorMethodImplementation::LocalExecutable {
                executable_index: index as u32,
            },
        })
        .collect();
    Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: ACTOR_FILE_ID.to_string(),
        source_ast_hash: "source:f445h-e4r-combined-actor".to_string(),
        module_path: "combined".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: vec![LinkedActorDeclaration {
            actor_type: skiff_runtime_linked_program::ServiceSymbolRef {
                module_path: "combined".to_string(),
                symbol: "CombinedActor".to_string(),
            },
            implementation_owner: Some(actor_owner()),
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            actor_name: "CombinedActor".to_string(),
            actor_id_type: string_type(),
            fields: Vec::new(),
            public_methods,
            actor_runtime_abi_version: ACTOR_RUNTIME_ABI_VERSION_V1.to_string(),
        }],
        types: Vec::new(),
        constants: Vec::new(),
        executables,
        external_refs: ExternalRefTable::default(),
    })
}

struct ActorHarness {
    interpreter: Arc<Interpreter>,
    store: ActorInstanceStore,
    handle: ActorInstanceHandle,
}

impl ActorHarness {
    fn new(include_activation: bool) -> Self {
        let file = actor_file(include_activation);
        let (interpreter, _) = interpreter_for(Arc::clone(&file));
        let store = ActorInstanceStore::new();
        let actor_id = br#""combined-actor""#.to_vec();
        let fence = ActorInstanceFence {
            incarnation: ActorIncarnationKey {
                logical_key: ActorLogicalKey {
                    service_id: SERVICE_ID.to_string(),
                    actor_type_identity: "combined.CombinedActor".to_string(),
                    actor_id_type_identity: "builtin:string".to_string(),
                    actor_id_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.to_string(),
                    actor_id_hash: format!("sha256:{}", hex::encode(Sha256::digest(&actor_id))),
                    canonical_actor_id_key_bytes: actor_id,
                },
                epoch: 1,
            },
            actor_abi_identity: actor_abi(),
            actor_implementation_identity: actor_implementation(),
            declaration_owner: actor_owner(),
        };
        let program = EvalRuntimeProgram {
            service_id: SERVICE_ID.to_string(),
            service_files: vec![file],
            packages: Vec::new(),
            package_files: Vec::new(),
            service_resources: PublicationResourceTable::default(),
            package_resources: Vec::new(),
            spawn_routes: HashMap::new(),
            link_overlay: LinkOverlay::default(),
            types: RuntimeTypeContext::default(),
        };
        let handle = store
            .activate(ActorActivationRequest {
                fence,
                bootstrap_encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1,
                bootstrap_payload: b"{}",
                program: program.projection().type_view(),
            })
            .expect("combined Actor activation");
        Self {
            interpreter,
            store,
            handle,
        }
    }

    async fn execute(
        &self,
        method: &str,
        control: HarnessControl,
        config: HarnessConfig,
    ) -> std::result::Result<Vec<u8>, String> {
        ActorMethodExecutor::new(&self.store)
            .execute(
                &self.interpreter,
                ActorMethodExecutionRequest {
                    instance: &self.handle,
                    method_identity: &method_identity(method),
                    arguments_payload: b"[]",
                    context: execution_context(&self.interpreter, control, config),
                },
            )
            .await
            .map_err(|error| error.to_string())
    }
}

static ACTIVATION_INSTRUCTION: OnceLock<ActivationRelativeServiceCall> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn linked_activation_instruction() -> ActivationRelativeServiceCall {
    ACTIVATION_INSTRUCTION
        .get_or_init(compile_activation_instruction)
        .clone()
}

fn compile_activation_instruction() -> ActivationRelativeServiceCall {
    const PROVIDER_PACKAGE: &str = "example.com/f445h-e4r-provider";
    const PROVIDER_SERVICE: &str = "example.com/f445h-e4r-provider";
    const CONSUMER_PACKAGE: &str = "example.com/f445h-e4r-consumer-package";
    const CONSUMER_SERVICE: &str = "example.com/f445h-e4r-consumer";

    let temp = TempFixture::new("f445h-e4r-activation-link");
    let artifact_root = temp.child("artifacts");
    let platform = repository_platform_sources();
    seed_canonical_std(&platform, &artifact_root).expect("combined canonical std seed");

    let provider_root = temp.child("provider");
    write_service_source(
        &provider_root,
        PROVIDER_PACKAGE,
        PROVIDER_SERVICE,
        "",
        "ready: main.ready\n",
        "serviceCalls:\n  - ready\n",
        "function ready() -> integer { return 7 }\n",
    );
    let provider = build_service(&platform, &provider_root, &artifact_root);

    let consumer_root = temp.child("consumer");
    write_service_source(
        &consumer_root,
        CONSUMER_PACKAGE,
        CONSUMER_SERVICE,
        &format!(
            "services:\n  - id: {PROVIDER_SERVICE}\n    version: {VERSION}\n    alias: provider\n"
        ),
        "{}\n",
        "serviceCalls: []\n",
        "function callProvider() -> integer { return provider/ready() }\n",
    );
    let consumer = build_service(&platform, &consumer_root, &artifact_root);

    let store = CanonicalArtifactStore::open(&artifact_root).expect("combined artifact store");
    let deployments = [&provider.deployment, &consumer.deployment]
        .iter()
        .map(|reference| {
            store
                .read_service_deployment(reference)
                .expect("combined service deployment")
        })
        .collect::<Vec<_>>();
    let contracts = [&provider.contract, &consumer.contract]
        .iter()
        .map(|reference| {
            store
                .read_service_contract(reference)
                .expect("combined service contract")
        })
        .collect::<Vec<_>>();
    let mut package_refs = BTreeMap::from([
        (
            provider.package.package_build_id.clone(),
            provider.package.clone(),
        ),
        (
            consumer.package.package_build_id.clone(),
            consumer.package.clone(),
        ),
    ]);
    for deployment in &deployments {
        for binding in &deployment.package_bindings {
            package_refs.insert(
                binding.package.package_build_id.clone(),
                binding.package.clone(),
            );
        }
    }
    let packages = package_refs
        .values()
        .map(|reference| {
            store
                .read_package_artifact(reference)
                .expect("combined package closure")
        })
        .collect::<Vec<_>>();
    let roots = vec![
        service_deployment_ref(&deployments[0]),
        service_deployment_ref(&deployments[1]),
    ];
    let deployment_values = deployments
        .iter()
        .map(|deployment| deployment.as_ref().clone())
        .collect::<Vec<_>>();
    let contract_values = contracts
        .iter()
        .map(|contract| contract.as_ref().clone())
        .collect::<Vec<_>>();
    let package_values = packages
        .iter()
        .map(|package| package.as_ref().clone())
        .collect::<Vec<_>>();
    let assembly = resolve_runtime_assembly(
        &roots,
        &deployment_values,
        &contract_values,
        &package_values,
    )
    .expect("combined runtime assembly");
    let hydrated = assembly
        .package_link_plan
        .code_slots
        .iter()
        .map(|slot| hydrate_package(&store, &slot.package))
        .collect::<Vec<_>>();
    let image =
        skiff_runtime_linker::link_package_fixture_from_runtime_assembly(&assembly, hydrated)
            .expect("combined linked execution image");
    let code = image
        .code_by_build(&consumer.package.package_build_id)
        .expect("combined consumer linked code");
    let mut instructions = code
        .files()
        .iter()
        .flat_map(|file| &file.executables)
        .flat_map(|executable| &executable.body.expressions)
        .filter_map(|expression| match expression {
            LinkedExprIr::Call {
                call:
                    CallIr {
                        target: LinkedCallTarget::ActivationRelativeService { instruction },
                        ..
                    },
            } => Some(instruction.clone()),
            _ => None,
        });
    let instruction = instructions
        .next()
        .expect("combined consumer activation-relative instruction");
    assert!(
        instructions.next().is_none(),
        "combined consumer fixture has one activation-relative instruction"
    );
    instruction
}

struct BuiltService {
    package: skiff_artifact_model::PackageArtifactRef,
    deployment: ServiceDeploymentRef,
    contract: ServiceContractRef,
}

fn build_service(
    platform: &CompilerPlatformSources,
    root: &Path,
    artifact_root: &Path,
) -> BuiltService {
    let output = build_authoring_object(
        platform,
        AuthoringObject::Package,
        root,
        artifact_root,
        "dev",
        true,
    )
    .expect("combined service authoring");
    BuiltService {
        package: serde_json::from_value(output["packageArtifactReceipt"]["artifact"].clone())
            .expect("combined package artifact ref"),
        deployment: serde_json::from_value(
            output["serviceDeploymentReceipt"]["deployment"].clone(),
        )
        .expect("combined service deployment ref"),
        contract: serde_json::from_value(output["serviceContractReceipt"]["contract"].clone())
            .expect("combined service contract ref"),
    }
}

fn write_service_source(
    root: &Path,
    package_id: &str,
    service_id: &str,
    dependency_yaml: &str,
    api: &str,
    service_calls: &str,
    source: &str,
) {
    fs::create_dir_all(root).expect("combined service source directory");
    fs::write(
        root.join("package.yml"),
        format!("id: {package_id}\nversion: {VERSION}\n{dependency_yaml}"),
    )
    .expect("combined package manifest");
    fs::write(root.join("api.yml"), api).expect("combined API manifest");
    fs::write(
        root.join("service.yml"),
        format!("id: {service_id}\n{service_calls}"),
    )
    .expect("combined service manifest");
    fs::write(
        root.join("config.dev.yml"),
        "timeout: 1000\nquota: { cpuMillis: 100, memoryBytes: 1048576 }\nprincipal: service:f445h-e4r\nlifecycle: { maxConcurrency: 1 }\n",
    )
    .expect("combined service config");
    fs::write(root.join("main.skiff"), source).expect("combined Skiff source");
}

fn hydrate_package(
    store: &CanonicalArtifactStore,
    reference: &skiff_artifact_model::PackageArtifactRef,
) -> HydratedPackageCode {
    let artifact = store
        .read_package_artifact(reference)
        .expect("combined package artifact");
    let files = artifact
        .files
        .iter()
        .map(|file| {
            store
                .read_file_ir(reference, file)
                .expect("combined File IR")
        })
        .collect::<Vec<_>>();
    let schema_index = Arc::new(PackageSchemaIndex {
        package_id: artifact.package_schema_index.package_id.clone(),
        package_schema_index_identity: artifact
            .package_schema_index
            .package_schema_index_identity
            .clone(),
        types: BTreeMap::new(),
    });
    HydratedPackageCode::new(artifact, files, PublicationResourceTable::default())
        .with_schema_index(schema_index)
}

fn service_deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

fn repository_platform_sources() -> CompilerPlatformSources {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/eval lives below repository root")
        .to_path_buf();
    CompilerPlatformSources::new(&root).expect("combined compiler platform sources")
}

struct TempFixture {
    root: PathBuf,
}

impl TempFixture {
    fn new(name: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("combined test clock after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "skiff-runtime-eval-{name}-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("combined temp fixture root");
        Self { root }
    }

    fn child(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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
async fn f445h_e4r_combined_r1_actual_pending_ready_pending_and_checkpoint_stay_runnable() {
    let harness = ActorHarness::new(false);
    let control = HarnessControl::request();
    let units = Arc::clone(&control.instruction_units);
    let result = tokio::time::timeout(
        Duration::from_secs(2),
        harness.execute("readyPending", control, HarnessConfig::ordinary()),
    )
    .await
    .expect("R1 Ready/Pending combined evaluator completes")
    .expect("R1 Ready/Pending combined evaluator remains successful");

    assert_eq!(result, b"11");
    assert!(
        units.load(Ordering::Acquire) >= 8,
        "R1 combined surface must cross real evaluator checkpoints"
    );
}

#[tokio::test]
async fn f445h_e4r_combined_r2_timeout_statement_and_expression_execute() {
    let harness = ActorHarness::new(false);
    let result = harness
        .execute(
            "timeout",
            HarnessControl::request(),
            HarnessConfig::ordinary(),
        )
        .await;
    let actual = result
        .as_ref()
        .err()
        .cloned()
        .unwrap_or_else(|| format!("success payload {:?}", result.as_ref().ok()));
    assert!(
        result.is_ok(),
        "R2 expected timeout statement + expression success; production returned {actual}"
    );
    assert_eq!(result.expect("R2 timeout success"), b"1");
}

#[tokio::test]
async fn f445h_e4r_combined_r3_concurrent_statement_value_and_actor_execute() {
    let harness = ActorHarness::new(false);
    let result = harness
        .execute(
            "concurrent",
            HarnessControl::request(),
            HarnessConfig::ordinary(),
        )
        .await;
    let actual = result
        .as_ref()
        .err()
        .cloned()
        .unwrap_or_else(|| format!("success payload {:?}", result.as_ref().ok()));
    assert!(
        result.is_ok(),
        "R3 expected concurrent statement + value inside a real Actor frame; production returned {actual}"
    );
    assert_eq!(result.expect("R3 concurrent success"), b"2");
}

#[test]
fn f445h_e4r_combined_r4_activation_ready_error_keeps_actor_segment() {
    let harness = ActorHarness::new(true);
    let blocking = Arc::new(BlockingConfigState::default());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (activation_tx, activation_rx) = mpsc::channel();
    let activation_interpreter = Arc::clone(&harness.interpreter);
    let activation_store = harness.store.clone();
    let activation_handle = harness.handle.clone();
    let activation_blocking = Arc::clone(&blocking);
    let activation_thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("combined activation runtime");
        let result = runtime.block_on(async {
            ActorMethodExecutor::new(&activation_store)
                .execute(
                    &activation_interpreter,
                    ActorMethodExecutionRequest {
                        instance: &activation_handle,
                        method_identity: &method_identity("activation"),
                        arguments_payload: b"[]",
                        context: execution_context(
                            &activation_interpreter,
                            HarnessControl::request(),
                            HarnessConfig::blocking(entered_tx, activation_blocking),
                        ),
                    },
                )
                .await
                .map_err(|error| error.to_string())
        });
        let _ = activation_tx.send(result);
    });

    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("activation method entered while owning its Actor segment");

    let competitor_executor = ActorMethodExecutor::new(&harness.store);
    let competitor_method = method_identity("competitor");
    let mut competitor = Box::pin(competitor_executor.execute(
        &harness.interpreter,
        ActorMethodExecutionRequest {
            instance: &harness.handle,
            method_identity: &competitor_method,
            arguments_payload: b"[]",
            context: execution_context(
                &harness.interpreter,
                HarnessControl::request(),
                HarnessConfig::ordinary(),
            ),
        },
    ));
    assert!(
        matches!(first_poll(competitor.as_mut()), Poll::Pending),
        "competitor queues behind the activation method's held Actor segment"
    );

    {
        let mut released = blocking
            .released
            .lock()
            .expect("combined activation release lock");
        *released = true;
        blocking.release.notify_all();
    }
    let activation_before_competitor = activation_rx.recv_timeout(Duration::from_millis(250)).ok();
    let activation_completed_first = activation_before_competitor.is_some();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("combined competitor runtime");
    let competitor_result = runtime
        .block_on(async { tokio::time::timeout(Duration::from_secs(1), &mut competitor).await })
        .expect("competitor completes after the activation segment closes")
        .expect("competitor evaluator succeeds");
    assert_eq!(competitor_result, b"3");
    let activation_result = activation_before_competitor.unwrap_or_else(|| {
        activation_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("activation method completes after competitor")
    });
    activation_thread
        .join()
        .expect("combined activation thread joins");
    assert!(
        activation_result
            .as_ref()
            .is_err_and(|error| error.contains("no runtime assembly target")),
        "activation probe intentionally uses a first-Ready fail-closed production operation: {activation_result:?}"
    );
    assert!(
        activation_completed_first,
        "R4 expected first-Ready activation failure to retain the Actor segment; R1 pre-suspend let the queued competitor run first"
    );
}

#[tokio::test]
async fn f445h_e4r_combined_r4_stream_observes_child_scope_and_cleans_non_end() {
    let file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:f445h-e4r-combined-stream".to_string(),
        source_ast_hash: "source:f445h-e4r-combined-stream".to_string(),
        module_path: "combined.stream".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "combined.stream.consume".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr {
                slots: vec![SlotIr {
                    index: 0,
                    name: "item".to_string(),
                    kind: "local".to_string(),
                }],
                frame_size: 1,
            },
            may_suspend: true,
            body: LinkedExecutableBody {
                blocks: vec![BlockIr {
                    label: "body".to_string(),
                    statements: Vec::new(),
                }],
                statements: Vec::new(),
                expressions: Vec::new(),
            },
        }],
        external_refs: ExternalRefTable::default(),
    });
    let (interpreter, stream) = interpreter_for(Arc::clone(&file));
    let deadline = Instant::now() + Duration::from_secs(5);
    let (control, child_scope) = HarnessControl::child(deadline);
    let context = execution_context(&interpreter, control, HarnessConfig::ordinary());
    let addr = ExecutableAddr {
        unit: UnitAddr::Service,
        file: FileAddr::FileIrIdentity(file.file_ir_identity.clone()),
        executable: 0,
    };
    let mut heap = RequestHeap::default();
    let mut env = Env::new();
    let stream_value = stream_value("f445h-e4r-combined-pending-stream");
    let mut execution = Box::pin(interpreter.exec_program_stream_for_in(
        context,
        &mut heap,
        &mut env,
        &addr,
        &file,
        &file.executables[0],
        0,
        "body",
        stream_value,
        None,
        &[],
    ));

    assert!(
        matches!(first_poll(execution.as_mut()), Poll::Pending),
        "combined stream reaches the real pending next()"
    );
    assert!(matches!(
        child_scope.terminal_at(deadline),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    let outcome = tokio::time::timeout(Duration::from_millis(250), &mut execution).await;
    let completed_from_child_scope = outcome.is_ok();
    let terminal = outcome
        .as_ref()
        .ok()
        .map(|result| format!("{result:?}"))
        .unwrap_or_else(|| "harness timeout while stream next remained pending".to_string());
    drop(execution);
    assert_eq!(
        stream.state.cleanup_cancels.load(Ordering::Acquire),
        1,
        "non-End stream termination must run exactly one consumer cleanup"
    );
    assert!(
        completed_from_child_scope,
        "R4 expected current child scope to terminate pending next() before cleanup; {terminal}; next received {} cancellation token(s)",
        stream
            .state
            .last_cancel_token_count
            .load(Ordering::Acquire)
    );
}

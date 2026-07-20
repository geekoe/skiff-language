use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest, ActorPutControlRequest,
    ActorRemoveControlRequest, CancellationToken, CapabilityError, CapabilityFuture,
    ConfigCapabilityApi, ConfigCapabilityContext, DbCapabilityContext, ExecutionControl,
    ExecutionControlApi, ExecutionControlResult, FileCapabilityApi, FileCapabilityContext,
    FileCapabilityFuture, FileCapabilitySource, FileCapabilitySourceApi, FileChunkSource,
    FileSourceStreamApi, FileSourceStreamContext, HttpCapabilityFuture, HttpClientCapabilityApi,
    HttpClientCapabilityContext, OwnedActorCapabilityContext, OwnedConfigCapabilityContext,
    OwnedExecutionControl, OwnedExecutionControlApi, OwnedWebsocketCapabilityContext,
    SpawnSubmitControlRequest, StreamCancelSignal, StreamLifetimeGuard, StreamPoll,
    StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeResult, StreamSink,
    TelemetryCapabilityApi, TelemetryCapabilityContext, WebsocketCapabilityApi,
    WebsocketCapabilityContext,
};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    request_heap::RequestHeap,
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::RuntimeTypePlan,
};

use crate::{
    capabilities::{
        EffectDispatchApi, EffectDispatchContext, EvalRuntimeFactory, EvalRuntimeFactoryApi,
        HttpRuntimeOptions, OutboundServiceApi, OutboundServiceContext, TestEffectDouble,
        TestEffectDoubleContext, TestEffectDoubleContextApi,
    },
    error::{Result, RuntimeError},
};

pub(super) fn runtime_factory() -> EvalRuntimeFactory {
    EvalRuntimeFactory::new(TestRuntimeFactory)
}

pub(super) fn execution_control() -> ExecutionControl<'static> {
    ExecutionControl::new(TestExecutionControl::default())
}

pub(super) fn config_context() -> ConfigCapabilityContext<'static> {
    ConfigCapabilityContext::new(TestConfig)
}

pub(super) fn file_context() -> FileCapabilityContext {
    FileCapabilityContext::new(TestFile)
}

pub(super) fn file_source_stream_context(
    stream_runtime: StreamRuntime,
) -> FileSourceStreamContext<'static> {
    FileSourceStreamContext::from_api(TestFileSourceStream { stream_runtime })
}

pub(super) fn websocket_context() -> WebsocketCapabilityContext<'static> {
    WebsocketCapabilityContext::new(TestWebsocket)
}

pub(super) fn actor_context() -> ActorCapabilityContext<'static> {
    ActorCapabilityContext::new(TestActor)
}

pub(super) fn effects_context() -> EffectDispatchContext {
    EffectDispatchContext::new(TestEffects)
}

pub(super) fn outbound_context() -> OutboundServiceContext {
    OutboundServiceContext::new(TestOutbound {
        cancellation: CancellationToken::new(),
    })
}

#[derive(Debug)]
struct TestRuntimeFactory;

impl EvalRuntimeFactoryApi for TestRuntimeFactory {
    fn stream_runtime(&self) -> StreamRuntime {
        StreamRuntime::new(TestStreamRuntime)
    }

    fn reusable_test_effect_doubles(
        &self,
        _doubles: HashMap<String, TestEffectDouble>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(TestEffectDoubles)
    }

    fn one_shot_test_effect_double_sequences(
        &self,
        _doubles: HashMap<String, Vec<TestEffectDouble>>,
        _stream_runtime: &StreamRuntime,
        _test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        TestEffectDoubleContext::new(TestEffectDoubles)
    }
}

#[derive(Debug)]
struct TestStreamRuntime;

impl StreamRuntimeApi for TestStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        panic!("ordinary package-direct test does not create streams")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        panic!("ordinary package-direct test does not create streams")
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        panic!("ordinary package-direct test does not create streams")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        panic!("ordinary package-direct test does not create streams")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async { panic!("ordinary package-direct test does not poll streams") })
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async { panic!("ordinary package-direct test does not poll streams") })
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async { panic!("ordinary package-direct test does not poll streams") })
    }

    fn cancel(&self, _value: &Value) {}
}

#[derive(Clone)]
struct TestExecutionControl {
    cancelled: Arc<AtomicBool>,
    cancellation: CancellationToken,
}

impl Default for TestExecutionControl {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            cancellation: CancellationToken::new(),
        }
    }
}

impl ExecutionControlApi for TestExecutionControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for TestExecutionControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

#[derive(Clone)]
struct TestConfig;

impl ConfigCapabilityApi for TestConfig {
    fn owned(&self) -> OwnedConfigCapabilityContext {
        ConfigCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ConfigCapabilityContext<'_> {
        ConfigCapabilityContext::new(self.clone())
    }

    fn read_config_target(
        &self,
        _current_addr: &ExecutableAddr,
        _target: &str,
        _args: &[Value],
        _type_arg: Option<&RuntimeTypePlan>,
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        Err(CapabilityError::unsupported(
            "test config capability is unavailable",
        ))
    }
}

#[derive(Clone)]
struct TestFileSource;

impl FileCapabilitySourceApi for TestFileSource {
    fn context_for_request(&self, _db_context: DbCapabilityContext) -> FileCapabilityContext {
        file_context()
    }
}

#[derive(Clone)]
struct TestFile;

impl FileCapabilityApi for TestFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(TestFileSource)
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "test file capability is unavailable",
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
                "test file capability is unavailable",
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
                "test file capability is unavailable",
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
                "test file capability is unavailable",
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
                "test file capability is unavailable",
            ))
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        _next_chunk: FileChunkSource<'a>,
    ) -> FileCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "test file capability is unavailable",
            ))
        })
    }
}

struct TestFileSourceStream {
    stream_runtime: StreamRuntime,
}

impl FileSourceStreamApi for TestFileSourceStream {
    fn stream_runtime_handle(&self) -> StreamRuntime {
        self.stream_runtime.clone()
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        _stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async {
            Err(skiff_runtime_capability_context::FileCapabilityError::file(
                "test file stream capability is unavailable",
            ))
        })
    }
}

#[derive(Clone)]
struct TestActor;

impl ActorCapabilityApi for TestActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "test-runtime"
    }
    fn service_id(&self) -> &str {
        "test-service"
    }
    fn service_version(&self) -> &str {
        "1.0.0"
    }
    fn request_id(&self) -> &str {
        "test-request"
    }
    fn request_target(&self) -> &str {
        "test-service"
    }
    fn request_build_id(&self) -> &str {
        "test-build"
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
    fn activation_identity(&self) -> Option<&str> {
        None
    }
    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn put_actor<'a>(
        &'a self,
        _request: ActorPutControlRequest,
        _object_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }

    fn submit_spawn<'a>(
        &'a self,
        _request: SpawnSubmitControlRequest,
        _args_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ()> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }
}

#[derive(Clone)]
struct TestWebsocket;

impl WebsocketCapabilityApi for TestWebsocket {
    fn owned(&self) -> OwnedWebsocketCapabilityContext {
        WebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> WebsocketCapabilityContext<'_> {
        WebsocketCapabilityContext::new(self.clone())
    }

    fn service_id(&self) -> &str {
        "test-service"
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
            "test websocket capability is unavailable",
        ))
    }
    fn send_connection_binary_to_business_identity(
        &self,
        _business_identity: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "test websocket capability is unavailable",
        ))
    }
    fn send_connection_text_to_connection(
        &self,
        _connection_id: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "test websocket capability is unavailable",
        ))
    }
    fn send_connection_binary_to_connection(
        &self,
        _connection_id: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported(
            "test websocket capability is unavailable",
        ))
    }
}

struct TestEffects;

impl EffectDispatchApi for TestEffects {
    fn telemetry_context(&self) -> TelemetryCapabilityContext {
        TelemetryCapabilityContext::new(TestTelemetry)
    }

    fn http_client_context(
        &self,
        _http_options: HttpRuntimeOptions,
        _stream_runtime: StreamRuntime,
        _test_effect_doubles: TestEffectDoubleContext,
    ) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(TestHttp)
    }
}

struct TestTelemetry;

impl TelemetryCapabilityApi for TestTelemetry {
    fn emit_native(
        &self,
        _target: &str,
        _args: &[Value],
    ) -> skiff_runtime_capability_context::CapabilityResult<Value> {
        Err(CapabilityError::unsupported(
            "test telemetry capability is unavailable",
        ))
    }
}

struct TestHttp;

impl HttpClientCapabilityApi for TestHttp {
    fn with_stream_runtime(&self, _stream_runtime: StreamRuntime) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(Self)
    }

    fn dispatch_http_request<'a>(&'a self, _input: &'a Value) -> HttpCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test http capability is unavailable",
            ))
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        _input: &'a Value,
        _expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test http capability is unavailable",
            ))
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        _input: &'a Value,
        _expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> HttpCapabilityFuture<'a, Value> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test http capability is unavailable",
            ))
        })
    }
}

struct TestEffectDoubles;

impl TestEffectDoubleContextApi for TestEffectDoubles {
    fn next_test_effect_double(&self, _target: &str) -> Option<TestEffectDouble> {
        None
    }
    fn dispatch_test_effect_double(
        &self,
        _target: &str,
        _input: Option<&Value>,
    ) -> Option<Result<Value>> {
        None
    }
    fn dispatch_test_stable_target_double(
        &self,
        _target: &str,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        None
    }
    fn dispatch_test_host_operation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        None
    }
    fn dispatch_test_http_effect_invocation_double(
        &self,
        _target: &str,
        _input: Option<&RuntimeValue>,
        _arg_plan: Option<&RuntimeTypePlan>,
        _return_plan: Option<&RuntimeTypePlan>,
        _heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        None
    }
}

struct TestOutbound {
    cancellation: CancellationToken,
}

impl OutboundServiceApi for TestOutbound {
    fn service_dependencies(&self) -> &[skiff_runtime_linked_program::ServiceDependencyConstraint] {
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
        RuntimeError::Unsupported("test outbound capability is unavailable".to_string())
    }
    fn start_request(
        &self,
        _start: skiff_runtime_capability_context::OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> Result<skiff_runtime_capability_context::OutboundStartedRequest> {
        Err(RuntimeError::Unsupported(
            "test outbound capability is unavailable".to_string(),
        ))
    }
    fn receive_response<'a>(
        &'a self,
        _lease: &'a skiff_runtime_capability_context::OutboundRequestLease,
        _target: &'a str,
        _receiver: &'a mut skiff_runtime_capability_context::OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> crate::capabilities::EvalCapabilityFuture<
        'a,
        skiff_runtime_capability_context::OutboundResponse,
    > {
        Box::pin(async {
            Err(RuntimeError::Unsupported(
                "test outbound capability is unavailable".to_string(),
            ))
        })
    }
    fn cancel_signal(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_boundary::stream::{stream_id, stream_value};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorRemoveControlRequest, ActorReplaceControlRequest,
    CancellationToken, CapabilityError, CapabilityFuture, ConfigCapabilityApi,
    ConfigCapabilityContext, DbCapabilityContext, ExecutionControl, ExecutionControlApi,
    ExecutionControlResult, FileCapabilityApi, FileCapabilityContext, FileCapabilityFuture,
    FileCapabilitySource, FileCapabilitySourceApi, FileChunkSource, FileSourceStreamApi,
    FileSourceStreamContext, HttpCapabilityFuture, HttpClientCapabilityApi,
    HttpClientCapabilityContext, OwnedActorCapabilityContext, OwnedConfigCapabilityContext,
    OwnedExecutionControl, OwnedExecutionControlApi, OwnedWebsocketCapabilityContext,
    SpawnSubmitControlRequest, StreamCancelSignal, StreamInternalItem, StreamLifetimeGuard,
    StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError,
    StreamRuntimeResult, StreamSink, StreamSinkApi, TelemetryCapabilityApi,
    TelemetryCapabilityContext, WebsocketCapabilityApi, WebsocketCapabilityContext,
};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    request_heap::RequestHeap,
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::RuntimeTypePlan,
};
use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};

use crate::{
    assembly_execution::service_error_channel::RecordingRestrictedServiceDiagnosticSink,
    capabilities::{
        EffectDispatchApi, EffectDispatchContext, EvalRuntimeFactory, EvalRuntimeFactoryApi,
        HttpRuntimeOptions, OutboundServiceApi, OutboundServiceContext, TestEffectDouble,
        TestEffectDoubleContext, TestEffectDoubleContextApi, WebsocketCapabilityRebinder,
    },
    error::{Result, RuntimeError},
};

pub(crate) fn runtime_factory() -> EvalRuntimeFactory {
    EvalRuntimeFactory::new(TestRuntimeFactory)
}

pub(crate) fn execution_control() -> ExecutionControl<'static> {
    ExecutionControl::new(TestExecutionControl::default())
}

pub(crate) fn execution_control_with_deadline(
    deadline: Option<Instant>,
) -> ExecutionControl<'static> {
    ExecutionControl::new(TestExecutionControl::with_deadline(deadline))
}

pub(crate) fn config_context() -> ConfigCapabilityContext<'static> {
    ConfigCapabilityContext::new(TestConfig)
}

pub(crate) fn file_context() -> FileCapabilityContext {
    FileCapabilityContext::new(TestFile)
}

pub(crate) fn file_source_stream_context(
    stream_runtime: StreamRuntime,
) -> FileSourceStreamContext<'static> {
    FileSourceStreamContext::from_api(TestFileSourceStream { stream_runtime })
}

pub(crate) fn websocket_context() -> WebsocketCapabilityContext<'static> {
    WebsocketCapabilityContext::new(TestWebsocket {
        service_id: "test-service".to_string(),
        websocket_entry_id: None,
    })
}

pub(crate) fn websocket_rebinder() -> WebsocketCapabilityRebinder {
    WebsocketCapabilityRebinder::new(|service_id, websocket_entry_id| {
        WebsocketCapabilityContext::new(TestWebsocket {
            service_id: service_id.to_string(),
            websocket_entry_id: websocket_entry_id.map(str::to_string),
        })
        .owned()
    })
}

pub(crate) fn actor_context() -> ActorCapabilityContext<'static> {
    ActorCapabilityContext::new(TestActor { trace_id: None })
}

pub(crate) fn actor_context_with_trace(trace_id: &'static str) -> ActorCapabilityContext<'static> {
    ActorCapabilityContext::new(TestActor {
        trace_id: Some(trace_id),
    })
}

pub(crate) fn effects_context() -> EffectDispatchContext {
    EffectDispatchContext::new(TestEffects)
}

pub(crate) fn outbound_context() -> OutboundServiceContext {
    OutboundServiceContext::new(TestOutbound {
        cancellation: CancellationToken::new(),
    })
}

#[derive(Debug)]
struct TestRuntimeFactory;

impl EvalRuntimeFactoryApi for TestRuntimeFactory {
    fn stream_runtime(&self) -> StreamRuntime {
        StreamRuntime::new(TestStreamRuntime::default())
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

#[derive(Debug, Default)]
struct TestStreamRuntime {
    next_id: AtomicU64,
    buffered: Mutex<HashMap<u64, VecDeque<Value>>>,
    channels: Mutex<HashMap<u64, Arc<TestStreamChannel>>>,
}

impl TestStreamRuntime {
    fn stream_id(value: &Value) -> StreamRuntimeResult<u64> {
        stream_id(value)
            .and_then(|id| id.parse().ok())
            .ok_or_else(|| {
                skiff_runtime_capability_context::StreamRuntimeError::decode(
                    "ordinary test stream handle is invalid",
                )
            })
    }

    async fn poll(&self, value: &Value) -> StreamRuntimeResult<StreamPoll> {
        let id = Self::stream_id(value)?;
        {
            let mut buffered = self.buffered.lock().expect("test stream mutex poisoned");
            if let Some(items) = buffered.get_mut(&id) {
                return match items.pop_front() {
                    Some(item) => Ok(StreamPoll::Item(item)),
                    None => {
                        buffered.remove(&id);
                        Ok(StreamPoll::End)
                    }
                };
            }
        }
        let channel = self
            .channels
            .lock()
            .expect("test stream mutex poisoned")
            .get(&id)
            .cloned();
        let Some(channel) = channel else {
            return Ok(StreamPoll::End);
        };
        let cancel_notified = channel.cancel_notify.notified();
        tokio::pin!(cancel_notified);
        cancel_notified.as_mut().enable();
        if channel.cancelled.load(Ordering::Acquire) {
            self.finish_channel(id, &channel);
            return Err(StreamRuntimeError::cancelled());
        }
        let event = {
            let mut receiver = channel.receiver.lock().await;
            tokio::select! {
                biased;
                _ = &mut cancel_notified => {
                    self.finish_channel(id, &channel);
                    return Err(StreamRuntimeError::cancelled());
                }
                event = receiver.recv() => event,
            }
        };
        match event {
            Some(TestStreamEvent::Item(item)) => Ok(StreamPoll::Item(item)),
            Some(TestStreamEvent::InternalItem(item)) => Ok(StreamPoll::InternalItem(item)),
            Some(TestStreamEvent::End) | None => {
                self.finish_channel(id, &channel);
                Ok(StreamPoll::End)
            }
            Some(TestStreamEvent::Fail(error)) => {
                self.finish_channel(id, &channel);
                Err(error)
            }
        }
    }

    fn finish_channel(&self, id: u64, channel: &TestStreamChannel) {
        self.channels
            .lock()
            .expect("test stream mutex poisoned")
            .remove(&id);
        channel
            .lifetime
            .lock()
            .expect("test stream lifetime mutex poisoned")
            .take();
    }

    fn create_channel(&self, lifetime: Option<StreamLifetimeGuard>) -> (Value, StreamSink) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancel_notify = Arc::new(Notify::new());
        self.channels
            .lock()
            .expect("test stream mutex poisoned")
            .insert(
                id,
                Arc::new(TestStreamChannel {
                    receiver: AsyncMutex::new(receiver),
                    cancelled: Arc::clone(&cancelled),
                    cancel_notify: Arc::clone(&cancel_notify),
                    lifetime: Mutex::new(lifetime),
                }),
            );
        (
            stream_value(&id.to_string()),
            StreamSink::new(TestStreamSink {
                id,
                sender,
                cancelled,
                cancel_notify,
            }),
        )
    }
}

#[derive(Debug)]
struct TestStreamChannel {
    receiver: AsyncMutex<mpsc::Receiver<TestStreamEvent>>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
    lifetime: Mutex<Option<StreamLifetimeGuard>>,
}

#[derive(Debug)]
enum TestStreamEvent {
    Item(Value),
    InternalItem(StreamInternalItem),
    End,
    Fail(StreamRuntimeError),
}

#[derive(Clone, Debug)]
struct TestStreamSink {
    id: u64,
    sender: mpsc::Sender<TestStreamEvent>,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

impl TestStreamSink {
    async fn send_event(&self, event: TestStreamEvent) -> StreamRuntimeResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(StreamRuntimeError::cancelled());
        }
        self.sender
            .send(event)
            .await
            .map_err(|_| StreamRuntimeError::decode("ordinary test stream receiver was dropped"))
    }
}

impl StreamSinkApi for TestStreamSink {
    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StreamRuntimeResult<()>> + Send + 'a>>
    {
        Box::pin(async move {
            if cancel_tokens.iter().any(CancellationToken::is_cancelled) {
                return Err(StreamRuntimeError::cancelled());
            }
            self.send_event(TestStreamEvent::InternalItem(item)).await
        })
    }

    fn send<'a>(
        &'a self,
        item: Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StreamRuntimeResult<()>> + Send + 'a>>
    {
        Box::pin(async move { self.send_event(TestStreamEvent::Item(item)).await })
    }

    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StreamRuntimeResult<()>> + Send + 'a>>
    {
        Box::pin(async move {
            if cancel_flags.iter().any(|flag| flag.load(Ordering::Acquire)) {
                return Err(StreamRuntimeError::cancelled());
            }
            self.send_event(TestStreamEvent::Item(item)).await
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        _signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = StreamRuntimeResult<()>> + Send + 'a>>
    {
        Box::pin(async move {
            if cancel_tokens.iter().any(CancellationToken::is_cancelled) {
                return Err(StreamRuntimeError::cancelled());
            }
            self.send_event(TestStreamEvent::Item(item)).await
        })
    }

    fn end<'a>(&'a self) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = self.send_event(TestStreamEvent::End).await;
        })
    }

    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            let _ = self.send_event(TestStreamEvent::Fail(error)).await;
        })
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other
            .downcast_ref::<Self>()
            .is_some_and(|other| self.id == other.id)
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(TestStreamCancelSignal {
            cancelled: Arc::clone(&self.cancelled),
            cancel_notify: Arc::clone(&self.cancel_notify),
        })
    }
}

#[derive(Debug)]
struct TestStreamCancelSignal {
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
}

impl skiff_runtime_capability_context::StreamCancelSignalApi for TestStreamCancelSignal {
    fn wait_cancelled<'a>(
        &'a self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            loop {
                if self.cancelled.load(Ordering::Acquire) {
                    return;
                }
                let notified = self.cancel_notify.notified();
                if self.cancelled.load(Ordering::Acquire) {
                    return;
                }
                notified.await;
            }
        })
    }
}

impl StreamRuntimeApi for TestStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        self.create_channel(None)
    }

    fn channel_stream_with_lifetime(&self, lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        self.create_channel(Some(lifetime))
    }

    fn pull_stream_with_cancellation(
        &self,
        _source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        panic!("ordinary package-direct test does not create streams")
    }

    fn buffered_stream(&self, items: Vec<Value>) -> Value {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.buffered
            .lock()
            .expect("test stream mutex poisoned")
            .insert(id, items.into());
        stream_value(&id.to_string())
    }

    fn next_with_cancel<'a>(
        &'a self,
        value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async move { self.poll(value).await })
    }

    fn next_with_cancellation<'a>(
        &'a self,
        value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async move { self.poll(value).await })
    }

    fn next<'a>(
        &'a self,
        value: &'a Value,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>,
    > {
        Box::pin(async move { self.poll(value).await })
    }

    fn cancel(&self, value: &Value) {
        let Ok(id) = Self::stream_id(value) else {
            return;
        };
        if let Some(channel) = self
            .channels
            .lock()
            .expect("test stream mutex poisoned")
            .remove(&id)
        {
            channel.cancelled.store(true, Ordering::Release);
            channel
                .lifetime
                .lock()
                .expect("test stream lifetime mutex poisoned")
                .take();
            channel.cancel_notify.notify_waiters();
        }
    }
}

#[derive(Clone)]
struct TestExecutionControl {
    cancelled: Arc<AtomicBool>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
}

impl Default for TestExecutionControl {
    fn default() -> Self {
        Self::with_deadline(None)
    }
}

impl TestExecutionControl {
    fn with_deadline(deadline: Option<Instant>) -> Self {
        let cancellation = CancellationToken::new();
        Self {
            cancelled: cancellation.cancel_flag(),
            cancellation,
            deadline,
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

    fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancellation.is_cancelled() {
            Err(skiff_runtime_capability_context::ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        self.check_cancelled()
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.check_cancelled()?;
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            Err(
                skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(
                    skiff_runtime_capability_context::ExecutionBudgetFailure {
                        reason:
                            skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
                        instruction_count: 0,
                        limit: None,
                        elapsed_ms: 0.0,
                    },
                ),
            )
        } else {
            Ok(())
        }
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

    fn deadline(&self) -> Option<Instant> {
        self.deadline
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
struct TestActor {
    trace_id: Option<&'static str>,
}

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
    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }
    fn trace_id(&self) -> Option<&str> {
        self.trace_id
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
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

    fn invoke_actor<'a>(
        &'a self,
        _request: skiff_runtime_capability_context::ActorInvocationRequest,
    ) -> CapabilityFuture<'a, skiff_runtime_capability_context::ActorInvocationOutcome> {
        Box::pin(async {
            Err(CapabilityError::unsupported(
                "test actor capability is unavailable",
            ))
        })
    }
}

#[derive(Clone)]
struct TestWebsocket {
    service_id: String,
    websocket_entry_id: Option<String>,
}

impl WebsocketCapabilityApi for TestWebsocket {
    fn owned(&self) -> OwnedWebsocketCapabilityContext {
        WebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> WebsocketCapabilityContext<'_> {
        WebsocketCapabilityContext::new(self.clone())
    }

    fn service_id(&self) -> &str {
        &self.service_id
    }
    fn websocket_entry_id(&self) -> Option<&str> {
        self.websocket_entry_id.as_deref()
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
            .with_restricted_service_diagnostic_sink(RecordingRestrictedServiceDiagnosticSink)
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

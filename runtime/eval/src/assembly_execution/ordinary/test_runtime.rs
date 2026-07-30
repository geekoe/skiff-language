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
    ExecutionControlResult, ExecutionScope, ExecutionScopeAccessError, FileCapabilityApi,
    FileCapabilityContext, FileCapabilityFuture, FileCapabilitySource, FileCapabilitySourceApi,
    FileChunkSource, FileSourceStreamApi, FileSourceStreamContext, HttpCapabilityFuture,
    HttpClientCapabilityApi, HttpClientCapabilityContext, OwnedActorCapabilityContext,
    OwnedConfigCapabilityContext, OwnedExecutionControl, OwnedExecutionControlApi,
    OwnedWebsocketCapabilityContext as SharedOwnedWebsocketCapabilityContext,
    SpawnSubmitControlRequest, StreamCancelSignal, StreamInternalItem, StreamLifetimeGuard,
    StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError,
    StreamRuntimeResult, StreamSink, StreamSinkApi, TelemetryCapabilityApi,
    TelemetryCapabilityContext, WebsocketCapabilityApi,
    WebsocketCapabilityContext as SharedWebsocketCapabilityContext,
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
        HttpRuntimeOptions, TestEffectDouble, TestEffectDoubleContext, TestEffectDoubleContextApi,
        WebsocketCapabilityContext,
    },
    error::{Result, RuntimeError},
    program_execution::{
        ActivationExecutionContextRebinder, ActivationExecutionOperation,
        OwnedActivationExecutionCapabilityBundle,
    },
    RuntimeAssemblyEvalTarget,
};

#[path = "test_runtime/scoped_execution.rs"]
mod scoped_execution;

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

pub(crate) fn activation_execution_context_rebinder(
    request_actor: &ActorCapabilityContext<'_>,
    stream_runtime: StreamRuntime,
    test_effect_doubles: TestEffectDoubleContext,
    http_options: HttpRuntimeOptions,
) -> Arc<dyn ActivationExecutionContextRebinder> {
    Arc::new(TestActivationExecutionContextRebinder {
        request_actor: request_actor.owned(),
        stream_runtime,
        test_effect_doubles,
        http_options,
    })
}

pub(crate) fn actor_context() -> ActorCapabilityContext<'static> {
    ActorCapabilityContext::new(TestActor::request(None))
}

pub(crate) fn actor_context_with_trace(trace_id: &'static str) -> ActorCapabilityContext<'static> {
    ActorCapabilityContext::new(TestActor::request(Some(trace_id.to_string())))
}

pub(crate) fn effects_context() -> EffectDispatchContext {
    EffectDispatchContext::new(TestEffects)
}

struct TestActivationExecutionContextRebinder {
    request_actor: OwnedActorCapabilityContext,
    stream_runtime: StreamRuntime,
    test_effect_doubles: TestEffectDoubleContext,
    http_options: HttpRuntimeOptions,
}

impl ActivationExecutionContextRebinder for TestActivationExecutionContextRebinder {
    fn rebind(
        &self,
        target: &RuntimeAssemblyEvalTarget,
        _operation: &ActivationExecutionOperation,
    ) -> Result<OwnedActivationExecutionCapabilityBundle> {
        let actor =
            ActorCapabilityContext::new(TestActor::for_activation(&self.request_actor, target))
                .owned();
        let service_id = target
            .activation_context()
            .identity()
            .deployment
            .service_id
            .as_str();
        let websocket_entry_id = target
            .activation_context()
            .websocket_entry_id()
            .map(|entry| entry.as_str());
        let websocket = WebsocketCapabilityContext::new(TestWebsocket {
            service_id: service_id.to_string(),
            websocket_entry_id: websocket_entry_id.map(str::to_string),
        })
        .owned();
        let effects = effects_context();
        let http_client = effects.http_client_context(
            self.http_options.clone(),
            self.stream_runtime.clone(),
            self.test_effect_doubles.clone(),
        );
        Ok(OwnedActivationExecutionCapabilityBundle::new(
            ConfigCapabilityContext::owned(&config_context()),
            DbCapabilityContext::unavailable(),
            FileCapabilitySource::new(TestFileSource),
            websocket,
            effects,
            http_client,
            actor.clone(),
            actor,
        ))
    }
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
    execution_scope: ExecutionScope,
}

impl Default for TestExecutionControl {
    fn default() -> Self {
        Self::with_deadline(None)
    }
}

impl TestExecutionControl {
    fn with_deadline(deadline: Option<Instant>) -> Self {
        let cancellation = CancellationToken::new();
        let execution_scope = scoped_execution::request_scope(cancellation.clone(), deadline);
        Self {
            cancelled: cancellation.cancel_flag(),
            cancellation,
            deadline,
            execution_scope,
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

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        scoped_execution::current_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        scoped_execution::derive_scope(self, local_deadline, site)
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

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: skiff_artifact_model::InstructionSourceSite,
    ) -> std::result::Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
    service_id: String,
    service_version: String,
    request_id: String,
    request_build_id: String,
    runtime_id: String,
    trace_id: Option<String>,
    activation_identity: Option<ActivationIdentityControl>,
}

impl TestActor {
    fn request(trace_id: Option<String>) -> Self {
        Self {
            service_id: "test-service".to_string(),
            service_version: "1.0.0".to_string(),
            request_id: "test-request".to_string(),
            request_build_id: "test-build".to_string(),
            runtime_id: "test-runtime".to_string(),
            trace_id,
            activation_identity: None,
        }
    }

    fn for_activation(
        request: &ActorCapabilityContext<'_>,
        target: &RuntimeAssemblyEvalTarget,
    ) -> Self {
        let activation = target.activation_context();
        let identity = activation.identity();
        Self {
            service_id: identity.deployment.service_id.clone(),
            service_version: identity.deployment.contract_version.clone(),
            request_id: request.request_id().to_string(),
            request_build_id: activation
                .implementation_package_build_id()
                .as_str()
                .to_string(),
            runtime_id: request.runtime_id().to_string(),
            trace_id: request.trace_id().map(str::to_string),
            activation_identity: Some(ActivationIdentityControl {
                assembly_identity: identity.assembly_identity.clone(),
                generation: identity.assembly_generation,
                runtime_replica_id: identity.runtime_replica_id.clone(),
                deployment_revision: identity.deployment.deployment_revision.clone(),
            }),
        }
    }
}

impl ActorCapabilityApi for TestActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        &self.runtime_id
    }
    fn service_id(&self) -> &str {
        &self.service_id
    }
    fn service_version(&self) -> &str {
        &self.service_version
    }
    fn request_id(&self) -> &str {
        &self.request_id
    }
    fn request_target(&self) -> &str {
        "test-service"
    }
    fn request_build_id(&self) -> &str {
        &self.request_build_id
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
        self.activation_identity.as_ref()
    }
    fn trace_id(&self) -> Option<&str> {
        self.trace_id.as_deref()
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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
    fn owned(&self) -> SharedOwnedWebsocketCapabilityContext {
        SharedWebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> SharedWebsocketCapabilityContext<'_> {
        SharedWebsocketCapabilityContext::new(self.clone())
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

    fn dispatch_http_request<'a>(
        &'a self,
        _input: &'a Value,
        _execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, Value> {
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
        _execution_control: OwnedExecutionControl,
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
        _execution_control: OwnedExecutionControl,
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

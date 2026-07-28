use std::{
    any::Any,
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::Value;
use skiff_artifact_model::{InstructionSourceSite, PackageBuildId};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    error::{RuntimeErrorPayload, WirePayload},
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    service_error::{CatchIdentity, ExceptionStackFrame, OpaqueServiceError},
    type_plan::RuntimeTypePlan,
};

use crate::{CancellationToken, ExecutionControl, OwnedExecutionControl};

pub type StreamRuntimeResult<T> = Result<T, StreamRuntimeError>;

/// Caller-side facts captured when an in-process service stream is created.
///
/// The fixed failure itself remains independent of either request heap. These
/// facts let eval invoke the canonical importer only when the terminal is
/// observed, without retaining the provider heap or inferring provenance from
/// a generic producer payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedServiceStreamImport {
    caller_package_build_id: PackageBuildId,
    caller_executable_addr: ExecutableAddr,
    call_site: InstructionSourceSite,
    caller_stack_at_site: Vec<ExceptionStackFrame>,
    remote_service_id: String,
    remote_operation_id: String,
}

impl FixedServiceStreamImport {
    pub fn new(
        caller_package_build_id: PackageBuildId,
        caller_executable_addr: ExecutableAddr,
        call_site: InstructionSourceSite,
        caller_stack_at_site: Vec<ExceptionStackFrame>,
        remote_service_id: String,
        remote_operation_id: String,
    ) -> Self {
        Self {
            caller_package_build_id,
            caller_executable_addr,
            call_site,
            caller_stack_at_site,
            remote_service_id,
            remote_operation_id,
        }
    }

    pub fn caller_package_build_id(&self) -> &PackageBuildId {
        &self.caller_package_build_id
    }

    pub fn caller_executable_addr(&self) -> &ExecutableAddr {
        &self.caller_executable_addr
    }

    pub fn call_site(&self) -> &InstructionSourceSite {
        &self.call_site
    }

    pub fn caller_stack_at_site(&self) -> &[ExceptionStackFrame] {
        &self.caller_stack_at_site
    }

    pub fn remote_service_id(&self) -> &str {
        &self.remote_service_id
    }

    pub fn remote_operation_id(&self) -> &str {
        &self.remote_operation_id
    }
}

/// Heap-independent, strict service failure carried by a stream terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedServiceStreamFailure {
    error: OpaqueServiceError,
    import: Option<FixedServiceStreamImport>,
}

impl FixedServiceStreamFailure {
    /// Creates a typed handoff without in-process import provenance, as used
    /// by the outbound capability seam.
    pub fn new(error: OpaqueServiceError) -> Self {
        Self {
            error,
            import: None,
        }
    }

    /// Creates an in-process terminal that can be imported in the consumer
    /// heap after the provider task and heap have been destroyed.
    pub fn with_import(error: OpaqueServiceError, import: FixedServiceStreamImport) -> Self {
        Self {
            error,
            import: Some(import),
        }
    }

    pub fn error(&self) -> &OpaqueServiceError {
        &self.error
    }

    pub fn import(&self) -> Option<&FixedServiceStreamImport> {
        self.import.as_ref()
    }
}

impl fmt::Display for FixedServiceStreamFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("canonical service stream failure")
    }
}

impl Error for FixedServiceStreamFailure {}

impl WirePayload for FixedServiceStreamFailure {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: "InternalError".to_string(),
            message: "canonical service stream failure".to_string(),
            status: None,
            details: None,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Borrowed typed branch of a producer terminal.
pub enum StreamProducerFailureRef<'a> {
    FixedService(&'a FixedServiceStreamFailure),
    Dynamic(&'a dyn WirePayload),
}

/// Producer-terminal carrier with a first-class fixed service branch.
///
/// Eval can inspect [`StreamProducerFailureRef`] directly; only the dynamic
/// branch exposes a generic wire payload.
pub trait StreamProducerFailure: WirePayload {
    fn failure_ref(&self) -> StreamProducerFailureRef<'_>;
}

impl StreamProducerFailure for FixedServiceStreamFailure {
    fn failure_ref(&self) -> StreamProducerFailureRef<'_> {
        StreamProducerFailureRef::FixedService(self)
    }
}

#[derive(Debug)]
struct DynamicStreamProducerFailure {
    error: Box<dyn WirePayload>,
}

impl DynamicStreamProducerFailure {
    fn new(error: Box<dyn WirePayload>) -> Self {
        Self { error }
    }
}

impl fmt::Display for DynamicStreamProducerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.error)
    }
}

impl Error for DynamicStreamProducerFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.error.as_ref())
    }
}

impl WirePayload for DynamicStreamProducerFailure {
    fn payload(&self) -> RuntimeErrorPayload {
        self.error.payload()
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.error.catch_projection()
    }

    fn as_any(&self) -> &dyn Any {
        self.error.as_any()
    }
}

impl StreamProducerFailure for DynamicStreamProducerFailure {
    fn failure_ref(&self) -> StreamProducerFailureRef<'_> {
        StreamProducerFailureRef::Dynamic(self.error.as_ref())
    }
}

#[derive(Debug)]
/// Stream cancellation is an internal terminal, not an ordinary producer error.
///
/// ```compile_fail
/// use skiff_runtime_capability_context::StreamRuntimeError;
/// use skiff_runtime_model::error::WirePayload;
///
/// let _ = WirePayload::payload(&StreamRuntimeError::cancelled());
/// ```
pub enum StreamRuntimeError {
    Decode(String),
    Cancelled,
    Producer(Box<dyn StreamProducerFailure>),
}

impl StreamRuntimeError {
    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn cancelled() -> Self {
        Self::Cancelled
    }

    pub fn producer(error: impl WirePayload) -> Self {
        Self::producer_boxed(Box::new(error))
    }

    pub fn producer_boxed(error: Box<dyn WirePayload>) -> Self {
        Self::Producer(Box::new(DynamicStreamProducerFailure::new(error)))
    }

    pub fn fixed_service_failure(error: OpaqueServiceError) -> Self {
        Self::Producer(Box::new(FixedServiceStreamFailure::new(error)))
    }

    pub fn fixed_service_failure_with_import(
        error: OpaqueServiceError,
        caller_package_build_id: PackageBuildId,
        caller_executable_addr: ExecutableAddr,
        call_site: InstructionSourceSite,
        caller_stack_at_site: Vec<ExceptionStackFrame>,
        remote_service_id: String,
        remote_operation_id: String,
    ) -> Self {
        Self::Producer(Box::new(FixedServiceStreamFailure::with_import(
            error,
            FixedServiceStreamImport::new(
                caller_package_build_id,
                caller_executable_addr,
                call_site,
                caller_stack_at_site,
                remote_service_id,
                remote_operation_id,
            ),
        )))
    }

    pub fn is_cancellation_terminal(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    pub fn ordinary_payload(&self) -> Option<RuntimeErrorPayload> {
        match self {
            Self::Decode(message) => Some(RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            }),
            Self::Cancelled => None,
            Self::Producer(error) => Some(error.payload()),
        }
    }

    pub fn ordinary_catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::Cancelled | Self::Decode(_) => None,
            Self::Producer(error) => error.catch_projection(),
        }
    }

    #[allow(clippy::type_complexity)]
    pub fn fixed_service_failure_parts(
        &self,
    ) -> Option<(
        &OpaqueServiceError,
        Option<(
            &PackageBuildId,
            &ExecutableAddr,
            &InstructionSourceSite,
            &[ExceptionStackFrame],
            &str,
            &str,
        )>,
    )> {
        let Self::Producer(error) = self else {
            return None;
        };
        let StreamProducerFailureRef::FixedService(failure) = error.failure_ref() else {
            return None;
        };
        Some((
            failure.error(),
            failure.import().map(|import| {
                (
                    import.caller_package_build_id(),
                    import.caller_executable_addr(),
                    import.call_site(),
                    import.caller_stack_at_site(),
                    import.remote_service_id(),
                    import.remote_operation_id(),
                )
            }),
        ))
    }
}

impl fmt::Display for StreamRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(message) => formatter.write_str(message),
            Self::Cancelled => formatter.write_str("request was cancelled"),
            Self::Producer(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for StreamRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Producer(error) => Some(error.as_ref()),
            Self::Decode(_) | Self::Cancelled => None,
        }
    }
}

pub trait StreamPullSource: Send {
    fn next<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>>;
}

#[derive(Debug)]
pub enum StreamPoll {
    Item(Value),
    InternalItem(StreamInternalItem),
    End,
}

/// An in-process stream item that cannot be represented by the wire JSON carrier. The heap owns
/// every handle reachable from `value`, so the item can cross the producer task boundary without
/// borrowing the provider request heap.
#[derive(Debug)]
pub struct StreamInternalItem {
    value: RuntimeValue,
    heap: RequestHeap,
}

impl StreamInternalItem {
    pub fn new(value: RuntimeValue, heap: RequestHeap) -> Self {
        Self { value, heap }
    }

    pub fn into_parts(self) -> (RuntimeValue, RequestHeap) {
        (self.value, self.heap)
    }
}

pub trait StreamCancelSignalApi: Any + Send + Sync + fmt::Debug {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

#[derive(Clone)]
pub struct StreamCancelSignal {
    inner: Arc<dyn StreamCancelSignalApi>,
}

impl StreamCancelSignal {
    pub fn new<T>(inner: T) -> Self
    where
        T: StreamCancelSignalApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
    }

    pub async fn wait_cancelled(&self) {
        self.inner.wait_cancelled().await;
    }
}

impl fmt::Debug for StreamCancelSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamCancelSignal")
    }
}

pub trait StreamLifetimeGuardApi: Any + Send + Sync + fmt::Debug {}

/// Opaque request-lifetime guard retained by the concrete stream registry until the registry
/// observes one terminal transition. It deliberately exposes no activation-specific API.
#[derive(Clone)]
pub struct StreamLifetimeGuard {
    _inner: Arc<dyn StreamLifetimeGuardApi>,
}

impl StreamLifetimeGuard {
    pub fn new<T>(inner: T) -> Self
    where
        T: StreamLifetimeGuardApi + 'static,
    {
        Self {
            _inner: Arc::new(inner),
        }
    }
}

impl fmt::Debug for StreamLifetimeGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamLifetimeGuard")
    }
}

pub trait StreamSinkApi: Any + Send + Sync + fmt::Debug {
    /// Gives an in-process boundary a chance to project a non-JSON value before the ordinary emit
    /// encoder runs. Returning `None` preserves the existing typed JSON path.
    fn project_runtime_item(
        &self,
        _item: RuntimeValue,
        _source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        Ok(None)
    }
    fn send_internal_with_cancellation<'a>(
        &'a self,
        _item: StreamInternalItem,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async {
            Err(StreamRuntimeError::decode(
                "stream sink does not accept in-process runtime items",
            ))
        })
    }
    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>>;
    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>>;
    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>>;
    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
    fn is_cancelled(&self) -> bool;
    fn is_same_stream(&self, other: &StreamSink) -> bool;
    fn cancel_flag(&self) -> Arc<AtomicBool>;
    fn cancel_signal(&self) -> StreamCancelSignal;
}

#[derive(Clone)]
pub struct StreamSink {
    inner: Arc<dyn StreamSinkApi>,
}

impl StreamSink {
    pub fn new<T>(inner: T) -> Self
    where
        T: StreamSinkApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub async fn send(&self, item: Value) -> StreamRuntimeResult<()> {
        self.inner.send(item).await
    }

    pub async fn send_with_cancel(
        &self,
        item: Value,
        cancel_flags: &[Arc<AtomicBool>],
    ) -> StreamRuntimeResult<()> {
        self.inner.send_with_cancel(item, cancel_flags).await
    }

    pub async fn send_with_cancellation(
        &self,
        item: Value,
        signals: &[StreamCancelSignal],
        cancel_tokens: impl IntoIterator<Item = CancellationToken>,
    ) -> StreamRuntimeResult<()> {
        self.inner
            .send_with_cancellation(item, signals, cancel_tokens.into_iter().collect())
            .await
    }

    pub fn project_runtime_item(
        &self,
        item: RuntimeValue,
        source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        self.inner.project_runtime_item(item, source_heap)
    }

    pub async fn send_internal_with_cancellation(
        &self,
        item: StreamInternalItem,
        signals: &[StreamCancelSignal],
        cancel_tokens: impl IntoIterator<Item = CancellationToken>,
    ) -> StreamRuntimeResult<()> {
        self.inner
            .send_internal_with_cancellation(item, signals, cancel_tokens.into_iter().collect())
            .await
    }

    pub async fn end(&self) {
        self.inner.end().await;
    }

    pub async fn fail(&self, error: StreamRuntimeError) {
        self.inner.fail(error).await;
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    pub fn is_same_stream(&self, other: &Self) -> bool {
        self.inner.is_same_stream(other)
    }

    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        self.inner.cancel_flag()
    }

    pub fn cancel_signal(&self) -> StreamCancelSignal {
        self.inner.cancel_signal()
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
    }
}

impl fmt::Debug for StreamSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamSink")
    }
}

#[derive(Clone, Debug)]
pub struct TypedStreamSink {
    pub sink: StreamSink,
    pub item_type: RuntimeTypePlan,
}

#[derive(Clone, Debug, Default)]
pub struct StreamCapabilityContext {
    current_stream_sink: Option<StreamSink>,
    response_stream_sink: Option<TypedStreamSink>,
}

impl StreamCapabilityContext {
    pub fn new(
        current_stream_sink: Option<StreamSink>,
        response_stream_sink: Option<TypedStreamSink>,
    ) -> Self {
        Self {
            current_stream_sink,
            response_stream_sink,
        }
    }
}

pub trait StreamRuntimeApi: Any + Send + Sync + fmt::Debug {
    fn channel_stream(&self) -> (Value, StreamSink);
    fn channel_stream_with_lifetime(&self, lifetime: StreamLifetimeGuard) -> (Value, StreamSink);
    fn pull_stream_with_cancellation(
        &self,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value;
    fn buffered_stream(&self, items: Vec<Value>) -> Value;
    fn next_with_cancel<'a>(
        &'a self,
        value: &'a Value,
        signals: &'a [StreamCancelSignal],
        cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>>;
    fn next_with_cancellation<'a>(
        &'a self,
        value: &'a Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>>;
    fn next<'a>(
        &'a self,
        value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>>;
    fn cancel(&self, value: &Value);
    fn open_request_scope(&self, _request_generation: u64) -> bool {
        false
    }
    fn close_request_scope(&self, _request_generation: u64) {}
    fn close_owner(&self) {}
    fn channel_stream_in_request_scope(&self, _request_generation: u64) -> (Value, StreamSink) {
        self.channel_stream()
    }
    fn channel_stream_with_lifetime_in_request_scope(
        &self,
        _request_generation: u64,
        lifetime: StreamLifetimeGuard,
    ) -> (Value, StreamSink) {
        self.channel_stream_with_lifetime(lifetime)
    }
    fn pull_stream_with_cancellation_in_request_scope(
        &self,
        _request_generation: u64,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value {
        self.pull_stream_with_cancellation(source, cancellation)
    }
    fn buffered_stream_in_request_scope(
        &self,
        _request_generation: u64,
        items: Vec<Value>,
    ) -> Value {
        self.buffered_stream(items)
    }
}

#[derive(Clone)]
pub struct StreamRuntime {
    inner: Arc<dyn StreamRuntimeApi>,
    request_generation: Option<u64>,
}

pub struct StreamRuntimeOwner {
    inner: Arc<dyn StreamRuntimeApi>,
    target: StreamRuntimeOwnerTarget,
}

enum StreamRuntimeOwnerTarget {
    Root,
    Request(u64),
    Noop,
}

impl StreamRuntime {
    pub fn new<T>(inner: T) -> Self
    where
        T: StreamRuntimeApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
            request_generation: None,
        }
    }

    pub fn channel_stream(&self) -> (Value, StreamSink) {
        match self.request_generation {
            Some(request_generation) => self
                .inner
                .channel_stream_in_request_scope(request_generation),
            None => self.inner.channel_stream(),
        }
    }

    pub fn channel_stream_with_lifetime(
        &self,
        lifetime: StreamLifetimeGuard,
    ) -> (Value, StreamSink) {
        match self.request_generation {
            Some(request_generation) => self
                .inner
                .channel_stream_with_lifetime_in_request_scope(request_generation, lifetime),
            None => self.inner.channel_stream_with_lifetime(lifetime),
        }
    }

    pub fn pull_stream_with_cancellation(
        &self,
        source: impl StreamPullSource + 'static,
        cancellation: CancellationToken,
    ) -> Value {
        match self.request_generation {
            Some(request_generation) => self.inner.pull_stream_with_cancellation_in_request_scope(
                request_generation,
                Box::new(source),
                cancellation,
            ),
            None => self
                .inner
                .pull_stream_with_cancellation(Box::new(source), cancellation),
        }
    }

    pub fn buffered_stream(&self, items: impl IntoIterator<Item = Value>) -> Value {
        let items = items.into_iter().collect();
        match self.request_generation {
            Some(request_generation) => self
                .inner
                .buffered_stream_in_request_scope(request_generation, items),
            None => self.inner.buffered_stream(items),
        }
    }

    pub async fn next_with_cancel(
        &self,
        value: &Value,
        signals: &[StreamCancelSignal],
        cancel_flags: &[Arc<AtomicBool>],
    ) -> StreamRuntimeResult<StreamPoll> {
        self.inner
            .next_with_cancel(value, signals, cancel_flags)
            .await
    }

    pub async fn next_with_cancellation(
        &self,
        value: &Value,
        signals: &[StreamCancelSignal],
        cancel_tokens: impl IntoIterator<Item = CancellationToken>,
    ) -> StreamRuntimeResult<StreamPoll> {
        self.inner
            .next_with_cancellation(value, signals, cancel_tokens.into_iter().collect())
            .await
    }

    pub async fn next(&self, value: &Value) -> StreamRuntimeResult<StreamPoll> {
        self.inner.next(value).await
    }

    pub fn cancel(&self, value: &Value) {
        self.inner.cancel(value);
    }

    pub fn request_scope(&self, request_generation: u64) -> (Self, StreamRuntimeOwner) {
        let opened = self.inner.open_request_scope(request_generation);
        let scoped_generation = opened.then_some(request_generation);
        (
            Self {
                inner: self.inner.clone(),
                request_generation: scoped_generation,
            },
            StreamRuntimeOwner {
                inner: self.inner.clone(),
                target: if opened {
                    StreamRuntimeOwnerTarget::Request(request_generation)
                } else {
                    StreamRuntimeOwnerTarget::Noop
                },
            },
        )
    }

    pub fn owner(&self) -> StreamRuntimeOwner {
        StreamRuntimeOwner {
            inner: self.inner.clone(),
            target: StreamRuntimeOwnerTarget::Root,
        }
    }

    pub fn request_scope_generation(&self) -> Option<u64> {
        self.request_generation
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
    }
}

impl Drop for StreamRuntimeOwner {
    fn drop(&mut self) {
        match self.target {
            StreamRuntimeOwnerTarget::Root => self.inner.close_owner(),
            StreamRuntimeOwnerTarget::Request(request_generation) => {
                self.inner.close_request_scope(request_generation)
            }
            StreamRuntimeOwnerTarget::Noop => {}
        }
    }
}

impl fmt::Debug for StreamRuntimeOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamRuntimeOwner")
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for StreamRuntimeOwnerTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => formatter.write_str("Root"),
            Self::Request(request_generation) => formatter
                .debug_tuple("Request")
                .field(request_generation)
                .finish(),
            Self::Noop => formatter.write_str("Noop"),
        }
    }
}

impl fmt::Debug for StreamRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamRuntime")
    }
}

#[derive(Clone)]
pub struct HttpResponseStreamCapabilityContext<'execution> {
    execution: HttpResponseStreamExecution<'execution>,
    stream_context: StreamCapabilityContext,
}

#[derive(Clone)]
enum HttpResponseStreamExecution<'execution> {
    Borrowed(ExecutionControl<'execution>),
    Owned(OwnedExecutionControl),
}

impl HttpResponseStreamExecution<'_> {
    fn cancellation_token(&self) -> CancellationToken {
        match self {
            Self::Borrowed(execution) => execution.cancellation_token(),
            Self::Owned(execution) => execution.cancellation_token(),
        }
    }
}

impl<'execution> HttpResponseStreamCapabilityContext<'execution> {
    pub fn new(
        execution: ExecutionControl<'execution>,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            execution: HttpResponseStreamExecution::Borrowed(execution),
            stream_context,
        }
    }

    pub fn response_item_type(&self, target: &str) -> StreamRuntimeResult<&RuntimeTypePlan> {
        Ok(&self.response_stream_sink(target)?.item_type)
    }

    pub async fn send_response_event(&self, target: &str, event: Value) -> StreamRuntimeResult<()> {
        let typed_sink = self.response_stream_sink(target)?;
        let mut signals = Vec::new();
        if let Some(inner_sink) = self.stream_context.current_stream_sink.as_ref() {
            if !inner_sink.is_same_stream(&typed_sink.sink) {
                signals.push(inner_sink.cancel_signal());
            }
        }
        typed_sink
            .sink
            .send_with_cancellation(event, &signals, [self.execution.cancellation_token()])
            .await
    }

    fn response_stream_sink(&self, target: &str) -> StreamRuntimeResult<&TypedStreamSink> {
        self.stream_context
            .response_stream_sink
            .as_ref()
            .ok_or_else(|| {
                StreamRuntimeError::decode(format!(
                    "{target} used outside a raw HTTP streaming response context"
                ))
            })
    }
}

impl HttpResponseStreamCapabilityContext<'static> {
    pub fn from_owned_execution(
        execution: OwnedExecutionControl,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            execution: HttpResponseStreamExecution::Owned(execution),
            stream_context,
        }
    }
}

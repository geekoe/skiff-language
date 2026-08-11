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

use crate::{
    CancellationToken, ExecutionControl, ExecutionScope, ExecutionScopeAccessError,
    ExecutionScopeLeaseTerminal, OwnedExecutionControl,
};

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
    heap: Box<RequestHeap>,
}

impl StreamInternalItem {
    pub fn new(value: RuntimeValue, heap: RequestHeap) -> Self {
        Self {
            value,
            heap: Box::new(heap),
        }
    }

    pub fn into_parts(self) -> (RuntimeValue, RequestHeap) {
        (self.value, *self.heap)
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

#[derive(Clone)]
pub struct StreamRuntimeOwner {
    // Context clones and detached stream tasks share one close authority.
    // Independently opened request scopes still receive distinct leases and
    // therefore retain the concrete runtime's nested-owner accounting.
    lease: Arc<StreamRuntimeOwnerLease>,
}

struct StreamRuntimeOwnerLease {
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
                lease: Arc::new(StreamRuntimeOwnerLease {
                    inner: self.inner.clone(),
                    target: if opened {
                        StreamRuntimeOwnerTarget::Request(request_generation)
                    } else {
                        StreamRuntimeOwnerTarget::Noop
                    },
                }),
            },
        )
    }

    pub fn owner(&self) -> StreamRuntimeOwner {
        StreamRuntimeOwner {
            lease: Arc::new(StreamRuntimeOwnerLease {
                inner: self.inner.clone(),
                target: StreamRuntimeOwnerTarget::Root,
            }),
        }
    }

    /// Opens one additional owner for the already-selected request scope.
    ///
    /// Detached producer tasks call this only when they are about to task, so
    /// a parked-but-never-driven producer cannot keep a request registry alive.
    pub fn retain_request_scope(&self) -> Option<StreamRuntimeOwner> {
        self.request_generation.map(|request_generation| {
            let opened = self.inner.open_request_scope(request_generation);
            StreamRuntimeOwner {
                lease: Arc::new(StreamRuntimeOwnerLease {
                    inner: self.inner.clone(),
                    target: if opened {
                        StreamRuntimeOwnerTarget::Request(request_generation)
                    } else {
                        StreamRuntimeOwnerTarget::Noop
                    },
                }),
            }
        })
    }

    pub fn request_scope_generation(&self) -> Option<u64> {
        self.request_generation
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
    }
}

impl Drop for StreamRuntimeOwnerLease {
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
            .field("target", &self.lease.target)
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

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        match self {
            Self::Borrowed(execution) => execution.execution_scope(),
            Self::Owned(execution) => execution.execution_scope(),
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
        let scope = self.execution.execution_scope().map_err(|error| {
            StreamRuntimeError::decode(format!(
                "current execution scope is unavailable for {target}: {error}"
            ))
        })?;
        let (lease, completion) = scope.acquire_lease();
        let child_cancellation = lease.child_cancellation_token();
        let mut signals = Vec::new();
        if let Some(inner_sink) = self.stream_context.current_stream_sink.as_ref() {
            if !inner_sink.is_same_stream(&typed_sink.sink) {
                signals.push(inner_sink.cancel_signal());
            }
        }
        let send = async {
            let output = typed_sink
                .sink
                .send_with_cancellation(
                    event,
                    &signals,
                    [self.execution.cancellation_token(), child_cancellation],
                )
                .await;
            (completion.complete(), output)
        };
        tokio::pin!(send);
        tokio::select! {
            biased;
            terminal = lease.wait() => match terminal {
                ExecutionScopeLeaseTerminal::Control(_) => Err(StreamRuntimeError::cancelled()),
                ExecutionScopeLeaseTerminal::Completed => {
                    unreachable!("response sink scope lease completion is owned by the send branch")
                }
            },
            (completed, output) = &mut send => {
                if completed {
                    output
                } else {
                    Err(StreamRuntimeError::cancelled())
                }
            }
        }
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

#[cfg(test)]
mod f445h_i6_response_sink_scope_tests {
    use std::{
        future,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use serde_json::json;
    use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
    use tokio::sync::Notify;

    use super::*;
    use crate::{
        CancellationSignals, CancellationSource, ExecutionControlApi, ExecutionControlResult,
        ExecutionScope, ExecutionScopeAccessError, FileSourceStreamContext,
        OwnedExecutionControlApi, StreamConsumerCleanup,
    };

    #[derive(Clone)]
    struct TestControlState {
        scope: ExecutionScope,
        root_token: CancellationToken,
        root_flag: Arc<AtomicBool>,
    }

    impl TestControlState {
        fn owned(&self) -> OwnedExecutionControl {
            OwnedExecutionControl::new(TestOwnedControl(self.clone()))
        }
    }

    struct TestBorrowedControl(TestControlState);

    impl ExecutionControlApi for TestBorrowedControl {
        fn owned(&self) -> OwnedExecutionControl {
            self.0.owned()
        }

        fn cancel_flag(&self) -> Arc<AtomicBool> {
            self.0.root_flag.clone()
        }

        fn cancellation_token(&self) -> CancellationToken {
            self.0.root_token.clone()
        }

        fn deadline(&self) -> Option<std::time::Instant> {
            self.0
                .scope
                .effective_deadline()
                .map(|deadline| deadline.at())
        }

        fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
            Ok(self.0.scope.clone())
        }

        fn derive_scope(
            &self,
            local_deadline: std::time::Instant,
            site: InstructionSourceSite,
        ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
            Ok(TestControlState {
                scope: self
                    .0
                    .scope
                    .derive(local_deadline, site)
                    .map_err(ExecutionScopeAccessError::from)?,
                root_token: self.0.root_token.clone(),
                root_flag: self.0.root_flag.clone(),
            }
            .owned())
        }

        fn check_cancelled(&self) -> ExecutionControlResult<()> {
            Ok(())
        }

        fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
            Ok(())
        }

        fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
            Ok(())
        }

        fn file_source_stream_context(
            &self,
            _stream_runtime: StreamRuntime,
        ) -> FileSourceStreamContext<'static> {
            unreachable!("response sink scope test does not use file streams")
        }
    }

    struct TestOwnedControl(TestControlState);

    impl OwnedExecutionControlApi for TestOwnedControl {
        fn borrow(&self) -> ExecutionControl<'_> {
            ExecutionControl::new(TestBorrowedControl(self.0.clone()))
        }

        fn cancelled(&self) -> &AtomicBool {
            self.0.root_flag.as_ref()
        }

        fn cancellation_token(&self) -> CancellationToken {
            self.0.root_token.clone()
        }

        fn deadline(&self) -> Option<std::time::Instant> {
            self.0
                .scope
                .effective_deadline()
                .map(|deadline| deadline.at())
        }

        fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
            Ok(self.0.scope.clone())
        }

        fn derive_scope(
            &self,
            local_deadline: std::time::Instant,
            site: InstructionSourceSite,
        ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
            self.borrow().derive_scope(local_deadline, site)
        }
    }

    #[derive(Debug, Default)]
    struct CapacitySinkState {
        capacity: Notify,
        pending: AtomicUsize,
        writes: AtomicUsize,
        ends: AtomicUsize,
        failures: AtomicUsize,
    }

    #[derive(Clone, Debug)]
    struct CapacitySink {
        state: Arc<CapacitySinkState>,
    }

    struct PendingSendGuard(Arc<CapacitySinkState>);

    impl Drop for PendingSendGuard {
        fn drop(&mut self) {
            self.0.pending.fetch_sub(1, Ordering::AcqRel);
        }
    }

    #[derive(Debug)]
    struct NeverCancelled;

    impl StreamCancelSignalApi for NeverCancelled {
        fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(future::pending())
        }
    }

    impl StreamSinkApi for CapacitySink {
        fn send<'a>(
            &'a self,
            _item: Value,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            Box::pin(future::pending())
        }

        fn send_with_cancel<'a>(
            &'a self,
            _item: Value,
            _cancel_flags: &'a [Arc<AtomicBool>],
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            Box::pin(future::pending())
        }

        fn send_with_cancellation<'a>(
            &'a self,
            _item: Value,
            signals: &'a [StreamCancelSignal],
            cancel_tokens: Vec<CancellationToken>,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
            Box::pin(async move {
                self.state.pending.fetch_add(1, Ordering::AcqRel);
                let _pending = PendingSendGuard(self.state.clone());
                let signal_wait = async {
                    match signals.first() {
                        Some(signal) => signal.wait_cancelled().await,
                        None => future::pending().await,
                    }
                };
                let token_signals = CancellationSignals::from_tokens(cancel_tokens);
                tokio::select! {
                    biased;
                    _ = signal_wait => Err(StreamRuntimeError::cancelled()),
                    _ = token_signals.wait_cancelled() => Err(StreamRuntimeError::cancelled()),
                    _ = self.state.capacity.notified() => {
                        self.state.writes.fetch_add(1, Ordering::AcqRel);
                        Ok(())
                    }
                }
            })
        }

        fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.state.ends.fetch_add(1, Ordering::AcqRel);
            })
        }

        fn fail<'a>(
            &'a self,
            _error: StreamRuntimeError,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
            Box::pin(async move {
                self.state.failures.fetch_add(1, Ordering::AcqRel);
            })
        }

        fn is_cancelled(&self) -> bool {
            false
        }

        fn is_same_stream(&self, other: &StreamSink) -> bool {
            other
                .downcast_ref::<Self>()
                .is_some_and(|other| Arc::ptr_eq(&self.state, &other.state))
        }

        fn cancel_flag(&self) -> Arc<AtomicBool> {
            Arc::new(AtomicBool::new(false))
        }

        fn cancel_signal(&self) -> StreamCancelSignal {
            StreamCancelSignal::new(NeverCancelled)
        }
    }

    fn site() -> InstructionSourceSite {
        InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
        }
    }

    fn context(
        scope: ExecutionScope,
        root_token: CancellationToken,
        state: Arc<CapacitySinkState>,
    ) -> HttpResponseStreamCapabilityContext<'static> {
        let control = TestControlState {
            root_flag: root_token.cancel_flag(),
            root_token,
            scope,
        }
        .owned();
        HttpResponseStreamCapabilityContext::from_owned_execution(
            control,
            StreamCapabilityContext::new(
                None,
                Some(TypedStreamSink {
                    sink: StreamSink::new(CapacitySink { state }),
                    item_type: RuntimeTypePlan::json_value_plan(),
                }),
            ),
        )
    }

    async fn wait_for_pending(state: &CapacitySinkState) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.pending.load(Ordering::Acquire) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("response sink should reach the capacity wait");
    }

    fn assert_no_sink_terminal_side_effects(state: &CapacitySinkState) {
        assert_eq!(state.writes.load(Ordering::Acquire), 0);
        assert_eq!(state.pending.load(Ordering::Acquire), 0);
        assert_eq!(state.ends.load(Ordering::Acquire), 0);
        assert_eq!(state.failures.load(Ordering::Acquire), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn f445h_i6_response_sink_scope_current_deadline_wakes_capacity_pending_and_fences_late_wake(
    ) {
        let root = CancellationSource::new();
        let request_scope = ExecutionScope::request(root.token(), None);
        let current_scope = request_scope
            .derive(
                tokio::time::Instant::now().into_std() + Duration::from_secs(5),
                site(),
            )
            .expect("derived current scope");
        let lifecycle = current_scope.clone();
        let state = Arc::new(CapacitySinkState::default());
        let response = context(current_scope, root.token(), state.clone());

        let task = tokio::spawn(async move {
            response
                .send_response_event("std.http.stream.emitResponse", json!({"chunk": 1}))
                .await
        });
        wait_for_pending(&state).await;
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;

        assert!(
            task.is_finished(),
            "current absolute deadline must wake a capacity-Pending response sink"
        );
        let terminal = task.await.expect("response sink task should not panic");
        assert!(matches!(terminal, Err(StreamRuntimeError::Cancelled)));
        state.capacity.notify_waiters();
        tokio::task::yield_now().await;

        assert_no_sink_terminal_side_effects(&state);
        assert_eq!(
            lifecycle.lifecycle_snapshot(),
            Default::default(),
            "deadline winner must release the scope lease/waiter/timer"
        );
    }

    #[tokio::test]
    async fn f445h_i6_response_sink_scope_ancestor_stop_wakes_capacity_pending_and_fences_late_wake(
    ) {
        let root = CancellationSource::new();
        let parent_scope = ExecutionScope::request(root.token(), None);
        let (ancestor_lease, _ancestor_completion) = parent_scope.acquire_lease();
        let current_scope = ancestor_lease.child_execution_scope();
        let lifecycle = current_scope.clone();
        let state = Arc::new(CapacitySinkState::default());
        let response = context(current_scope, root.token(), state.clone());

        let task = tokio::spawn(async move {
            response
                .send_response_event("std.http.stream.emitResponse", json!({"chunk": 1}))
                .await
        });
        wait_for_pending(&state).await;
        drop(ancestor_lease);
        tokio::task::yield_now().await;

        assert!(
            task.is_finished(),
            "current ancestor stop must wake a capacity-Pending response sink"
        );
        let terminal = task.await.expect("response sink task should not panic");
        assert!(matches!(terminal, Err(StreamRuntimeError::Cancelled)));
        state.capacity.notify_waiters();
        tokio::task::yield_now().await;

        assert_no_sink_terminal_side_effects(&state);
        assert_eq!(
            lifecycle.lifecycle_snapshot(),
            Default::default(),
            "ancestor winner must release every scope lease/waiter"
        );
    }

    #[tokio::test]
    async fn f445h_i6_response_sink_scope_capacity_completion_settles_lease_and_writes_once() {
        let root = CancellationSource::new();
        let current_scope = ExecutionScope::request(root.token(), None);
        let lifecycle = current_scope.clone();
        let state = Arc::new(CapacitySinkState::default());
        let response = context(current_scope, root.token(), state.clone());

        let task = tokio::spawn(async move {
            response
                .send_response_event("std.http.stream.emitResponse", json!({"chunk": 1}))
                .await
        });
        wait_for_pending(&state).await;
        state.capacity.notify_waiters();

        assert!(
            task.await
                .expect("response sink task should not panic")
                .is_ok(),
            "capacity completion should remain a normal response write"
        );
        assert_eq!(state.writes.load(Ordering::Acquire), 1);
        assert_eq!(state.pending.load(Ordering::Acquire), 0);
        assert_eq!(state.ends.load(Ordering::Acquire), 0);
        assert_eq!(state.failures.load(Ordering::Acquire), 0);
        assert_eq!(
            lifecycle.lifecycle_snapshot(),
            Default::default(),
            "normal capacity completion must release the scope lease/waiter"
        );
    }

    #[test]
    fn f445h_i6_response_sink_scope_keeps_natural_end_and_non_end_cleanup_with_consumer_owner() {
        let natural_end_cancels = Arc::new(AtomicUsize::new(0));
        {
            let cancels = natural_end_cancels.clone();
            let mut cleanup =
                StreamConsumerCleanup::from_cancel(&json!("natural-end"), move |_| {
                    cancels.fetch_add(1, Ordering::AcqRel);
                });
            cleanup.reached_end();
        }
        assert_eq!(natural_end_cancels.load(Ordering::Acquire), 0);

        let non_end_cancels = Arc::new(AtomicUsize::new(0));
        {
            let cancels = non_end_cancels.clone();
            let _cleanup = StreamConsumerCleanup::from_cancel(&json!("non-end"), move |_| {
                cancels.fetch_add(1, Ordering::AcqRel);
            });
        }
        assert_eq!(non_end_cancels.load(Ordering::Acquire), 1);
    }
}

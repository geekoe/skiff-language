use std::{
    any::Any,
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::Value;
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    service_error::{CatchIdentity, PlatformBuiltinErrorIdentity},
    type_plan::RuntimeTypePlan,
};

use crate::{CancellationToken, ExecutionControl};

pub type StreamRuntimeResult<T> = Result<T, StreamRuntimeError>;

const REQUEST_CANCELLED_MESSAGE: &str = "request was cancelled";

#[derive(Debug)]
pub enum StreamRuntimeError {
    Decode(String),
    Cancelled,
    Producer(Box<dyn WirePayload>),
}

impl StreamRuntimeError {
    pub fn decode(message: impl Into<String>) -> Self {
        Self::Decode(message.into())
    }

    pub fn cancelled() -> Self {
        Self::Cancelled
    }

    pub fn producer(error: impl WirePayload) -> Self {
        Self::Producer(Box::new(error))
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

impl WirePayload for StreamRuntimeError {
    fn payload(&self) -> RuntimeErrorPayload {
        match self {
            Self::Decode(message) => RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message: message.clone(),
                status: None,
                details: None,
            },
            Self::Cancelled => cancel_payload(),
            Self::Producer(error) => error.payload(),
        }
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        match self {
            Self::Cancelled => Some((
                PlatformBuiltinErrorIdentity::Cancel.catch_identity(),
                serde_json::json!({
                    "message": REQUEST_CANCELLED_MESSAGE,
                }),
            )),
            Self::Producer(error) => error.catch_projection(),
            Self::Decode(_) => None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn cancel_payload() -> RuntimeErrorPayload {
    RuntimeErrorPayload {
        code: "CancelError".to_string(),
        message: REQUEST_CANCELLED_MESSAGE.to_string(),
        status: None,
        details: None,
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
    execution: ExecutionControl<'execution>,
    stream_context: StreamCapabilityContext,
}

impl<'execution> HttpResponseStreamCapabilityContext<'execution> {
    pub fn new(
        execution: ExecutionControl<'execution>,
        stream_context: StreamCapabilityContext,
    ) -> Self {
        Self {
            execution,
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

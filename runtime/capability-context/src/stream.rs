use std::{
    any::Any,
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
};

use serde_json::Value;
use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};
use skiff_runtime_model::type_plan::RuntimeTypePlan;

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

    fn catch_projection(&self) -> Option<(TypeIdentity, Value)> {
        match self {
            Self::Cancelled => Some((
                TypeIdentity::builtin("CancelError"),
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
    End,
}

pub trait StreamCancelSignalApi: Any + Send + Sync + fmt::Debug {}

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
}

impl fmt::Debug for StreamCancelSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamCancelSignal")
    }
}

pub trait StreamSinkApi: Any + Send + Sync + fmt::Debug {
    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>>;
    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<AtomicBool>],
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
    fn next<'a>(
        &'a self,
        value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>>;
    fn cancel(&self, value: &Value);
}

#[derive(Clone)]
pub struct StreamRuntime {
    inner: Arc<dyn StreamRuntimeApi>,
}

impl StreamRuntime {
    pub fn new<T>(inner: T) -> Self
    where
        T: StreamRuntimeApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn channel_stream(&self) -> (Value, StreamSink) {
        self.inner.channel_stream()
    }

    pub fn pull_stream_with_cancellation(
        &self,
        source: impl StreamPullSource + 'static,
        cancellation: CancellationToken,
    ) -> Value {
        self.inner
            .pull_stream_with_cancellation(Box::new(source), cancellation)
    }

    pub fn buffered_stream(&self, items: impl IntoIterator<Item = Value>) -> Value {
        self.inner.buffered_stream(items.into_iter().collect())
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

    pub async fn next(&self, value: &Value) -> StreamRuntimeResult<StreamPoll> {
        self.inner.next(value).await
    }

    pub fn cancel(&self, value: &Value) {
        self.inner.cancel(value);
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
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
        let mut cancel_flags = vec![self.execution.cancel_flag()];
        if let Some(inner_sink) = self.stream_context.current_stream_sink.as_ref() {
            if !inner_sink.is_same_stream(&typed_sink.sink) {
                cancel_flags.push(inner_sink.cancel_flag());
            }
        }
        typed_sink.sink.send_with_cancel(event, &cancel_flags).await
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

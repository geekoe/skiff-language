use std::{
    any::Any,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde_json::Value;
use skiff_runtime_capability_context::{
    CancellationToken, StreamCancelSignal, StreamCancelSignalApi, StreamLifetimeGuard,
    StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError, StreamRuntimeResult,
    StreamSink, StreamSinkApi,
};
use skiff_runtime_model::{
    error::{RuntimeErrorPayload, WirePayload},
    service_error::CatchIdentity,
    vm_heap::VmHeapError,
    vm_root::{VmRootSource, VmRootVisitor},
};
use skiff_runtime_request_contract::HttpNameValue;
use skiff_runtime_scheduler::{
    RequestByteStreamFailure, RequestByteStreamPullFuture, RequestByteStreamPullStartError,
    RequestByteStreamSource, RequestResourceHandle, RequestResourceTable,
    RequestResourceTermination,
};

use crate::OwnedExecutionControl;

/// Heap-free owned future returned by a bytecode HTTP provider.
///
/// The scheduler polls this exact future synchronously once. A provider does
/// not report `Ready` or `Pending` through a second status channel.
pub type BytecodeHttpFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, BytecodeHttpFailure>> + Send + 'static>>;

/// Heap-free owned future returned by the transport-only server-stream writer.
pub type BytecodeServerStreamWriteFuture =
    Pin<Box<dyn Future<Output = Result<(), BytecodeServerStreamWriteFailure>> + Send + 'static>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: Vec<HttpNameValue>,
    /// Preserves the language ABI distinction between nullable body absence
    /// and a present zero-length bytes payload.
    pub body: Option<Vec<u8>>,
    pub timeout_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeHttpResponse {
    pub status: u16,
    pub headers: Vec<HttpNameValue>,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytecodeHttpStreamResponse {
    pub status: u16,
    pub headers: Vec<HttpNameValue>,
    pub body: RequestResourceHandle,
}

/// Exact request-thread projection of one linked
/// `std.http.HttpResponseStreamEvent`.
///
/// Sequence numbers are allocated by the central resource table. The host
/// writer may encode and flush this event but cannot assign sequence, buffer
/// capacity or terminal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BytecodeServerStreamFrame {
    Start {
        status: u16,
        headers: Vec<HttpNameValue>,
    },
    Chunk {
        sequence: u64,
        payload: Vec<u8>,
    },
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BytecodeServerStreamWriteFailure {
    Cancelled,
    DeadlineExceeded,
    RouterDisconnected,
    WriterFailed(String),
    InvalidProviderContract(String),
}

impl std::fmt::Display for BytecodeServerStreamWriteFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("server-stream write was cancelled"),
            Self::DeadlineExceeded => {
                formatter.write_str("server-stream write deadline was exceeded")
            }
            Self::RouterDisconnected => {
                formatter.write_str("Router disconnected before server-stream flush")
            }
            Self::WriterFailed(message) => {
                write!(formatter, "server-stream writer failed: {message}")
            }
            Self::InvalidProviderContract(message) => {
                write!(
                    formatter,
                    "server-stream writer contract is invalid: {message}"
                )
            }
        }
    }
}

impl std::error::Error for BytecodeServerStreamWriteFailure {}

/// Transport-only writer for one request's server-stream response.
///
/// Implementations capture the exact request id and Router sender. They may
/// encode/enqueue a frame and await its sole flush acknowledgement, but own no
/// sequence, capacity, mailbox or terminal authority. `flush` only constructs
/// the owned future: encoding and enqueueing begin on that future's first poll.
/// Once a frame is enqueued the future waits for its real acknowledgement; it
/// does not race a second cancellation or deadline authority.
pub trait BytecodeServerStreamWriterPort: Send + Sync {
    fn flush(
        &self,
        frame: BytecodeServerStreamFrame,
        execution: OwnedExecutionControl,
    ) -> BytecodeServerStreamWriteFuture;
}

pub(crate) type SharedBytecodeServerStreamWriterPort = Arc<dyn BytecodeServerStreamWriterPort>;

/// Narrow authority supplied only to the typed HTTP stream method.
///
/// This wrapper is deliberately not a second registry. It retains a clone of
/// the request's one scheduler-owned resource table so the capability-context
/// bridge can register and recover the exact packed handle in that table.
#[derive(Clone)]
pub struct BytecodeHttpStreamRegistrar {
    resources: RequestResourceTable,
}

impl BytecodeHttpStreamRegistrar {
    pub(crate) fn new(resources: RequestResourceTable) -> Self {
        Self { resources }
    }

    /// Capability-context runtime that registers pull sources directly in the
    /// same scheduler-owned request resource table.
    pub fn stream_runtime(&self) -> StreamRuntime {
        StreamRuntime::new(ResourceTableStreamRuntime {
            resources: self.resources.clone(),
        })
    }

    /// Consumes the sealed numeric carrier emitted by [`Self::stream_runtime`]
    /// and claims its exact packed owner/slot/generation once.
    pub fn take_exact_route(
        &self,
        token: Value,
    ) -> Result<RequestResourceHandle, BytecodeHttpFailure> {
        let route = token.as_u64().ok_or_else(|| {
            BytecodeHttpFailure::InvalidProviderContract(
                "HTTP stream body is not a sealed numeric resource route".to_string(),
            )
        })?;
        self.resources
            .claim_vm_route(skiff_runtime_model::vm_value::VmHandle::new(route))
            .map_err(|error| {
                BytecodeHttpFailure::InvalidProviderContract(format!(
                    "HTTP stream body route was rejected: {error}"
                ))
            })
    }
}

struct CapabilityPullState {
    source: Mutex<Option<Box<dyn StreamPullSource>>>,
    cancellation: CancellationToken,
    terminated: AtomicBool,
}

impl std::fmt::Debug for CapabilityPullState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CapabilityPullState")
            .field("terminated", &self.terminated.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

struct CapabilityByteStreamSource {
    shared: Arc<CapabilityPullState>,
}

impl CapabilityByteStreamSource {
    fn new(source: Box<dyn StreamPullSource>, cancellation: CancellationToken) -> Self {
        Self {
            shared: Arc::new(CapabilityPullState {
                source: Mutex::new(Some(source)),
                cancellation,
                terminated: AtomicBool::new(false),
            }),
        }
    }
}

impl VmRootSource for CapabilityByteStreamSource {
    fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        Ok(())
    }
}

impl RequestByteStreamSource for CapabilityByteStreamSource {
    fn start_pull(&self) -> Result<RequestByteStreamPullFuture, RequestByteStreamPullStartError> {
        if self.shared.terminated.load(Ordering::Acquire) {
            return Err(RequestByteStreamPullStartError::Terminated);
        }
        let mut source = self
            .shared
            .source
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(RequestByteStreamPullStartError::PullInProgress)?;
        let shared = Arc::clone(&self.shared);
        Ok(Box::pin(async move {
            let output = {
                let mut next = source.next();
                let cancellation = shared.cancellation.clone();
                let mut cancelled = Box::pin(cancellation.wait_cancelled());
                std::future::poll_fn(|context| {
                    if cancelled.as_mut().poll(context).is_ready() {
                        return std::task::Poll::Ready(Err(RequestByteStreamFailure::Cancelled));
                    }
                    match next.as_mut().poll(context) {
                        std::task::Poll::Ready(output) => std::task::Poll::Ready(Ok(output)),
                        std::task::Poll::Pending => std::task::Poll::Pending,
                    }
                })
                .await
            };
            {
                let mut slot = shared
                    .source
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if output.is_ok() && !shared.terminated.load(Ordering::Acquire) {
                    let previous = slot.replace(source);
                    debug_assert!(previous.is_none());
                }
            }
            match output {
                Err(failure) => Err(failure),
                Ok(Ok(Some(value))) => skiff_runtime_boundary::value::bytes_payload(&value)
                    .map(Some)
                    .ok_or_else(|| {
                        RequestByteStreamFailure::InvalidProviderContract(
                            "HTTP body pull produced a non-bytes item".to_string(),
                        )
                    }),
                Ok(Ok(None)) => Ok(None),
                Ok(Err(error)) if error.is_cancellation_terminal() => {
                    Err(RequestByteStreamFailure::Cancelled)
                }
                Ok(Err(error)) => Err(RequestByteStreamFailure::Ordinary(Box::new(
                    CapabilityStreamOrdinaryFailure::new(error),
                ))),
            }
        }))
    }

    fn terminate(self: Box<Self>, _termination: RequestResourceTermination) {
        self.shared.terminated.store(true, Ordering::Release);
        self.shared.cancellation.cancel();
        drop(
            self.shared
                .source
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
        );
    }
}

#[derive(Debug)]
struct CapabilityStreamOrdinaryFailure {
    payload: RuntimeErrorPayload,
    catch: Option<(CatchIdentity, Value)>,
}

impl CapabilityStreamOrdinaryFailure {
    fn new(error: StreamRuntimeError) -> Self {
        Self {
            payload: error
                .ordinary_payload()
                .expect("cancellation was split before ordinary stream failure construction"),
            catch: error.ordinary_catch_projection(),
        }
    }
}

impl std::fmt::Display for CapabilityStreamOrdinaryFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.payload.fmt(formatter)
    }
}

impl std::error::Error for CapabilityStreamOrdinaryFailure {}

impl WirePayload for CapabilityStreamOrdinaryFailure {
    fn payload(&self) -> RuntimeErrorPayload {
        self.payload.clone()
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        self.catch.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Clone)]
struct ResourceTableStreamRuntime {
    resources: RequestResourceTable,
}

impl std::fmt::Debug for ResourceTableStreamRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResourceTableStreamRuntime")
            .field("resources", &self.resources)
            .finish()
    }
}

impl ResourceTableStreamRuntime {
    fn register_pull_source(
        &self,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value {
        let source = Box::new(CapabilityByteStreamSource::new(source, cancellation));
        let route = self
            .resources
            .register_byte_stream(source)
            .map_or(0, |handle| handle.vm_handle().get());
        Value::Number(route.into())
    }

    fn unsupported_poll() -> StreamRuntimeResult<skiff_runtime_capability_context::StreamPoll> {
        Err(StreamRuntimeError::decode(
            "resource-table bytecode stream runtime only admits pull-source registration",
        ))
    }
}

impl StreamRuntimeApi for ResourceTableStreamRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        (Value::Null, StreamSink::new(RejectingStreamSink))
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        self.channel_stream()
    }

    fn pull_stream_with_cancellation(
        &self,
        source: Box<dyn StreamPullSource>,
        cancellation: CancellationToken,
    ) -> Value {
        self.register_pull_source(source, cancellation)
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        Value::Null
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<
        Box<
            dyn Future<Output = StreamRuntimeResult<skiff_runtime_capability_context::StreamPoll>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Self::unsupported_poll() })
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<
        Box<
            dyn Future<Output = StreamRuntimeResult<skiff_runtime_capability_context::StreamPoll>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Self::unsupported_poll() })
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<
        Box<
            dyn Future<Output = StreamRuntimeResult<skiff_runtime_capability_context::StreamPoll>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { Self::unsupported_poll() })
    }

    fn cancel(&self, value: &Value) {
        let Some(route) = value.as_u64() else {
            return;
        };
        let Ok(handle) = self
            .resources
            .validate_vm_route(skiff_runtime_model::vm_value::VmHandle::new(route))
        else {
            return;
        };
        let _ = self
            .resources
            .terminate(&handle, RequestResourceTermination::Cancelled);
    }
}

#[derive(Debug)]
struct RejectingStreamSink;

impl RejectingStreamSink {
    fn rejected<'a>() -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async {
            Err(StreamRuntimeError::decode(
                "resource-table bytecode stream runtime does not create channel streams",
            ))
        })
    }
}

impl StreamSinkApi for RejectingStreamSink {
    fn send<'a>(
        &'a self,
        _item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Self::rejected()
    }

    fn send_with_cancel<'a>(
        &'a self,
        _item: Value,
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Self::rejected()
    }

    fn send_with_cancellation<'a>(
        &'a self,
        _item: Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Self::rejected()
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
        true
    }

    fn is_same_stream(&self, _other: &StreamSink) -> bool {
        false
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(true))
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        StreamCancelSignal::new(AlreadyCancelledSignal)
    }
}

#[derive(Debug)]
struct AlreadyCancelledSignal;

impl StreamCancelSignalApi for AlreadyCancelledSignal {
    fn wait_cancelled<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

/// Closed HTTP failure vocabulary crossing the provider/scheduler seam.
///
/// Cancellation is an internal terminal and is never wrapped as an ordinary
/// wire error. Deadline, response-cap, transport, input and provider-contract
/// failures stay distinct so the request driver cannot infer a winner from a
/// diagnostic string.
#[derive(Debug)]
pub enum BytecodeHttpFailure {
    Cancelled,
    DeadlineExceeded,
    ResponseLimitExceeded {
        limit_bytes: usize,
        received_bytes: usize,
    },
    Transport(Box<dyn WirePayload>),
    InvalidInput(Box<dyn WirePayload>),
    InvalidProviderContract(String),
}

impl std::fmt::Display for BytecodeHttpFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("HTTP request was cancelled"),
            Self::DeadlineExceeded => formatter.write_str("HTTP request deadline exceeded"),
            Self::ResponseLimitExceeded {
                limit_bytes,
                received_bytes,
            } => write!(
                formatter,
                "HTTP response exceeded {limit_bytes} byte limit after {received_bytes} bytes"
            ),
            Self::Transport(error) => write!(formatter, "HTTP transport failed: {error}"),
            Self::InvalidInput(error) => write!(formatter, "HTTP input was rejected: {error}"),
            Self::InvalidProviderContract(message) => {
                write!(formatter, "HTTP provider contract is invalid: {message}")
            }
        }
    }
}

impl std::error::Error for BytecodeHttpFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) | Self::InvalidInput(error) => Some(error.as_ref()),
            Self::Cancelled
            | Self::DeadlineExceeded
            | Self::ResponseLimitExceeded { .. }
            | Self::InvalidProviderContract(_) => None,
        }
    }
}

/// Exact bytecode HTTP execution port. There is intentionally no SSE method.
pub trait BytecodeHttpClientPort: Send + Sync {
    fn request(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
    ) -> BytecodeHttpFuture<BytecodeHttpResponse>;

    fn stream(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
        registrar: BytecodeHttpStreamRegistrar,
    ) -> BytecodeHttpFuture<BytecodeHttpStreamResponse>;
}

pub(crate) type SharedBytecodeHttpClientPort = Arc<dyn BytecodeHttpClientPort>;

#[cfg(test)]
mod tests {
    use std::{
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    use skiff_runtime_scheduler::{BytecodeSchedulerPorts, RequestExecutionContext};
    use skiff_runtime_vm::VmFiber;

    use super::*;

    struct EndSource;

    impl StreamPullSource for EndSource {
        fn next<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
            Box::pin(async { Ok(None) })
        }
    }

    struct PendingSource;

    impl StreamPullSource for PendingSource {
        fn next<'a>(
            &'a mut self,
        ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<Option<Value>>> + Send + 'a>> {
            Box::pin(std::future::pending())
        }
    }

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn context() -> RequestExecutionContext<VmFiber> {
        RequestExecutionContext::create(BytecodeSchedulerPorts::default())
    }

    #[test]
    fn registrar_emits_numeric_exact_route_and_claims_it_once() {
        let context = context();
        let resources = context.resource_table();
        let registrar = BytecodeHttpStreamRegistrar::new(resources.clone());
        let cancellation = CancellationToken::new();
        let token = registrar
            .stream_runtime()
            .pull_stream_with_cancellation(EndSource, cancellation.clone());

        assert!(token.is_number());
        let handle = registrar.take_exact_route(token.clone()).unwrap();
        assert!(resources.validate(&handle).is_ok());
        assert!(matches!(
            registrar.take_exact_route(token),
            Err(BytecodeHttpFailure::InvalidProviderContract(_))
        ));

        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
        assert!(cancellation.is_cancelled());
    }

    #[test]
    fn outstanding_pull_is_single_consumer_and_termination_cancels_without_revival() {
        let cancellation = CancellationToken::new();
        let source = CapabilityByteStreamSource::new(Box::new(PendingSource), cancellation.clone());
        let shared = Arc::clone(&source.shared);
        let mut pull = source.start_pull().unwrap();
        assert!(matches!(
            source.start_pull(),
            Err(RequestByteStreamPullStartError::PullInProgress)
        ));
        let waker = Waker::from(Arc::new(NoopWake));
        let mut task = Context::from_waker(&waker);
        assert!(matches!(pull.as_mut().poll(&mut task), Poll::Pending));

        Box::new(source).terminate(RequestResourceTermination::Cancelled);
        assert!(cancellation.is_cancelled());
        assert!(matches!(
            pull.as_mut().poll(&mut task),
            Poll::Ready(Err(RequestByteStreamFailure::Cancelled))
        ));
        assert!(shared.terminated.load(Ordering::Acquire));
        assert!(shared
            .source
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_none());
    }

    #[test]
    fn http_port_remains_object_safe() {
        fn accepts(_port: Option<Arc<dyn BytecodeHttpClientPort>>) {}
        accepts(None);
    }
}

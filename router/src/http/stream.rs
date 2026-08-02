//! Streamed HTTP response writer (TS `httpStreamResponseWriter.ts` parity):
//! serial event ordering, cumulative response ceiling, bounded backpressure
//! with a drain deadline, and terminal propagation into the dispatch port.

use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use hyper::body::{Body, Frame};
use skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader;
use tokio::sync::mpsc;

use super::dispatch::{CancelSignal, PendingTerminalSource};
use super::error::HttpError;

/// Default bound for the streaming event channel between the dispatcher and
/// the HTTP writer (bounded mailbox; plan §3.8).
pub const DEFAULT_STREAM_CHANNEL_CAPACITY: usize = 32;

/// Terminal source of one streamed dispatch (C-dispatch §4.2 word list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpStreamErrorSource {
    ClientDisconnect,
    Backpressure,
    ResponseTooLarge,
    CallbackError,
    ProtocolError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStreamError {
    pub source: HttpStreamErrorSource,
    pub message: String,
}

impl HttpStreamError {
    pub fn new(source: HttpStreamErrorSource, message: impl Into<String>) -> Self {
        Self {
            source,
            message: message.into(),
        }
    }

    pub fn terminal_source(&self) -> PendingTerminalSource {
        match self.source {
            HttpStreamErrorSource::ClientDisconnect => PendingTerminalSource::ClientDisconnect,
            HttpStreamErrorSource::Backpressure => PendingTerminalSource::Backpressure,
            HttpStreamErrorSource::ResponseTooLarge
            | HttpStreamErrorSource::CallbackError
            | HttpStreamErrorSource::ProtocolError => PendingTerminalSource::CallbackError,
        }
    }
}

impl fmt::Display for HttpStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.source, self.message)
    }
}

impl std::error::Error for HttpStreamError {}

/// Stream event sink consumed by the dispatch port.
#[async_trait]
pub trait HttpStreamSink: Send + Sync {
    async fn enqueue_start(
        &self,
        http_response: RuntimeHttpResponseFrameHeader,
    ) -> Result<(), HttpStreamError>;
    async fn enqueue_chunk(&self, payload: Bytes) -> Result<(), HttpStreamError>;
    async fn enqueue_end(&self) -> Result<(), HttpStreamError>;

    /// Closes the event channel after the terminal (also ends the HTTP body).
    fn close(&self);
}

pub(crate) enum StreamMessage {
    Start(RuntimeHttpResponseFrameHeader),
    Chunk(Bytes),
    End,
}

struct SinkState {
    started: bool,
    ended: bool,
    response_bytes: usize,
}

/// Bounded channel-backed stream sink with ceiling and drain deadline.
pub struct ChannelStreamSink {
    tx: Mutex<Option<mpsc::Sender<StreamMessage>>>,
    max_response_bytes: usize,
    drain_timeout: Duration,
    state: Mutex<SinkState>,
    closed: AtomicBool,
    cancel: CancelSignal,
}

impl ChannelStreamSink {
    pub(crate) fn channel(
        capacity: usize,
        max_response_bytes: usize,
        drain_timeout: Duration,
        cancel: CancelSignal,
    ) -> (Arc<Self>, mpsc::Receiver<StreamMessage>) {
        let (tx, rx) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                tx: Mutex::new(Some(tx)),
                max_response_bytes,
                drain_timeout,
                state: Mutex::new(SinkState {
                    started: false,
                    ended: false,
                    response_bytes: 0,
                }),
                closed: AtomicBool::new(false),
                cancel,
            }),
            rx,
        )
    }

    fn with_state(
        &self,
        update: impl FnOnce(&mut SinkState) -> Result<(), HttpStreamError>,
    ) -> Result<(), HttpStreamError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(HttpStreamError::new(
                HttpStreamErrorSource::CallbackError,
                "HTTP stream writer is closed",
            ));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut state)
    }

    async fn send(&self, message: StreamMessage) -> Result<(), HttpStreamError> {
        let sender = self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let Some(sender) = sender else {
            return Err(HttpStreamError::new(
                HttpStreamErrorSource::CallbackError,
                "HTTP stream writer is closed",
            ));
        };
        match tokio::time::timeout(self.drain_timeout, sender.send(message)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => {
                self.cancel.cancel(
                    skiff_runtime_transport::cancel_reason::RequestCancelReason::ClientDisconnect,
                );
                Err(HttpStreamError::new(
                    HttpStreamErrorSource::ClientDisconnect,
                    "HTTP client disconnected while writing the stream",
                ))
            }
            Err(_) => Err(HttpStreamError::new(
                HttpStreamErrorSource::Backpressure,
                "HTTP response drain timed out",
            )),
        }
    }
}

#[async_trait]
impl HttpStreamSink for ChannelStreamSink {
    async fn enqueue_start(
        &self,
        http_response: RuntimeHttpResponseFrameHeader,
    ) -> Result<(), HttpStreamError> {
        self.with_state(|state| {
            if state.started {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ProtocolError,
                    "duplicate response.start received",
                ));
            }
            state.started = true;
            Ok(())
        })?;
        self.send(StreamMessage::Start(http_response)).await
    }

    async fn enqueue_chunk(&self, payload: Bytes) -> Result<(), HttpStreamError> {
        self.with_state(|state| {
            if !state.started {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ProtocolError,
                    "response.chunk received before response.start",
                ));
            }
            if state.ended {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ProtocolError,
                    "response.chunk received after response.end",
                ));
            }
            state.response_bytes =
                state
                    .response_bytes
                    .checked_add(payload.len())
                    .ok_or_else(|| {
                        HttpStreamError::new(
                            HttpStreamErrorSource::ResponseTooLarge,
                            "runtime response byte count overflow",
                        )
                    })?;
            if state.response_bytes > self.max_response_bytes {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ResponseTooLarge,
                    format!("runtime response exceeds {} bytes", self.max_response_bytes),
                ));
            }
            Ok(())
        })?;
        self.send(StreamMessage::Chunk(payload)).await
    }

    async fn enqueue_end(&self) -> Result<(), HttpStreamError> {
        self.with_state(|state| {
            if !state.started {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ProtocolError,
                    "response.end received before response.start",
                ));
            }
            if state.ended {
                return Err(HttpStreamError::new(
                    HttpStreamErrorSource::ProtocolError,
                    "duplicate response.end received",
                ));
            }
            state.ended = true;
            Ok(())
        })?;
        self.send(StreamMessage::End).await
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

/// Hyper body for one streamed response; ends on `response.end` or channel
/// close. Fires client-disconnect cancellation when the client drops the
/// connection and hyper drops this body.
pub struct ChannelStreamBody {
    rx: mpsc::Receiver<StreamMessage>,
    cancel: CancelSignal,
    cancelled_on_drop: bool,
}

impl ChannelStreamBody {
    pub(crate) fn new(rx: mpsc::Receiver<StreamMessage>, cancel: CancelSignal) -> Self {
        Self {
            rx,
            cancel,
            cancelled_on_drop: false,
        }
    }
}

impl Drop for ChannelStreamBody {
    fn drop(&mut self) {
        if !self.cancelled_on_drop {
            self.cancel.cancel(
                skiff_runtime_transport::cancel_reason::RequestCancelReason::ClientDisconnect,
            );
        }
    }
}

impl Body for ChannelStreamBody {
    type Data = Bytes;
    type Error = std::convert::Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.rx.poll_recv(context) {
            Poll::Ready(Some(StreamMessage::Start(_))) => {
                Poll::Ready(Some(Ok(Frame::data(Bytes::new()))))
            }
            Poll::Ready(Some(StreamMessage::Chunk(payload))) => {
                Poll::Ready(Some(Ok(Frame::data(payload))))
            }
            Poll::Ready(Some(StreamMessage::End)) | Poll::Ready(None) => {
                self.cancelled_on_drop = true;
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Converts an HTTP mapping failure into the canonical platform error.
pub fn stream_error_to_http_error(error: HttpStreamError) -> HttpError {
    match error.source {
        HttpStreamErrorSource::ResponseTooLarge => {
            HttpError::platform(502, "ResponseTooLarge", error.message, None)
        }
        HttpStreamErrorSource::ClientDisconnect => {
            HttpError::provider_unavailable("HTTP client disconnected before response.start")
        }
        _ => HttpError::platform(502, "InvalidHttpResponse", error.message, None),
    }
}

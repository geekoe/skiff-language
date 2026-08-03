//! HTTP → dispatcher port (C-dispatch §7.2 typed inputs/outputs).
//!
//! W-http owns the HTTP-side shape of the port and the fake seam;
//! W-dispatch implements the production dispatcher against the same contract.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_request_contract::OpaqueServiceError;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::{
    RuntimeHttpNameValueFrameHeader, ResponseErrorFrameHeader,
};
use skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestStartFrameHeader;
use tokio::sync::watch;

use super::stream::HttpStreamSink;

/// Ordinary request dispatch input (C-dispatch §7.2).
#[derive(Debug, Clone)]
pub struct DispatchRequest {
    pub header: RuntimeAssemblyRequestStartFrameHeader,
    pub payload_bytes: Bytes,
    pub timeout: Duration,
    pub client_disconnect: CancelWatch,
}

/// Unary dispatch output: canonical HTTP phase of `response.end`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnaryHttpResponse {
    pub status: u16,
    pub headers: Vec<RuntimeHttpNameValueFrameHeader>,
    pub payload: Bytes,
}

/// One runtime frame outcome of a test dispatch (plan §7 E-http; TS
/// `dispatchAssemblyTestBinary` parity). The control endpoint re-emits
/// runtime `response.end` / `response.error` frames verbatim as HTTP 200;
/// only dispatcher-level failures are control errors.
#[derive(Debug, Clone, PartialEq)]
pub enum TestDispatchOutcome {
    End(UnaryHttpResponse),
    /// The exact runtime `response.error` frame header and its opaque
    /// payload bytes (fixed-service payload; empty for control errors).
    Error(ResponseErrorFrameHeader, Bytes),
}

/// Terminal sources of ordinary requests (C-dispatch §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingTerminalSource {
    RuntimeResponseEnd,
    RuntimeResponseError,
    RuntimeRequestCancel,
    Timeout,
    CallerAbort,
    ClientDisconnect,
    Backpressure,
    ProtocolError,
    CallbackError,
    RuntimeDisconnect,
    RouterShutdown,
}

impl PendingTerminalSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RuntimeResponseEnd => "runtime_response_end",
            Self::RuntimeResponseError => "runtime_response_error",
            Self::RuntimeRequestCancel => "runtime_request_cancel",
            Self::Timeout => "timeout",
            Self::CallerAbort => "caller_abort",
            Self::ClientDisconnect => "client_disconnect",
            Self::Backpressure => "backpressure",
            Self::ProtocolError => "protocol_error",
            Self::CallbackError => "callback_error",
            Self::RuntimeDisconnect => "runtime_disconnect",
            Self::RouterShutdown => "router_shutdown",
        }
    }
}

/// Cancel-frame rule (C-dispatch §4.3): sources that must not send a frame.
pub fn cancel_reason_for_terminal(source: PendingTerminalSource) -> Option<RequestCancelReason> {
    match source {
        PendingTerminalSource::RuntimeResponseEnd
        | PendingTerminalSource::RuntimeResponseError
        | PendingTerminalSource::RuntimeRequestCancel
        | PendingTerminalSource::RuntimeDisconnect => None,
        PendingTerminalSource::Timeout => Some(RequestCancelReason::Timeout),
        PendingTerminalSource::CallerAbort => Some(RequestCancelReason::CallerCancel),
        PendingTerminalSource::ClientDisconnect => Some(RequestCancelReason::ClientDisconnect),
        PendingTerminalSource::Backpressure => Some(RequestCancelReason::Backpressure),
        PendingTerminalSource::ProtocolError | PendingTerminalSource::CallbackError => {
            Some(RequestCancelReason::ProtocolError)
        }
        PendingTerminalSource::RouterShutdown => Some(RequestCancelReason::RouterShutdown),
    }
}

/// Dispatch failure projected to the HTTP platform error mapping.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpDispatchError {
    Control {
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    },
    FixedService(OpaqueServiceError),
    Timeout {
        timeout_ms: u64,
    },
    Cancelled {
        source: PendingTerminalSource,
        message: String,
    },
}

/// The HTTP dispatch port consumed by W-http.
#[async_trait]
pub trait HttpDispatchPort: Send + Sync {
    async fn dispatch_unary(
        &self,
        request: DispatchRequest,
    ) -> Result<UnaryHttpResponse, HttpDispatchError>;

    async fn dispatch_stream(
        &self,
        request: DispatchRequest,
        sink: Arc<dyn HttpStreamSink>,
    ) -> Result<(), HttpDispatchError>;

    /// Test-dispatch seam (TS `dispatchAssemblyTestBinary` parity): returns
    /// the exact runtime frame outcome so the control endpoint can re-emit
    /// `response.end` / `response.error` frames without conflating them with
    /// dispatcher-level rejections.
    async fn dispatch_test(
        &self,
        request: DispatchRequest,
    ) -> Result<TestDispatchOutcome, HttpDispatchError>;
}

/// Client-disconnect cancellation signal (HTTP side) and watch (dispatch
/// side). Reasons follow the CONTRACT_H wire word list.
#[derive(Clone, Debug)]
pub struct CancelSignal {
    tx: watch::Sender<Option<RequestCancelReason>>,
}

impl CancelSignal {
    pub fn cancel(&self, reason: RequestCancelReason) {
        if self.tx.send(Some(reason)).is_err() {
            // Receiver already dropped; cancellation is best-effort.
        }
    }
}

#[derive(Clone, Debug)]
pub struct CancelWatch {
    rx: watch::Receiver<Option<RequestCancelReason>>,
}

impl CancelWatch {
    pub fn is_cancelled(&self) -> bool {
        self.rx.borrow().is_some()
    }

    pub fn reason(&self) -> Option<RequestCancelReason> {
        *self.rx.borrow()
    }

    /// Waits until cancellation or the watch is closed.
    pub async fn wait(mut self) -> Option<RequestCancelReason> {
        loop {
            if let Some(reason) = *self.rx.borrow() {
                return Some(reason);
            }
            if self.rx.changed().await.is_err() {
                return None;
            }
        }
    }
}

pub fn cancel_channel() -> (CancelSignal, CancelWatch) {
    let (tx, rx) = watch::channel(None);
    (CancelSignal { tx }, CancelWatch { rx })
}

/// Fires `client_disconnect` when dropped (service future aborted by hyper
/// on client disconnect). Defuse after the response is handed to hyper.
pub struct CancelOnDrop {
    signal: Option<CancelSignal>,
}

impl CancelOnDrop {
    pub fn new(signal: CancelSignal) -> Self {
        Self {
            signal: Some(signal),
        }
    }

    pub fn defuse(&mut self) {
        self.signal = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(signal) = &self.signal {
            signal.cancel(RequestCancelReason::ClientDisconnect);
        }
    }
}

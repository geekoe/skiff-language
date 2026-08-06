//! Production HTTP composition: deployment-record HTTP surface, the
//! `HttpDispatchPort` ↔ `RequestDispatcher` adapter (contract
//! `DispatchRequest { header, payload_bytes, timeout, cancel_signal }` →
//! `DispatchSubmit` with timeout/cancel conversion) and the request-family
//! inbound sink that relays dispatcher outcomes to the awaiting HTTP phase.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::{
    decode_request_cancel_frame, decode_response_chunk_frame, decode_response_end_frame,
    decode_response_error_frame, decode_response_start_frame, ResponseErrorFrameHeader,
    RuntimeHttpResponseFrameHeader,
};
use skiff_runtime_transport::runtime_assembly_request::{
    decode_runtime_assembly_websocket_connect_response_end_frame,
    decode_runtime_assembly_websocket_jsonrpc_response_end_frame,
    RuntimeAssemblyWebSocketConnectResponseFrameHeader,
    RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader,
};
use tokio::sync::mpsc;

use crate::dispatch::{
    DispatchSubmit, DispatchedFrame, PendingTerminal, RequestDispatcher, RequestOutcome,
    RuntimeResponseFrame, SubmitRejectReason, SubmitResult, TerminalSource,
};
use crate::http::dispatch::{
    cancel_reason_for_terminal, DispatchRequest, HttpDispatchError, HttpDispatchPort,
    PendingTerminalSource, TestDispatchOutcome, UnaryHttpResponse,
};
use crate::http::ingress::HttpGatewaySurfaceView;
use crate::http::stream::{HttpStreamError, HttpStreamSink};
use crate::session::demux::InboundFrameSink;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::TerminalKind;
use crate::supervisor::ws::{ConnectOutcome, WsDispatchStore};
use crate::ws::OverflowPolicy;

/// Default per-request dispatch event channel capacity. The inbound runtime
/// frame budget is 64 frames per session (C-session §5.3), so a channel that
/// mirrors it never needs to block the session task; overflow fails closed
/// through the dispatcher backpressure terminal.
pub const DISPATCH_EVENT_CHANNEL_CAPACITY: usize = 64;

/// One dispatcher outcome forwarded from the request-family sink to the
/// awaiting HTTP phase.
#[derive(Debug, Clone, PartialEq)]
pub enum HttpDispatchEvent {
    Frame {
        frame: DispatchedFrame,
        http: Option<RuntimeHttpResponseFrameHeader>,
    },
    Terminal {
        terminal: PendingTerminal,
    },
}

/// Per-request correlation router (composition owner; no pending duplication
/// with `RequestDispatcher` — this table only routes already-dispatched
/// outcomes to the HTTP phase).
#[derive(Debug)]
pub struct PendingHttpRouter {
    pending: Mutex<BTreeMap<String, mpsc::Sender<HttpDispatchEvent>>>,
    overflow_terminals: AtomicU64,
}

impl Default for PendingHttpRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingHttpRouter {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
            overflow_terminals: AtomicU64::new(0),
        }
    }

    pub fn register(&self, request_id: &str) -> Result<mpsc::Receiver<HttpDispatchEvent>, String> {
        let (tx, rx) = mpsc::channel(DISPATCH_EVENT_CHANNEL_CAPACITY);
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pending.insert(request_id.to_string(), tx).is_some() {
            return Err(format!(
                "pending HTTP dispatch correlation already exists for {request_id}"
            ));
        }
        Ok(rx)
    }

    pub fn unregister(&self, request_id: &str) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(request_id);
    }

    /// Non-blocking delivery. `false` means the channel is full or the HTTP
    /// phase is gone: the caller (request sink) must fail the exact pending
    /// through the dispatcher (backpressure terminal).
    pub fn deliver(&self, request_id: &str, event: HttpDispatchEvent) -> bool {
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(request_id)
            .cloned();
        match sender {
            Some(sender) => match sender.try_send(event) {
                Ok(()) => true,
                Err(_) => {
                    self.overflow_terminals.fetch_add(1, Ordering::Relaxed);
                    false
                }
            },
            None => false,
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    pub fn overflow_terminal_count(&self) -> u64 {
        self.overflow_terminals.load(Ordering::Relaxed)
    }
}

/// Request-family inbound sink (plan §5.5): decodes one Runtime→Router
/// response/cancel frame, drives the production `RequestDispatcher`, and
/// relays accepted frames/terminals to the awaiting HTTP phase.
#[derive(Debug, Clone)]
pub struct RequestFrameSink {
    dispatcher: Arc<RequestDispatcher>,
    router: Arc<PendingHttpRouter>,
    ws: Option<Arc<WsDispatchStore>>,
}

impl RequestFrameSink {
    pub fn new(dispatcher: Arc<RequestDispatcher>, router: Arc<PendingHttpRouter>) -> Self {
        Self::new_with_ws(dispatcher, router, None)
    }

    /// E-ws additive constructor: additionally routes websocketConnect /
    /// websocketJsonRpc response frames through the WS dispatch store.
    pub fn new_with_ws(
        dispatcher: Arc<RequestDispatcher>,
        router: Arc<PendingHttpRouter>,
        ws: Option<Arc<WsDispatchStore>>,
    ) -> Self {
        Self {
            dispatcher,
            router,
            ws,
        }
    }
}

impl InboundFrameSink for RequestFrameSink {
    fn family(&self) -> skiff_runtime_transport::protocol::RuntimeFrameFamily {
        skiff_runtime_transport::protocol::RuntimeFrameFamily::Request
    }

    fn accepts_frame_type(&self, frame_type: &str) -> bool {
        matches!(
            frame_type,
            "response.start" | "response.chunk" | "response.end" | "response.error"
        )
    }

    fn handle(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind> {
        if let Some(ws) = &self.ws {
            if let Ok((header, error)) = decode_response_error_frame(raw) {
                // A runtime-side error settles the websocketConnect
                // correlation as unavailable (fail fast). The settle is a
                // no-op for ordinary/JSON-RPC request ids.
                let reason = match &error {
                    skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(
                        error,
                    ) => match error.envelope() {
                        skiff_runtime_request_contract::service_error::ServiceErrorEnvelope::PublicTypedError {
                            package_id,
                            stable_schema_key,
                            ..
                        } => format!("service error {package_id}:{stable_schema_key}"),
                        skiff_runtime_request_contract::service_error::ServiceErrorEnvelope::InternalError {
                            payload,
                            ..
                        } => payload.message.clone(),
                        skiff_runtime_request_contract::service_error::ServiceErrorEnvelope::PlatformError {
                            builtin_error_identity,
                            ..
                        } => format!("platform error {builtin_error_identity:?}"),
                    },
                    skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(
                        payload,
                    ) => payload.message.clone(),
                };
                ws.connect_unavailable(header.request_id(), reason);
            }
            // WS connect admission response (E-ws): settle the connect
            // correlation; no ordinary dispatcher pending is involved.
            if let Ok(header) = decode_runtime_assembly_websocket_connect_response_end_frame(raw) {
                let outcome = match header.websocket_connect {
                    RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
                        business_identity,
                        admission_rank,
                        connection_policy,
                    } => {
                        let (max_connections, overflow, close_code, close_reason) =
                            match connection_policy {
                                Some(policy) => (
                                    policy.max_connections.get(),
                                    match policy.overflow {
                                        RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader::CloseOldest => {
                                            OverflowPolicy::CloseOldest
                                        }
                                        RuntimeAssemblyWebSocketConnectionPolicyOverflowFrameHeader::RejectNew => {
                                            OverflowPolicy::RejectNew
                                        }
                                    },
                                    policy.close_code,
                                    policy.close_reason,
                                ),
                                None => (
                                    u32::MAX,
                                    OverflowPolicy::RejectNew,
                                    None,
                                    None,
                                ),
                            };
                        ConnectOutcome::Accepted {
                            business_identity,
                            admission_rank,
                            max_connections,
                            overflow,
                            close_code,
                            close_reason,
                        }
                    }
                    RuntimeAssemblyWebSocketConnectResponseFrameHeader::Reject { code, reason } => {
                        ConnectOutcome::Rejected { code, reason }
                    }
                };
                ws.connect_response(&header.request_id, outcome);
                return Ok(());
            }
            // WS inbound JSON-RPC response (E-ws): the broker owns the peer
            // terminal; the store owns the runtime correlation.
            if let Ok((header, payload)) =
                decode_runtime_assembly_websocket_jsonrpc_response_end_frame(raw)
            {
                ws.on_inbound_response(
                    &header.request_id,
                    header.websocket_json_rpc.outcome,
                    payload,
                );
                return Ok(());
            }
        }
        let (frame, http) = decode_request_family_frame(raw)?;
        let request_id = frame.request_id().to_string();
        if self.dispatcher.is_task_attempt(&request_id) {
            // Durable task attempt terminal: the dispatcher already returned
            // it to the task control plane through the settlement port.
            // There is no HTTP phase; forwarding/backpressure would abort a
            // task frame as a client backpressure terminal.
            let _ = self.dispatcher.on_frame(session, frame);
            return Ok(());
        }
        if let Some(ws) = &self.ws {
            match &frame {
                RuntimeResponseFrame::Error { .. } => {
                    if ws.has_inbound(&request_id) {
                        ws.on_inbound_terminal(
                            &request_id,
                            crate::ws::InboundDispatchResult::InternalError,
                        );
                        return Ok(());
                    }
                }
                RuntimeResponseFrame::Cancel { .. } => {
                    if ws.has_inbound(&request_id) {
                        ws.on_inbound_terminal(
                            &request_id,
                            crate::ws::InboundDispatchResult::RuntimeUnavailable,
                        );
                        return Ok(());
                    }
                }
                _ => {}
            }
        }
        let outcome = self.dispatcher.on_frame(session, frame);
        for dispatched in outcome.frames {
            let request_id = dispatched.request_id().to_string();
            let event = HttpDispatchEvent::Frame {
                frame: dispatched.clone(),
                http: http
                    .as_ref()
                    .filter(|_| matches!(dispatched, DispatchedFrame::Start { .. }))
                    .cloned()
                    .or_else(|| match &dispatched {
                        DispatchedFrame::End { .. } => http.clone(),
                        _ => None,
                    }),
            };
            if !self.router.deliver(&request_id, event) {
                self.dispatcher.backpressure(&request_id);
            }
        }
        for terminal in outcome.terminals {
            let request_id = terminal.request_id.clone();
            if !self
                .router
                .deliver(&request_id, HttpDispatchEvent::Terminal { terminal })
            {
                // The HTTP phase is gone; the terminal was already counted by
                // the dispatcher and its permit released.
            }
        }
        Ok(())
    }
}

fn decode_request_family_frame(
    raw: &[u8],
) -> Result<(RuntimeResponseFrame, Option<RuntimeHttpResponseFrameHeader>), TerminalKind> {
    let decoded = skiff_runtime_transport::protocol::decode_binary_frame(raw)
        .map_err(|_| TerminalKind::MalformedFrame)?;
    let frame_type = decoded
        .header
        .get("type")
        .and_then(|value| value.as_str())
        .ok_or(TerminalKind::MalformedFrame)?;
    match frame_type {
        "response.start" => {
            let header =
                decode_response_start_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
            Ok((
                RuntimeResponseFrame::Start {
                    request_id: header.request_id,
                },
                Some(header.http_response),
            ))
        }
        "response.chunk" => {
            let (header, payload) =
                decode_response_chunk_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
            Ok((
                RuntimeResponseFrame::Chunk {
                    request_id: header.request_id,
                    seq: header.seq,
                    payload,
                },
                None,
            ))
        }
        "response.end" => {
            let (header, payload) =
                decode_response_end_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
            let http = match &header.metadata {
                skiff_runtime_transport::protocol::ResponseEndFrameMetadata::Http(http) => {
                    Some(http.clone())
                }
                skiff_runtime_transport::protocol::ResponseEndFrameMetadata::None => None,
            };
            Ok((
                RuntimeResponseFrame::End {
                    request_id: header.request_id,
                    payload_present: header.payload_present,
                    payload,
                },
                http,
            ))
        }
        "response.error" => {
            let (header, error) =
                decode_response_error_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
            let request_id = header.request_id().to_string();
            Ok((RuntimeResponseFrame::Error { request_id, error }, None))
        }
        "request.cancel" => {
            let header =
                decode_request_cancel_frame(raw).map_err(|_| TerminalKind::MalformedFrame)?;
            Ok((
                RuntimeResponseFrame::Cancel {
                    request_id: header.request_id,
                    reason: header.reason,
                },
                None,
            ))
        }
        _ => Err(TerminalKind::MalformedFrame),
    }
}

impl DispatchedFrame {
    fn request_id(&self) -> &str {
        match self {
            Self::Start { request_id }
            | Self::Chunk { request_id, .. }
            | Self::End { request_id, .. }
            | Self::Error { request_id, .. } => request_id,
        }
    }
}

/// Builds the typed HTTP surface view from the deployment records referenced
/// by the captured epoch (deployment crate consumed read-only; no deployment
/// files are written). Only entries with an HTTP protocol surface are
/// projected; WebSocket entries belong to the WS surface view
/// (`load_ws_surface_view`), mirroring the TS HTTP/WebSocket surface split.
/// Surfaces are keyed by (deployment, gateway entry key): the same gateway
/// entry key may legally exist in multiple deployments (for example
/// `v1ModelsGet` in both agine.ai/aihub and agine.ai/codex-relay), and each
/// request resolves its surface through the exact service selector binding.
pub fn load_http_surface_view(
    artifact_root: &Path,
    profile: &str,
) -> Result<HttpGatewaySurfaceView, String> {
    let store = CanonicalArtifactStore::open(artifact_root)
        .map_err(|error| format!("open artifact store for HTTP surface: {error}"))?;
    crate::http::ingress::http_surface_view_from_pointers(&store, profile)
}

/// Production `HttpDispatchPort` over `RequestDispatcher` (C-dispatch §7.2).
///
/// Converts the contract `DispatchRequest` into the dispatcher's
/// `DispatchSubmit` (ordinary HTTP admission has no `prefer_session` hint),
/// awaits dispatcher outcomes through the [`PendingHttpRouter`], applies the
/// HTTP request deadline and the client-disconnect cancellation signal, and
/// maps dispatcher terminals to the HTTP platform error vocabulary.
#[derive(Debug)]
pub struct DispatcherHttpPort {
    dispatcher: Arc<RequestDispatcher>,
    router: Arc<PendingHttpRouter>,
    request_timeout: Duration,
}

impl DispatcherHttpPort {
    pub fn new(
        dispatcher: Arc<RequestDispatcher>,
        router: Arc<PendingHttpRouter>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            dispatcher,
            router,
            request_timeout,
        }
    }

    fn rejected(&self, request_id: &str, reason: SubmitRejectReason) -> HttpDispatchError {
        match reason {
            SubmitRejectReason::DeadlineExpired => HttpDispatchError::Timeout {
                timeout_ms: self.request_timeout.as_millis() as u64,
            },
            SubmitRejectReason::Duplicate => HttpDispatchError::Control {
                code: "DuplicateRequest".to_string(),
                message: format!("request id {request_id} is already pending"),
                status: Some(409),
                details: None,
            },
            SubmitRejectReason::NoCandidate
            | SubmitRejectReason::QueueFull
            | SubmitRejectReason::RevalidateFailClosed
            | SubmitRejectReason::Shutdown => HttpDispatchError::Control {
                code: "ServiceUnavailable".to_string(),
                message: format!("no eligible runtime for request {request_id}: {reason:?}"),
                status: Some(503),
                details: None,
            },
            SubmitRejectReason::InvalidMode | SubmitRejectReason::CallbackError => {
                HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::CallbackError,
                    message: format!("dispatch rejected: {reason:?}"),
                }
            }
        }
    }

    fn terminal_error(&self, terminal: PendingTerminal) -> HttpDispatchError {
        match terminal.outcome {
            RequestOutcome::Completed => HttpDispatchError::Cancelled {
                source: PendingTerminalSource::ProtocolError,
                message: "unary dispatch completed without a response.end frame".to_string(),
            },
            RequestOutcome::Failed => HttpDispatchError::Cancelled {
                source: PendingTerminalSource::ProtocolError,
                message: "response.error terminal without a decoded error frame".to_string(),
            },
            RequestOutcome::Cancelled => HttpDispatchError::Cancelled {
                source: terminal_source_to_http(terminal.source),
                message: terminal.source.as_str().to_string(),
            },
            RequestOutcome::ProtocolError => HttpDispatchError::Cancelled {
                source: PendingTerminalSource::ProtocolError,
                message: "runtime response protocol error".to_string(),
            },
        }
    }

    fn sink_error(&self, request_id: &str, error: HttpStreamError) -> HttpDispatchError {
        let source = error.terminal_source();
        match source {
            PendingTerminalSource::Backpressure => {
                self.dispatcher.backpressure(request_id);
            }
            PendingTerminalSource::ClientDisconnect => {
                self.dispatcher.client_disconnect(request_id);
            }
            _ => {
                self.dispatcher
                    .caller_abort(request_id, Some("protocol_error"));
            }
        }
        self.router.unregister(request_id);
        HttpDispatchError::Cancelled {
            source,
            message: error.message,
        }
    }
}

fn terminal_source_to_http(source: TerminalSource) -> PendingTerminalSource {
    match source {
        TerminalSource::RuntimeResponseEnd => PendingTerminalSource::RuntimeResponseEnd,
        TerminalSource::RuntimeResponseError => PendingTerminalSource::RuntimeResponseError,
        TerminalSource::RuntimeRequestCancel => PendingTerminalSource::RuntimeRequestCancel,
        TerminalSource::Timeout => PendingTerminalSource::Timeout,
        TerminalSource::CallerAbort => PendingTerminalSource::CallerAbort,
        TerminalSource::ClientDisconnect => PendingTerminalSource::ClientDisconnect,
        TerminalSource::Backpressure => PendingTerminalSource::Backpressure,
        TerminalSource::ProtocolError => PendingTerminalSource::ProtocolError,
        TerminalSource::CallbackError => PendingTerminalSource::CallbackError,
        TerminalSource::RuntimeDisconnect => PendingTerminalSource::RuntimeDisconnect,
        TerminalSource::RouterShutdown => PendingTerminalSource::RouterShutdown,
    }
}

#[async_trait]
impl HttpDispatchPort for DispatcherHttpPort {
    async fn dispatch_unary(
        &self,
        request: DispatchRequest,
    ) -> Result<UnaryHttpResponse, HttpDispatchError> {
        let request_id = request.header.request_id.clone();
        let submit = dispatch_submit_from_request(&request);
        let accepted = match self.dispatcher.submit(submit) {
            SubmitResult::Accepted { .. } => true,
            SubmitResult::Rejected { reason, .. } => {
                return Err(self.rejected(&request_id, reason));
            }
        };
        debug_assert!(accepted);
        let mut rx =
            self.router
                .register(&request_id)
                .map_err(|message| HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::CallbackError,
                    message,
                })?;
        let timeout_ms = request.timeout.as_millis() as u64;
        let result = tokio::select! {
            biased;
            reason = request.client_disconnect.clone().wait() => {
                let _ = reason;
                self.dispatcher.client_disconnect(&request_id);
                self.router.unregister(&request_id);
                Err(HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::ClientDisconnect,
                    message: "client disconnected before dispatch terminal".to_string(),
                })
            }
            _ = tokio::time::sleep(request.timeout) => {
                self.dispatcher.timeout(&request_id);
                self.router.unregister(&request_id);
                Err(HttpDispatchError::Timeout { timeout_ms })
            }
            event = rx.recv() => {
                match event {
                    Some(HttpDispatchEvent::Frame {
                        frame: DispatchedFrame::End { payload, .. },
                        http: Some(http),
                    }) => {
                        self.router.unregister(&request_id);
                        Ok(UnaryHttpResponse {
                            status: http.status,
                            headers: http.headers,
                            payload: Bytes::from(payload),
                        })
                    }
                    Some(HttpDispatchEvent::Frame {
                        frame: DispatchedFrame::Error { error, .. },
                        ..
                    }) => {
                        match error {
                            skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(error) => {
                                Err(HttpDispatchError::FixedService(error))
                            }
                            skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(payload) => {
                                Err(HttpDispatchError::Control {
                                    code: payload.code,
                                    message: payload.message,
                                    status: payload.status,
                                    details: payload.details,
                                })
                            }
                        }
                    }
                    Some(HttpDispatchEvent::Terminal { terminal }) => {
                        self.router.unregister(&request_id);
                        Err(self.terminal_error(terminal))
                    }
                    Some(HttpDispatchEvent::Frame { .. }) => {
                        self.router.unregister(&request_id);
                        Err(HttpDispatchError::Cancelled {
                            source: PendingTerminalSource::ProtocolError,
                            message: "unexpected response frame for unary dispatch".to_string(),
                        })
                    }
                    None => {
                        self.router.unregister(&request_id);
                        Err(HttpDispatchError::Cancelled {
                            source: PendingTerminalSource::Backpressure,
                            message: "dispatch correlation channel closed".to_string(),
                        })
                    }
                }
            }
        };
        // A terminal already settled the pending: make sure the correlation
        // entry is gone (idempotent).
        self.router.unregister(&request_id);
        result
    }

    async fn dispatch_stream(
        &self,
        request: DispatchRequest,
        sink: Arc<dyn HttpStreamSink>,
    ) -> Result<(), HttpDispatchError> {
        let request_id = request.header.request_id.clone();
        let submit = dispatch_submit_from_request(&request);
        let accepted = match self.dispatcher.submit(submit) {
            SubmitResult::Accepted { .. } => true,
            SubmitResult::Rejected { reason, .. } => {
                return Err(self.rejected(&request_id, reason));
            }
        };
        debug_assert!(accepted);
        let mut rx =
            self.router
                .register(&request_id)
                .map_err(|message| HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::CallbackError,
                    message,
                })?;
        let result = loop {
            let event = tokio::select! {
                biased;
                reason = request.client_disconnect.clone().wait() => {
                    let _ = reason;
                    self.dispatcher.client_disconnect(&request_id);
                    self.router.unregister(&request_id);
                    break Err(HttpDispatchError::Cancelled {
                        source: PendingTerminalSource::ClientDisconnect,
                        message: "client disconnected before stream terminal".to_string(),
                    });
                }
                _ = tokio::time::sleep(request.timeout) => {
                    self.dispatcher.timeout(&request_id);
                    self.router.unregister(&request_id);
                    break Err(HttpDispatchError::Timeout {
                        timeout_ms: request.timeout.as_millis() as u64,
                    });
                }
                event = rx.recv() => event,
            };
            match event {
                Some(HttpDispatchEvent::Frame {
                    frame: DispatchedFrame::Start { .. },
                    http: Some(http),
                }) => {
                    if let Err(error) = sink.enqueue_start(http).await {
                        break Err(self.sink_error(&request_id, error));
                    }
                }
                Some(HttpDispatchEvent::Frame {
                    frame: DispatchedFrame::Start { .. },
                    http: None,
                }) => {
                    self.dispatcher
                        .caller_abort(&request_id, Some("protocol_error"));
                    self.router.unregister(&request_id);
                    break Err(HttpDispatchError::Cancelled {
                        source: PendingTerminalSource::ProtocolError,
                        message: "response.start without HTTP metadata".to_string(),
                    });
                }
                Some(HttpDispatchEvent::Frame {
                    frame: DispatchedFrame::Chunk { payload, .. },
                    ..
                }) => {
                    if let Err(error) = sink.enqueue_chunk(Bytes::from(payload)).await {
                        break Err(self.sink_error(&request_id, error));
                    }
                }
                Some(HttpDispatchEvent::Frame {
                    frame: DispatchedFrame::End { .. },
                    ..
                }) => {
                    if let Err(error) = sink.enqueue_end().await {
                        break Err(self.sink_error(&request_id, error));
                    }
                }
                Some(HttpDispatchEvent::Frame {
                    frame: DispatchedFrame::Error { error, .. },
                    ..
                }) => {
                    break Err(match error {
                        skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(error) => {
                            HttpDispatchError::FixedService(error)
                        }
                        skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(payload) => {
                            HttpDispatchError::Control {
                                code: payload.code,
                                message: payload.message,
                                status: payload.status,
                                details: payload.details,
                            }
                        }
                    });
                }
                Some(HttpDispatchEvent::Terminal { terminal }) => {
                    if terminal.outcome == RequestOutcome::Completed {
                        self.router.unregister(&request_id);
                        break Ok(());
                    }
                    self.router.unregister(&request_id);
                    break Err(self.terminal_error(terminal));
                }
                None => {
                    self.router.unregister(&request_id);
                    break Err(HttpDispatchError::Cancelled {
                        source: PendingTerminalSource::Backpressure,
                        message: "dispatch correlation channel closed".to_string(),
                    });
                }
            }
        };
        self.router.unregister(&request_id);
        result
    }

    async fn dispatch_test(
        &self,
        request: DispatchRequest,
    ) -> Result<TestDispatchOutcome, HttpDispatchError> {
        let request_id = request.header.request_id.clone();
        let submit = dispatch_submit_from_request(&request);
        let accepted = match self.dispatcher.submit(submit) {
            SubmitResult::Accepted { .. } => true,
            SubmitResult::Rejected { reason, .. } => {
                return Err(self.rejected(&request_id, reason));
            }
        };
        debug_assert!(accepted);
        let mut rx =
            self.router
                .register(&request_id)
                .map_err(|message| HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::CallbackError,
                    message,
                })?;
        let timeout_ms = request.timeout.as_millis() as u64;
        let result = tokio::select! {
            biased;
            reason = request.client_disconnect.clone().wait() => {
                let _ = reason;
                self.dispatcher.client_disconnect(&request_id);
                self.router.unregister(&request_id);
                Err(HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::ClientDisconnect,
                    message: "client disconnected before dispatch terminal".to_string(),
                })
            }
            _ = tokio::time::sleep(request.timeout) => {
                self.dispatcher.timeout(&request_id);
                self.router.unregister(&request_id);
                Err(HttpDispatchError::Timeout { timeout_ms })
            }
            event = rx.recv() => {
                match event {
                    Some(HttpDispatchEvent::Frame {
                        frame: DispatchedFrame::End { payload, .. },
                        http: Some(http),
                    }) => {
                        self.router.unregister(&request_id);
                        Ok(TestDispatchOutcome::End(UnaryHttpResponse {
                            status: http.status,
                            headers: http.headers,
                            payload: Bytes::from(payload),
                        }))
                    }
                    Some(HttpDispatchEvent::Frame {
                        frame: DispatchedFrame::Error { error, .. },
                        ..
                    }) => {
                        let outcome = match error {
                            skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::FixedService(error) => {
                                TestDispatchOutcome::Error(
                                    ResponseErrorFrameHeader::fixed_service(request_id.clone()),
                                    Bytes::from(error.into_encoded_bytes()),
                                )
                            }
                            skiff_runtime_transport::protocol::ValidatedResponseErrorFrame::Control(payload) => {
                                TestDispatchOutcome::Error(
                                    ResponseErrorFrameHeader::control(request_id.clone(), payload),
                                    Bytes::new(),
                                )
                            }
                        };
                        self.router.unregister(&request_id);
                        Ok(outcome)
                    }
                    Some(HttpDispatchEvent::Terminal { terminal }) => {
                        self.router.unregister(&request_id);
                        Err(self.terminal_error(terminal))
                    }
                    Some(HttpDispatchEvent::Frame { .. }) => {
                        self.router.unregister(&request_id);
                        Err(HttpDispatchError::Cancelled {
                            source: PendingTerminalSource::ProtocolError,
                            message: "unexpected response frame for test dispatch".to_string(),
                        })
                    }
                    None => {
                        self.router.unregister(&request_id);
                        Err(HttpDispatchError::Cancelled {
                            source: PendingTerminalSource::Backpressure,
                            message: "dispatch correlation channel closed".to_string(),
                        })
                    }
                }
            }
        };
        // A terminal already settled the pending: make sure the correlation
        // entry is gone (idempotent).
        self.router.unregister(&request_id);
        result
    }
}

/// Deadline/cancel conversion helper kept public for tests: the HTTP phase
/// deadline is applied by the adapter (`tokio::time::sleep`), and the wire
/// deadline already lives in the codec-validated `request.start` header.
pub fn request_timeout_ms(timeout: Duration) -> u64 {
    timeout.as_millis() as u64
}

/// Contract conversion (C-dispatch §7.2): `DispatchRequest { header,
/// payload_bytes, timeout, cancel_signal }` → `DispatchSubmit`. Ordinary HTTP
/// admission has no `prefer_session` hint; the wire deadline already lives in
/// the codec-validated header and the HTTP phase deadline is applied by the
/// adapter itself.
pub fn dispatch_submit_from_request(request: &DispatchRequest) -> DispatchSubmit {
    DispatchSubmit {
        header: request.header.clone(),
        payload_bytes: request.payload_bytes.to_vec(),
        prefer_session: None,
    }
}

/// Terminal-source cancel conversion exposed for composition tests.
pub fn cancel_reason(source: PendingTerminalSource) -> Option<RequestCancelReason> {
    cancel_reason_for_terminal(source)
}

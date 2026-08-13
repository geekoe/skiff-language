//! Production port adapters over the `SessionLayer` outbound-writer seam
//! (W-composition; plan §3.3/§3.6/§3.8, C-dispatch §7.7, C-activation §8,
//! C-ws §5.7, C-actor §10).
//!
//! Every adapter is synchronous and non-blocking: session writes go through
//! `SessionLayer::write_session_frame` (bounded queue, frame/byte permit),
//! session aborts go through `request_close` (cancellation watcher), and
//! session truth is only read through the directory lock. No adapter holds
//! another owner's state across `.await`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::connection_protocol::{
    encode_connection_response_frame, ConnectionResponseFrameHeader,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, encode_request_cancel_frame, RequestCancelFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};

use crate::dispatch::{CandidateViewSource, LeaseRevalidate, RevalidateOutcome};
use crate::dispatch::{
    DispatchSubmit, RequestDispatcher, RuntimePeer, SessionAbortControl, TaskAttemptSubmit,
};
use crate::routing::{
    CandidateDirectoryView, DispatchCapabilities, RegisteredSessionLease, RuntimeCandidateQuery,
};
use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::layer::{SessionCloseReason, SessionLayer};
use crate::ws::{
    BrokerRuntimeResponse, DispatchInbound, InboundDispatchAction, RuntimeResponder,
    RuntimeViolationSink,
};

use super::http::{HttpDispatchEvent, PendingHttpRouter};

/// Deferred reference to the process `SessionLayer` (composition seam).
///
/// The dispatcher/WS/activation/actor ports are constructed before the
/// session layer exists (the manifest requires their consumers at session
/// construction), so session-backed adapters resolve the layer through this
/// handle. The supervisor sets it before any listener starts; every adapter
/// fails closed if it is ever called before then.
#[derive(Debug, Clone, Default)]
pub struct SessionHandle {
    layer: Arc<Mutex<Option<Arc<SessionLayer>>>>,
}

impl SessionHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, layer: Arc<SessionLayer>) {
        *self
            .layer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(layer);
    }

    pub fn layer(&self) -> Option<Arc<SessionLayer>> {
        self.layer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Deferred reference to the composition `PendingHttpRouter`.
///
/// The dispatcher session consumer is constructed before the HTTP
/// correlation router exists (the session layer requires its consumers at
/// construction); the supervisor sets the router right after it is created.
/// Until then, close terminals have no HTTP phase to deliver to and are
/// safely dropped (the dispatcher already released the permit).
#[derive(Debug, Clone, Default)]
pub struct PendingHttpHandle {
    router: Arc<Mutex<Option<Arc<PendingHttpRouter>>>>,
}

impl PendingHttpHandle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, router: Arc<PendingHttpRouter>) {
        *self
            .router
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(router);
    }

    fn router(&self) -> Option<Arc<PendingHttpRouter>> {
        self.router
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// C-routing-query directory view source: coherent lock-held snapshot plus
/// the per-session registration facts (dispatch capabilities + build ids +
/// lazy-load advertisement) retained by the session layer, with the router's
/// own artifact root injected for the lazy-load candidate rule
/// (integration-contract-v2 §1).
#[derive(Debug, Clone)]
pub struct SessionCandidateViewSource {
    session: SessionHandle,
    router_artifact_root: Option<String>,
}

impl SessionCandidateViewSource {
    pub fn new(session: SessionHandle, router_artifact_root: Option<String>) -> Self {
        Self {
            session,
            router_artifact_root,
        }
    }
}

impl CandidateViewSource for SessionCandidateViewSource {
    fn view(&self) -> CandidateDirectoryView {
        let Some(layer) = self.session.layer() else {
            return CandidateDirectoryView {
                revision: None,
                router_artifact_root: self.router_artifact_root.clone(),
                sessions: Vec::new(),
            };
        };
        let registration_facts = layer.registration_facts_snapshot();
        let directory = layer.directory_lock();
        RuntimeCandidateQuery::snapshot_directory_view(
            &directory,
            &registration_facts,
            self.router_artifact_root.clone(),
        )
    }
}

/// Plan §3.3 step 5: atomic revalidation against the directory record before
/// enqueue (session epoch / registration revision / exact tuple /
/// cancellation).
#[derive(Debug, Clone)]
pub struct DirectoryLeaseRevalidate {
    session: SessionHandle,
}

impl DirectoryLeaseRevalidate {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl LeaseRevalidate for DirectoryLeaseRevalidate {
    fn revalidate(&self, _request_id: &str, lease: &RegisteredSessionLease) -> RevalidateOutcome {
        let Some(layer) = self.session.layer() else {
            return RevalidateOutcome::StaleRevision;
        };
        let directory = layer.directory_lock();
        let Some(record) = directory.record(&lease.session_epoch) else {
            return RevalidateOutcome::StaleRevision;
        };
        if record.cancelled {
            return RevalidateOutcome::Cancelled;
        }
        if record.registration_revision != lease.registration_revision {
            return RevalidateOutcome::StaleRevision;
        }
        RevalidateOutcome::Ok
    }
}

/// C-dispatch §7.4: abort the exact session through its cancellation watcher
/// (never waits for the writer queue).
#[derive(Debug, Clone)]
pub struct LayerSessionAbort {
    session: SessionHandle,
}

impl LayerSessionAbort {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl SessionAbortControl for LayerSessionAbort {
    fn abort_session(&self, session: &RuntimeSessionEpoch) {
        if let Some(layer) = self.session.layer() {
            layer.request_close(session, SessionCloseReason::Disconnect);
        }
    }
}

/// C-dispatch §7.7 outbound `request.start` / `request.cancel` writer over
/// the bounded session outbound registry.
#[derive(Debug, Clone)]
pub struct SessionRuntimePeer {
    session: SessionHandle,
}

impl SessionRuntimePeer {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl RuntimePeer for SessionRuntimePeer {
    fn send_request_start(
        &self,
        session: &RuntimeSessionEpoch,
        request: &DispatchSubmit,
    ) -> Result<(), String> {
        let bytes = encode_binary_frame(&request.header, &request.payload_bytes)
            .map_err(|error| format!("request.start encode failed: {error}"))?;
        write_session_frame(&self.session, session, bytes)
    }

    fn send_request_cancel(
        &self,
        session: &RuntimeSessionEpoch,
        request_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        let header = RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: request_id.to_string(),
            reason: reason.to_string(),
        };
        let bytes = encode_request_cancel_frame(&header)
            .map_err(|error| format!("request.cancel encode failed: {error}"))?;
        write_session_frame(&self.session, session, bytes)
    }

    fn send_task_attempt_start(
        &self,
        session: &RuntimeSessionEpoch,
        attempt: &TaskAttemptSubmit,
    ) -> Result<(), String> {
        let bytes = encode_binary_frame(&attempt.header, &attempt.payload)
            .map_err(|error| format!("task attempt request.start encode failed: {error}"))?;
        write_session_frame(&self.session, session, bytes)
    }
}

fn write_session_frame(
    handle: &SessionHandle,
    session: &RuntimeSessionEpoch,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let layer = handle
        .layer()
        .ok_or_else(|| "session layer is not wired yet".to_string())?;
    layer.write_session_frame(session, bytes)
}

/// C-model-connection §3.3 outbound `connection.response` writer. One
/// instance is captured per `BrokerRuntimeSource` with the exact sender.
#[derive(Debug, Clone)]
pub struct WsRuntimeResponder {
    session: SessionHandle,
    runtime: RuntimeSessionEpoch,
}

impl WsRuntimeResponder {
    pub fn new(session: SessionHandle, runtime: RuntimeSessionEpoch) -> Self {
        Self { session, runtime }
    }
}

impl RuntimeResponder for WsRuntimeResponder {
    fn respond(&self, response: &BrokerRuntimeResponse) -> Result<(), String> {
        let header = ConnectionResponseFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "connection.response".to_string(),
            request_id: response.request_id.clone(),
            outcome: response.outcome,
            remote: response.remote.clone(),
        };
        let bytes = encode_connection_response_frame(&header, &response.payload)
            .map_err(|error| format!("connection.response encode failed: {error}"))?;
        write_session_frame(&self.session, &self.runtime, bytes)
    }
}

/// C-ws §4.2(4): a Runtime protocol violation terminates the exact session
/// (the session owner decides; composition wires the cancellation watcher).
#[derive(Debug, Clone)]
pub struct SessionRuntimeViolationSink {
    session: SessionHandle,
}

impl SessionRuntimeViolationSink {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl RuntimeViolationSink for SessionRuntimeViolationSink {
    fn on_violation(&self, source: &crate::ws::BrokerRuntimeSource, _reason: &str) {
        if let Some(layer) = self.session.layer() {
            layer.request_close(&source.sender, SessionCloseReason::Disconnect);
        }
    }
}

/// WS inbound method dispatch is the E-ws real boundary; until then inbound
/// peer requests fail closed (`serverBusy` per the broker queue-full path).
#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedDispatchInbound;

impl DispatchInbound for UnsupportedDispatchInbound {
    fn dispatch(&self, _action: InboundDispatchAction) -> Result<(), String> {
        Err("WS inbound method dispatch is not wired until E-ws".to_string())
    }
}

/// C-session §5.1 consumer wrapper: `RequestDispatcher` is session-keyed
/// (its pending/permit cleanup runs on `on_session_closed`).
#[derive(Debug, Clone)]
pub struct DispatcherSessionConsumer {
    dispatcher: Arc<RequestDispatcher>,
    router: PendingHttpHandle,
}

impl DispatcherSessionConsumer {
    pub fn new(dispatcher: Arc<RequestDispatcher>, router: PendingHttpHandle) -> Self {
        Self { dispatcher, router }
    }
}

impl SessionConsumer for DispatcherSessionConsumer {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::RequestDispatcher
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        let terminals = self.dispatcher.on_session_closed(session);
        if let Some(router) = self.router.router() {
            for terminal in terminals {
                let request_id = terminal.request_id.clone();
                // The HTTP phase may already be gone (client abandoned the
                // correlation); the dispatcher already released the permit,
                // so a failed delivery is a no-op, never a panic or retry.
                let _ = router.deliver(&request_id, HttpDispatchEvent::Terminal { terminal });
            }
        }
        Ok(())
    }
}

/// Snapshot accessor used by tests and health projections.
pub fn capabilities_by_session(
    layer: &SessionLayer,
) -> HashMap<RuntimeSessionEpoch, DispatchCapabilities> {
    layer.dispatch_capabilities_snapshot()
}

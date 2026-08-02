//! `RuntimeGenerationPinLedger`: Runtime generation pin acquire/release
//! pending/cache/session attachment (C-ws §3, authority design §3.2).
//!
//! The ledger is a pure synchronous reducer (`Mutex` is never held across
//! `.await`); async release resolution is exposed through
//! [`PendingReleaseHandle`]. Release timeout/reject/send failure completes
//! the client terminal and closes the exact Runtime session (1008) — a pin is
//! never silently retained (C-client-lifecycle §4).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION;
use skiff_runtime_transport::websocket_generation_lifecycle::{
    assert_websocket_generation_lifecycle_response_matches, WebSocketGenerationLifecycleControl,
    WebSocketGenerationLifecycleDirection, WebSocketGenerationLifecycleOperation,
    WebSocketGenerationLifecycleRejectionCode, WebSocketGenerationLifecycleSender,
    WebSocketGenerationLifecycleTuple,
};

use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;

use super::types::{Clock, SystemClock, CLOSE_RELEASE_TIMEOUT};

pub const RELEASE_REQUEST_PREFIX: &str = "skiff-websocket-lifecycle-request-v1:opaque";

#[derive(Debug, Clone)]
pub struct LedgerOptions {
    pub release_timeout_ms: u64,
}

impl Default for LedgerOptions {
    fn default() -> Self {
        Self {
            release_timeout_ms: super::types::RELEASE_TIMEOUT_MS_DEFAULT,
        }
    }
}

/// Port that writes one `websocket.generation.lifecycle` control to the exact
/// Runtime session.
pub trait RuntimeGenerationPeer: Send + Sync + fmt::Debug {
    fn send_control(
        &self,
        runtime: &RuntimeSessionEpoch,
        control: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), String>;
}

/// Port that closes the exact Runtime session (1008 protocol-unavailable) on
/// release timeout/reject/send failure.
pub trait RuntimeSessionClose: Send + Sync + fmt::Debug {
    fn close_session(&self, runtime: &RuntimeSessionEpoch, code: u16, reason: &str);
}

/// Port that answers whether the Runtime sender owns the pending WebSocket
/// connect admission (C-ws §3.2 sender-mismatch). The production
/// implementation is wired to the admission pool/dispatcher; absent wiring the
/// seam defaults to accepting.
pub trait PendingAdmissionSender: Send + Sync + fmt::Debug {
    fn is_pending_acquire_sender(
        &self,
        runtime: &RuntimeSessionEpoch,
        tuple: &WebSocketGenerationLifecycleTuple,
    ) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct AllowAnyPendingAdmission;

impl PendingAdmissionSender for AllowAnyPendingAdmission {
    fn is_pending_acquire_sender(
        &self,
        _runtime: &RuntimeSessionEpoch,
        _tuple: &WebSocketGenerationLifecycleTuple,
    ) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcquireDecision {
    Ack(WebSocketGenerationLifecycleControl),
    Reject(WebSocketGenerationLifecycleControl),
}

/// How a release resolved for the finalizer barrier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseResolution {
    Released,
    Failed { reason: String },
}

#[derive(Debug, Clone)]
pub enum ReleaseOutcome {
    /// No pin or the socket is not open: no release frame was written.
    Resolved,
    /// A release frame is (or was) pending; wait for its resolution.
    Pending(PendingReleaseHandle),
}

/// Async wait handle for one pending release. Deduplicated release calls share
/// the same resolution; a late-created receiver observes the already-resolved
/// state.
#[derive(Debug, Clone)]
pub struct PendingReleaseHandle {
    pub request_id: String,
    pub runtime: RuntimeSessionEpoch,
    resolution: tokio::sync::watch::Receiver<Option<ReleaseResolution>>,
}

impl PendingReleaseHandle {
    pub async fn wait(&self) -> ReleaseResolution {
        let mut resolution = self.resolution.clone();
        loop {
            if let Some(value) = resolution.borrow_and_update().clone() {
                return value;
            }
            if resolution.changed().await.is_err() {
                return ReleaseResolution::Failed {
                    reason: "release channel closed".to_string(),
                };
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LedgerHealthSnapshot {
    pub pins_acquired: usize,
    pub pins_pending_release: usize,
    pub cached_acquire_count: usize,
    pub release_acks: u64,
    pub release_failures: Vec<String>,
    pub runtime_closed: Vec<RuntimeSessionEpoch>,
    pub fail_stop_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct AcquiredPin {
    tuple: WebSocketGenerationLifecycleTuple,
    runtime: RuntimeSessionEpoch,
}

#[derive(Debug, Clone)]
struct PendingRelease {
    request: WebSocketGenerationLifecycleControl,
    request_id: String,
    runtime: RuntimeSessionEpoch,
    resolution: tokio::sync::watch::Sender<Option<ReleaseResolution>>,
}

#[derive(Debug, Clone)]
struct CachedAcquire {
    connection_id: String,
    tuple: WebSocketGenerationLifecycleTuple,
    runtime: RuntimeSessionEpoch,
}

#[derive(Debug, Default)]
struct LedgerInner {
    expected_by_connection_id: HashMap<String, WebSocketGenerationLifecycleTuple>,
    acquired_by_connection_id: HashMap<String, AcquiredPin>,
    pending_release_by_connection_id: HashMap<String, PendingRelease>,
    pending_release_by_request_id: HashMap<String, String>,
    cached_acquire_by_request_id: HashMap<String, CachedAcquire>,
    router_session_by_runtime: HashMap<RuntimeSessionEpoch, String>,
    runtime_by_router_session: HashMap<String, RuntimeSessionEpoch>,
    release_ack_count_by_runtime: HashMap<RuntimeSessionEpoch, u64>,
    release_failures: Vec<String>,
    runtime_closed: Vec<RuntimeSessionEpoch>,
    fail_stop_reason: Option<String>,
    next_release_request_id: u64,
}

/// Unique owner of Runtime generation pin acquire/release state
/// (C-ws §3, authority design §3.2).
#[derive(Debug)]
pub struct RuntimeGenerationPinLedger {
    inner: Mutex<LedgerInner>,
    peer: Arc<dyn RuntimeGenerationPeer>,
    close: Arc<dyn RuntimeSessionClose>,
    admission: Arc<dyn PendingAdmissionSender>,
    release_timeout_ms: u64,
}

impl RuntimeGenerationPinLedger {
    pub fn new(
        peer: Arc<dyn RuntimeGenerationPeer>,
        close: Arc<dyn RuntimeSessionClose>,
        admission: Arc<dyn PendingAdmissionSender>,
        options: LedgerOptions,
    ) -> Self {
        Self::with_clock(peer, close, admission, options, Arc::new(SystemClock))
    }

    pub fn with_clock(
        peer: Arc<dyn RuntimeGenerationPeer>,
        close: Arc<dyn RuntimeSessionClose>,
        admission: Arc<dyn PendingAdmissionSender>,
        options: LedgerOptions,
        _clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner: Mutex::new(LedgerInner::default()),
            peer,
            close,
            admission,
            release_timeout_ms: options.release_timeout_ms,
        }
    }

    /// Registers the exact admission expectation before dispatch
    /// (C-ws §3.2). Duplicate connection_id is a process fail-stop.
    pub fn expect_connection(
        &self,
        tuple: WebSocketGenerationLifecycleTuple,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        if inner
            .expected_by_connection_id
            .contains_key(&tuple.connection_id)
        {
            let reason = format!(
                "duplicate WebSocket generation pin expectation for {}",
                tuple.connection_id
            );
            inner.fail_stop_reason = Some(reason.clone());
            return Err(reason);
        }
        inner
            .expected_by_connection_id
            .insert(tuple.connection_id.clone(), tuple);
        Ok(())
    }

    /// Runtime `Acquire` (C-ws §3.2). Returns the exact-echo Ack/Reject
    /// control; the caller (demux) sends it to the runtime.
    pub fn handle_acquire(
        &self,
        runtime: &RuntimeSessionEpoch,
        request: &WebSocketGenerationLifecycleControl,
    ) -> AcquireDecision {
        let WebSocketGenerationLifecycleControl::Acquire {
            schema_version,
            frame_type,
            request_id,
            tuple,
            ..
        } = request
        else {
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
                "acquire expected",
            );
        };
        let mut inner = self.lock();
        if let Some(cached) = inner.cached_acquire_by_request_id.get(request_id) {
            if cached.runtime == *runtime
                && cached.connection_id == tuple.connection_id
                && cached.tuple == *tuple
            {
                return self.acquire_ack(schema_version, frame_type, request_id, tuple);
            }
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::RequestConflict,
                "acquire request id was reused",
            );
        }

        let decision = self.acquire_response(&mut inner, runtime, request);
        if matches!(decision, AcquireDecision::Ack(_)) {
            inner.cached_acquire_by_request_id.insert(
                request_id.clone(),
                CachedAcquire {
                    connection_id: tuple.connection_id.clone(),
                    tuple: tuple.clone(),
                    runtime: runtime.clone(),
                },
            );
            inner.acquired_by_connection_id.insert(
                tuple.connection_id.clone(),
                AcquiredPin {
                    tuple: tuple.clone(),
                    runtime: runtime.clone(),
                },
            );
            inner
                .router_session_by_runtime
                .insert(runtime.clone(), tuple.router_session_id.clone());
            inner
                .runtime_by_router_session
                .insert(tuple.router_session_id.clone(), runtime.clone());
        }
        decision
    }

    fn acquire_response(
        &self,
        inner: &mut LedgerInner,
        runtime: &RuntimeSessionEpoch,
        request: &WebSocketGenerationLifecycleControl,
    ) -> AcquireDecision {
        let WebSocketGenerationLifecycleControl::Acquire {
            request_id, tuple, ..
        } = request
        else {
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
                "acquire expected",
            );
        };
        if let Some(bound_runtime) = inner
            .runtime_by_router_session
            .get(&tuple.router_session_id)
        {
            if bound_runtime != runtime {
                return self.acquire_reject(
                    request,
                    WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
                    "router session does not belong to the runtime sender",
                );
            }
        }
        if let Some(bound_session) = inner.router_session_by_runtime.get(runtime) {
            if bound_session != &tuple.router_session_id {
                return self.acquire_reject(
                    request,
                    WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
                    "runtime sender is already bound to another router session",
                );
            }
        }
        let Some(expected) = inner.expected_by_connection_id.get(&tuple.connection_id) else {
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::NotAcquired,
                "connection is not pending admission",
            );
        };
        if expected != tuple {
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
                "acquire tuple does not match the admission expectation",
            );
        }
        if !self.admission.is_pending_acquire_sender(runtime, tuple) {
            return self.acquire_reject(
                request,
                WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
                "acquire sender does not own the pending WebSocket connect request",
            );
        }
        if let Some(existing) = inner.acquired_by_connection_id.get(&tuple.connection_id) {
            if existing.runtime != *runtime || existing.tuple != *tuple {
                return self.acquire_reject(
                    request,
                    WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
                    "connection already has a different generation pin",
                );
            }
        }
        self.acquire_ack(
            request_schema(request),
            request_frame_type(request),
            request_id,
            tuple,
        )
    }

    fn acquire_ack(
        &self,
        schema_version: &str,
        frame_type: &str,
        request_id: &str,
        tuple: &WebSocketGenerationLifecycleTuple,
    ) -> AcquireDecision {
        AcquireDecision::Ack(WebSocketGenerationLifecycleControl::Ack {
            schema_version: schema_version.to_string(),
            frame_type: frame_type.to_string(),
            operation: WebSocketGenerationLifecycleOperation::Acquire,
            request_id: request_id.to_string(),
            sender: WebSocketGenerationLifecycleSender::Router,
            tuple: tuple.clone(),
        })
    }

    fn acquire_reject(
        &self,
        request: &WebSocketGenerationLifecycleControl,
        code: WebSocketGenerationLifecycleRejectionCode,
        reason: &str,
    ) -> AcquireDecision {
        AcquireDecision::Reject(WebSocketGenerationLifecycleControl::Reject {
            schema_version: request_schema(request).to_string(),
            frame_type: request_frame_type(request).to_string(),
            operation: WebSocketGenerationLifecycleOperation::Acquire,
            request_id: request_request_id(request).to_string(),
            sender: WebSocketGenerationLifecycleSender::Router,
            tuple: request_tuple(request).clone(),
            code,
            reason: reason.to_string(),
        })
    }

    /// Finalizer step: release the exact connection pin (C-ws §3.3).
    /// Existing pending releases are deduplicated to the same promise; a pin
    /// that is not acquired or a closed socket resolves immediately without
    /// writing a frame.
    pub fn release_connection(
        &self,
        connection_id: &str,
        socket_open: bool,
    ) -> Result<ReleaseOutcome, String> {
        let mut inner = self.lock();
        if let Some(pending) = inner
            .pending_release_by_connection_id
            .get(connection_id)
            .cloned()
        {
            return Ok(ReleaseOutcome::Pending(PendingReleaseHandle {
                request_id: pending.request_id.clone(),
                runtime: pending.runtime.clone(),
                resolution: pending.resolution.subscribe(),
            }));
        }
        inner.expected_by_connection_id.remove(connection_id);
        inner
            .cached_acquire_by_request_id
            .retain(|_, cached| cached.connection_id != connection_id);
        let Some(acquired) = inner.acquired_by_connection_id.remove(connection_id) else {
            return Ok(ReleaseOutcome::Resolved);
        };
        if !socket_open {
            return Ok(ReleaseOutcome::Resolved);
        }

        let request_id = format!(
            "{RELEASE_REQUEST_PREFIX}:release-{}",
            inner.next_release_request_id
        );
        inner.next_release_request_id += 1;
        let (resolution_tx, resolution_rx) = tokio::sync::watch::channel(None);
        let request = WebSocketGenerationLifecycleControl::Release {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "websocket.generation.lifecycle".to_string(),
            request_id: request_id.clone(),
            sender: WebSocketGenerationLifecycleSender::Router,
            tuple: acquired.tuple.clone(),
        };
        inner.pending_release_by_connection_id.insert(
            connection_id.to_string(),
            PendingRelease {
                request: request.clone(),
                request_id: request_id.clone(),
                runtime: acquired.runtime.clone(),
                resolution: resolution_tx,
            },
        );
        inner
            .pending_release_by_request_id
            .insert(request_id.clone(), connection_id.to_string());
        let outcome = ReleaseOutcome::Pending(PendingReleaseHandle {
            request_id: request_id.clone(),
            runtime: acquired.runtime.clone(),
            resolution: resolution_rx,
        });
        if let Err(error) = self.peer.send_control(&acquired.runtime, &request) {
            close_session_locked(
                &mut inner,
                &self.close,
                &acquired.runtime,
                1008,
                "websocket generation release send failed",
            );
            self.finish_release(
                &mut inner,
                &request_id,
                Some(format!("release send failed: {error}")),
            );
        }
        Ok(outcome)
    }

    /// Runtime Ack/Reject for a pending release (C-ws §3.3). Unknown responses
    /// are protocol violations returned as `Err`.
    pub fn handle_release_response(
        &self,
        runtime: &RuntimeSessionEpoch,
        response: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), String> {
        let mut inner = self.lock();
        let Some(connection_id) = inner
            .pending_release_by_request_id
            .get(request_request_id(response))
            .cloned()
        else {
            return Err("unexpected websocket generation release response".to_string());
        };
        let pending = inner
            .pending_release_by_connection_id
            .get(&connection_id)
            .cloned()
            .ok_or_else(|| "pending release state missing".to_string())?;
        if &pending.runtime != runtime {
            return Err("websocket generation release response sender mismatch".to_string());
        }
        assert_websocket_generation_lifecycle_response_matches(&pending.request, response)
            .map_err(|error| error.to_string())?;
        match response {
            WebSocketGenerationLifecycleControl::Ack { .. } => {
                let count = inner
                    .release_ack_count_by_runtime
                    .entry(runtime.clone())
                    .or_default();
                *count += 1;
                self.finish_release(&mut inner, &pending.request_id, None);
                Ok(())
            }
            WebSocketGenerationLifecycleControl::Reject { code, reason, .. } => {
                let message = format!("runtime rejected release: {code:?}: {reason}");
                close_session_locked(
                    &mut inner,
                    &self.close,
                    runtime,
                    1008,
                    "websocket generation release rejected",
                );
                self.finish_release(&mut inner, &pending.request_id, Some(message));
                Ok(())
            }
            _ => Err("websocket generation release response must be ack or reject".to_string()),
        }
    }

    /// Release timeout (C-ws §3.3): resolves the pending as failed, closes the
    /// exact Runtime session with 1008 and never retains the pin.
    pub fn fire_release_timeout(&self, request_id: &str) -> Option<ReleaseResolution> {
        let mut inner = self.lock();
        let connection = inner.pending_release_by_request_id.get(request_id)?.clone();
        let pending = inner
            .pending_release_by_connection_id
            .get(&connection)
            .cloned()?;
        let message = format!("release timed out for {connection}");
        close_session_locked(
            &mut inner,
            &self.close,
            &pending.runtime,
            CLOSE_RELEASE_TIMEOUT.0,
            CLOSE_RELEASE_TIMEOUT.1,
        );
        Some(self.finish_release(&mut inner, request_id, Some(message)))
    }

    /// Runtime disconnect: clears ack counts, acquired pins, pending releases,
    /// cached acquires and session bindings for the exact runtime; pending
    /// releases resolve without an ACK (C-ws §3.3).
    pub fn runtime_disconnected(&self, runtime: &RuntimeSessionEpoch) {
        let mut inner = self.lock();
        inner.release_ack_count_by_runtime.remove(runtime);
        let affected = inner
            .acquired_by_connection_id
            .values()
            .filter(|acquired| &acquired.runtime == runtime)
            .map(|acquired| acquired.tuple.connection_id.clone())
            .collect::<Vec<_>>();
        for connection in &affected {
            inner.expected_by_connection_id.remove(connection);
            inner.acquired_by_connection_id.remove(connection);
        }
        let pending = inner
            .pending_release_by_connection_id
            .values()
            .filter(|pending| &pending.runtime == runtime)
            .map(|pending| pending.connection_key())
            .collect::<Vec<_>>();
        for connection in pending {
            if let Some(request_id) = inner
                .pending_release_by_connection_id
                .get(&connection)
                .map(|pending| pending.request_id.clone())
            {
                self.finish_release(&mut inner, &request_id, None);
            }
        }
        inner
            .cached_acquire_by_request_id
            .retain(|_, cached| &cached.runtime != runtime);
        if let Some(session) = inner.router_session_by_runtime.remove(runtime) {
            if inner.runtime_by_router_session.get(&session) == Some(runtime) {
                inner.runtime_by_router_session.remove(&session);
            }
        }
    }

    /// Flush waits for every pending release and aggregates failures
    /// (C-ws §3.3; gateway shutdown). Failures are preserved in health.
    pub async fn flush(&self) -> Result<(), Vec<String>> {
        let handles = {
            let inner = self.lock();
            inner
                .pending_release_by_connection_id
                .values()
                .map(|pending| PendingReleaseHandle {
                    request_id: pending.request_id.clone(),
                    runtime: pending.runtime.clone(),
                    resolution: pending.resolution.subscribe(),
                })
                .collect::<Vec<_>>()
        };
        let deadline = Duration::from_millis(self.release_timeout_ms + 1000);
        for handle in handles {
            let _ = tokio::time::timeout(deadline, handle.wait()).await;
        }
        let failures = {
            let mut inner = self.lock();
            if !inner.pending_release_by_request_id.is_empty() {
                inner
                    .release_failures
                    .push("flush with unresolved pending release".to_string());
            }
            inner.release_failures.clone()
        };
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures)
        }
    }

    pub fn snapshot(&self) -> LedgerHealthSnapshot {
        let inner = self.lock();
        LedgerHealthSnapshot {
            pins_acquired: inner.acquired_by_connection_id.len(),
            pins_pending_release: inner.pending_release_by_connection_id.len(),
            cached_acquire_count: inner.cached_acquire_by_request_id.len(),
            release_acks: inner.release_ack_count_by_runtime.values().sum(),
            release_failures: inner.release_failures.clone(),
            runtime_closed: inner.runtime_closed.clone(),
            fail_stop_reason: inner.fail_stop_reason.clone(),
        }
    }

    pub fn fail_stop_reason(&self) -> Option<String> {
        self.lock().fail_stop_reason.clone()
    }

    pub fn pending_release_request_id(&self, connection_id: &str) -> Option<String> {
        self.lock()
            .pending_release_by_connection_id
            .get(connection_id)
            .map(|pending| pending.request_id.clone())
    }

    pub fn release_ack_count(&self, runtime: &RuntimeSessionEpoch) -> u64 {
        self.lock()
            .release_ack_count_by_runtime
            .get(runtime)
            .copied()
            .unwrap_or(0)
    }

    fn finish_release(
        &self,
        inner: &mut LedgerInner,
        request_id: &str,
        failure: Option<String>,
    ) -> ReleaseResolution {
        let Some(connection_id) = inner.pending_release_by_request_id.remove(request_id) else {
            return ReleaseResolution::Released;
        };
        let Some(pending) = inner
            .pending_release_by_connection_id
            .remove(&connection_id)
        else {
            return ReleaseResolution::Released;
        };
        let resolution = match failure {
            Some(reason) => {
                inner.release_failures.push(reason.clone());
                // Release failure makes the exact Runtime session
                // protocol-unavailable: unbind the router session so a later
                // generation can bind it again (C-ws §3.3).
                if let Some(session) = inner.router_session_by_runtime.remove(&pending.runtime) {
                    if inner.runtime_by_router_session.get(&session) == Some(&pending.runtime) {
                        inner.runtime_by_router_session.remove(&session);
                    }
                }
                ReleaseResolution::Failed { reason }
            }
            None => ReleaseResolution::Released,
        };
        let _ = pending.resolution.send(Some(resolution.clone()));
        resolution
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, LedgerInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn close_session_locked(
    inner: &mut LedgerInner,
    close: &Arc<dyn RuntimeSessionClose>,
    runtime: &RuntimeSessionEpoch,
    code: u16,
    reason: &str,
) {
    close.close_session(runtime, code, reason);
    if !inner.runtime_closed.contains(runtime) {
        inner.runtime_closed.push(runtime.clone());
    }
}

impl PendingRelease {
    fn connection_key(&self) -> String {
        request_tuple(&self.request).connection_id.clone()
    }
}

fn request_schema(request: &WebSocketGenerationLifecycleControl) -> &str {
    match request {
        WebSocketGenerationLifecycleControl::Acquire { schema_version, .. }
        | WebSocketGenerationLifecycleControl::Release { schema_version, .. }
        | WebSocketGenerationLifecycleControl::Ack { schema_version, .. }
        | WebSocketGenerationLifecycleControl::Reject { schema_version, .. } => schema_version,
    }
}

fn request_frame_type(request: &WebSocketGenerationLifecycleControl) -> &str {
    match request {
        WebSocketGenerationLifecycleControl::Acquire { frame_type, .. }
        | WebSocketGenerationLifecycleControl::Release { frame_type, .. }
        | WebSocketGenerationLifecycleControl::Ack { frame_type, .. }
        | WebSocketGenerationLifecycleControl::Reject { frame_type, .. } => frame_type,
    }
}

fn request_request_id(request: &WebSocketGenerationLifecycleControl) -> &str {
    match request {
        WebSocketGenerationLifecycleControl::Acquire { request_id, .. }
        | WebSocketGenerationLifecycleControl::Release { request_id, .. }
        | WebSocketGenerationLifecycleControl::Ack { request_id, .. }
        | WebSocketGenerationLifecycleControl::Reject { request_id, .. } => request_id,
    }
}

fn request_tuple(
    request: &WebSocketGenerationLifecycleControl,
) -> &WebSocketGenerationLifecycleTuple {
    match request {
        WebSocketGenerationLifecycleControl::Acquire { tuple, .. }
        | WebSocketGenerationLifecycleControl::Release { tuple, .. }
        | WebSocketGenerationLifecycleControl::Ack { tuple, .. }
        | WebSocketGenerationLifecycleControl::Reject { tuple, .. } => tuple,
    }
}

impl SessionConsumer for RuntimeGenerationPinLedger {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::RuntimeGenerationPinLedger
    }

    fn on_session_closed(&self, session: &RuntimeSessionEpoch) -> Result<(), String> {
        self.runtime_disconnected(session);
        Ok(())
    }
}

// Keep the direction import referenced for documentation symmetry; the
// response echo validation is delegated to the frozen transport assertion.
#[allow(dead_code)]
fn _direction_anchor() -> WebSocketGenerationLifecycleDirection {
    WebSocketGenerationLifecycleDirection::RouterToRuntime
}

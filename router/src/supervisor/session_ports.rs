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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use skiff_runtime_transport::assembly_activation::{
    encode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::connection_protocol::{
    encode_connection_response_frame, ConnectionResponseFrameHeader,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, encode_request_cancel_frame, RequestCancelFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
    RuntimeAssemblyTaskInvocationFrameHeader, RuntimeAssemblyTaskRequestCallerFrameHeader,
    RuntimeAssemblyTaskRequestRoutingFrameHeader, RuntimeAssemblyTaskRequestStartFrameHeader,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    encode_websocket_generation_lifecycle_frame, WebSocketGenerationLifecycleControl,
    WebSocketGenerationLifecycleDirection,
};

use crate::activation::{ActivationParticipantBinding, EnqueueResult, SessionEnqueuePort};
use crate::actor::TaskWireStore;
use crate::bootstrap::{ActiveRoutingEpochStore, RoutingEpoch};
use crate::dispatch::{
    CandidateViewSource, LeaseRevalidate, RevalidateOutcome, RoutingEpochSource,
};
use crate::dispatch::{
    DispatchSubmit, RequestDispatcher, RuntimePeer, SessionAbortControl, TaskSubmit,
};
use crate::routing::{
    CandidateDirectoryView, DispatchCapabilities, RegisteredSessionLease, RuntimeCandidateQuery,
};
use crate::session::consumer::{ConsumerKind, SessionConsumer};
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::layer::{SessionCloseReason, SessionLayer};
use crate::ws::{
    BrokerRuntimeResponse, DispatchInbound, InboundDispatchAction, RuntimeGenerationPeer,
    RuntimeResponder, RuntimeSessionClose, RuntimeViolationSink,
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
/// the per-session dispatch capability binding retained by the session layer.
#[derive(Debug, Clone)]
pub struct SessionCandidateViewSource {
    session: SessionHandle,
}

impl SessionCandidateViewSource {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl CandidateViewSource for SessionCandidateViewSource {
    fn view(&self) -> CandidateDirectoryView {
        let Some(layer) = self.session.layer() else {
            return CandidateDirectoryView {
                revision: None,
                sessions: Vec::new(),
            };
        };
        let capabilities = layer.dispatch_capabilities_snapshot();
        let directory = layer.directory_lock();
        RuntimeCandidateQuery::snapshot_directory_view(&directory, &capabilities)
    }
}

/// Plan §3.3 step 1: captures the current whole epoch from the single
/// authority store. `None` means no committed epoch: admission fails closed.
#[derive(Debug, Clone)]
pub struct StoreRoutingEpochSource {
    store: Arc<ActiveRoutingEpochStore>,
}

impl StoreRoutingEpochSource {
    pub fn new(store: Arc<ActiveRoutingEpochStore>) -> Self {
        Self { store }
    }
}

impl RoutingEpochSource for StoreRoutingEpochSource {
    fn capture(&self) -> Option<Arc<RoutingEpoch>> {
        self.store.capture()
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
            return RevalidateOutcome::TupleMismatch;
        };
        let directory = layer.directory_lock();
        let Some(record) = directory.record(&lease.session_epoch) else {
            return RevalidateOutcome::TupleMismatch;
        };
        if record.cancelled {
            return RevalidateOutcome::Cancelled;
        }
        if record.registration_revision != lease.registration_revision {
            return RevalidateOutcome::StaleRevision;
        }
        if record.registered_tuple.as_ref() != Some(&lease.exact_registered_tuple) {
            return RevalidateOutcome::TupleMismatch;
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
    task_wire_store: Option<Arc<TaskWireStore>>,
}

impl SessionRuntimePeer {
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            task_wire_store: None,
        }
    }

    /// E-actor-rust: the derived function-task trace (and opaque task wire
    /// facts) are correlated through the actor lane wire store.
    pub fn with_task_wire_store(mut self, store: Arc<TaskWireStore>) -> Self {
        self.task_wire_store = Some(store);
        self
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

    fn send_task_submit(
        &self,
        session: &RuntimeSessionEpoch,
        task: &TaskSubmit,
    ) -> Result<(), String> {
        // Derived function task execution frame (TS `derivedTaskRequest`
        // parity): the dispatcher owns the derived pending; this port maps
        // it onto the canonical `runtimeAssembly task request.start` wire.
        // The recoverable args payload is the original task.submit payload
        // (TS `dispatchDerivedTask(ws, request, payloadBytes)`); the strict
        // Runtime decoder requires it to be present.
        let wire = self
            .task_wire_store
            .as_ref()
            .and_then(|store| store.get(&task.task_request_id))
            .ok_or_else(|| {
                format!(
                    "derived task {} has no captured task wire",
                    task.task_request_id
                )
            })?;
        let wire_trace_id = wire.frame.header.trace_id.clone();
        let span_id = format!(
            "{:016x}",
            now_nanos().wrapping_add(TASK_SPAN_SEQUENCE.fetch_add(1, Ordering::Relaxed))
        );
        let header = RuntimeAssemblyTaskRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: task.task_request_id.clone(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyTaskRequestCallerFrameHeader {
                kind: "service".to_string(),
            },
            routing: RuntimeAssemblyTaskRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                    task.authority.assembly_identity.clone(),
                ),
                assembly_generation: task.authority.assembly_generation,
                deployment: task.authority.deployment.clone(),
            },
            invocation: RuntimeAssemblyTaskInvocationFrameHeader {
                kind: "task".to_string(),
                target_kind: "function".to_string(),
                target: task.target.clone(),
            },
            deadline: task.deadline.as_ref().map(|deadline| {
                RuntimeAssemblyRequestDeadlineFrameHeader {
                    timeout_ms: deadline.timeout_ms,
                    expires_at: deadline.expires_at.clone(),
                }
            }),
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: wire_trace_id.unwrap_or_else(|| format!("task-trace-{span_id}")),
                span_id,
                parent_span_id: None,
                sampled: None,
            },
            test_effects_enabled: false,
            test_case_capability: None,
            task_attempt: None,
        };
        let bytes = encode_binary_frame(&header, &wire.frame.payload)
            .map_err(|error| format!("derived task request.start encode failed: {error}"))?;
        write_session_frame(&self.session, session, bytes)
    }
}

static TASK_SPAN_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
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

/// C-activation §8 `SessionEnqueuePort`: non-blocking prepare/commit/abort
/// writer per exact session; queue-full / missing writer maps to `QueueFull`
/// (the coordinator durably aborts and aborts the exact session).
#[derive(Debug, Clone)]
pub struct ActivationSessionEnqueuePort {
    session: SessionHandle,
}

impl ActivationSessionEnqueuePort {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl SessionEnqueuePort for ActivationSessionEnqueuePort {
    fn enqueue_prepare(
        &self,
        binding: &ActivationParticipantBinding,
        control: &skiff_artifact_model::AssemblyActivationControl,
    ) -> EnqueueResult {
        enqueue_activation(&self.session, binding, control)
    }

    fn enqueue_commit(
        &self,
        binding: &ActivationParticipantBinding,
        control: &skiff_artifact_model::AssemblyActivationControl,
    ) -> EnqueueResult {
        enqueue_activation(&self.session, binding, control)
    }

    fn enqueue_abort(
        &self,
        binding: &ActivationParticipantBinding,
        control: &skiff_artifact_model::AssemblyActivationControl,
    ) -> EnqueueResult {
        enqueue_activation(&self.session, binding, control)
    }

    fn abort_session(&self, session: &RuntimeSessionEpoch) {
        if let Some(layer) = self.session.layer() {
            layer.request_close(session, SessionCloseReason::Disconnect);
        }
    }
}

fn enqueue_activation(
    handle: &SessionHandle,
    binding: &ActivationParticipantBinding,
    control: &skiff_artifact_model::AssemblyActivationControl,
) -> EnqueueResult {
    let bytes = match encode_assembly_activation_frame(
        AssemblyActivationFrameDirection::RouterToRuntime,
        control,
    ) {
        Ok(bytes) => bytes,
        Err(_) => return EnqueueResult::QueueFull,
    };
    match write_session_frame(handle, &binding.session_epoch, bytes) {
        Ok(()) => EnqueueResult::Ok,
        Err(_) => EnqueueResult::QueueFull,
    }
}

/// C-ws §3.3 outbound lifecycle control writer over the session registry.
#[derive(Debug, Clone)]
pub struct WsRuntimeGenerationPeer {
    session: SessionHandle,
}

impl WsRuntimeGenerationPeer {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl RuntimeGenerationPeer for WsRuntimeGenerationPeer {
    fn send_control(
        &self,
        runtime: &RuntimeSessionEpoch,
        control: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), String> {
        let bytes = encode_websocket_generation_lifecycle_frame(
            WebSocketGenerationLifecycleDirection::RouterToRuntime,
            control,
        )
        .map_err(|error| format!("websocket lifecycle encode failed: {error}"))?;
        write_session_frame(&self.session, runtime, bytes)
    }
}

/// C-ws §3.3 exact-session close (1008 protocol-unavailable etc.).
#[derive(Debug, Clone)]
pub struct WsRuntimeSessionClose {
    session: SessionHandle,
}

impl WsRuntimeSessionClose {
    pub fn new(session: SessionHandle) -> Self {
        Self { session }
    }
}

impl RuntimeSessionClose for WsRuntimeSessionClose {
    fn close_session(&self, runtime: &RuntimeSessionEpoch, _code: u16, _reason: &str) {
        if let Some(layer) = self.session.layer() {
            layer.request_close(runtime, SessionCloseReason::Disconnect);
        }
    }
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

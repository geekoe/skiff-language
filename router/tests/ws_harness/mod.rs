//! Shared fake seams for W-WebSocket tests (C-ws §5.7, C-client-lifecycle
//! §6.7): fake writer/socket, fake runtime sender/responder/session close,
//! fake dispatcher, fake method catalog and fake violation sink.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::ws::{
    BrokerRuntimeResponse, BrokerRuntimeSource, DispatchInbound, InboundDispatchAction,
    MethodCatalog, PeerWriter, RuntimeGenerationPeer, RuntimeResponder, RuntimeSessionClose,
    RuntimeViolationSink,
};
use skiff_runtime_transport::websocket_generation_lifecycle::{
    WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleSender,
    WebSocketGenerationLifecycleTuple,
};

#[derive(Debug, Default)]
struct FakePeerWriterInner {
    writes: Vec<String>,
    close: Option<(u16, String)>,
    terminated: bool,
    fail_next: bool,
    buffered: u64,
}

/// Captured transport writer double (single-writer per generation).
#[derive(Debug, Clone, Default)]
pub struct FakePeerWriter {
    inner: Arc<Mutex<FakePeerWriterInner>>,
}

impl FakePeerWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn writes(&self) -> Vec<String> {
        self.inner.lock().unwrap().writes.clone()
    }

    pub fn close(&self) -> Option<(u16, String)> {
        self.inner.lock().unwrap().close.clone()
    }

    pub fn terminated(&self) -> bool {
        self.inner.lock().unwrap().terminated
    }

    pub fn fail_next(&self) {
        self.inner.lock().unwrap().fail_next = true;
    }

    pub fn set_buffered(&self, bytes: u64) {
        self.inner.lock().unwrap().buffered = bytes;
    }
}

impl PeerWriter for FakePeerWriter {
    fn write_text(&self, frame: String) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next {
            inner.fail_next = false;
            return Err("injected writer failure".to_string());
        }
        inner.writes.push(frame);
        Ok(())
    }

    fn buffered_bytes(&self) -> u64 {
        self.inner.lock().unwrap().buffered
    }

    fn close(&self, code: u16, reason: &str) -> Result<(), String> {
        self.inner.lock().unwrap().close = Some((code, reason.to_string()));
        Ok(())
    }

    fn terminate(&self) {
        self.inner.lock().unwrap().terminated = true;
    }
}

#[derive(Debug, Default)]
struct FakeRuntimePeerInner {
    controls: Vec<(RuntimeSessionEpoch, WebSocketGenerationLifecycleControl)>,
    fail_next: bool,
}

/// Ledger `RuntimeGenerationPeer` double.
#[derive(Debug, Clone, Default)]
pub struct FakeRuntimePeer {
    inner: Arc<Mutex<FakeRuntimePeerInner>>,
}

impl FakeRuntimePeer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn controls(&self) -> Vec<(RuntimeSessionEpoch, WebSocketGenerationLifecycleControl)> {
        self.inner.lock().unwrap().controls.clone()
    }

    pub fn fail_next_send(&self) {
        self.inner.lock().unwrap().fail_next = true;
    }
}

impl RuntimeGenerationPeer for FakeRuntimePeer {
    fn send_control(
        &self,
        runtime: &RuntimeSessionEpoch,
        control: &WebSocketGenerationLifecycleControl,
    ) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next {
            inner.fail_next = false;
            return Err("injected runtime peer failure".to_string());
        }
        inner.controls.push((runtime.clone(), control.clone()));
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeRuntimeCloseInner {
    closes: Vec<(RuntimeSessionEpoch, u16, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct FakeRuntimeSessionClose {
    inner: Arc<Mutex<FakeRuntimeCloseInner>>,
}

impl FakeRuntimeSessionClose {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn closes(&self) -> Vec<(RuntimeSessionEpoch, u16, String)> {
        self.inner.lock().unwrap().closes.clone()
    }
}

impl RuntimeSessionClose for FakeRuntimeSessionClose {
    fn close_session(&self, runtime: &RuntimeSessionEpoch, code: u16, reason: &str) {
        self.inner
            .lock()
            .unwrap()
            .closes
            .push((runtime.clone(), code, reason.to_string()));
    }
}

#[derive(Debug, Default)]
struct FakeResponderInner {
    responses: Vec<BrokerRuntimeResponse>,
    fail_next: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FakeRuntimeResponder {
    inner: Arc<Mutex<FakeResponderInner>>,
}

impl FakeRuntimeResponder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn responses(&self) -> Vec<BrokerRuntimeResponse> {
        self.inner.lock().unwrap().responses.clone()
    }

    pub fn fail_next(&self) {
        self.inner.lock().unwrap().fail_next = true;
    }
}

impl RuntimeResponder for FakeRuntimeResponder {
    fn respond(&self, response: &BrokerRuntimeResponse) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next {
            inner.fail_next = false;
            return Err("injected responder failure".to_string());
        }
        inner.responses.push(response.clone());
        Ok(())
    }
}

#[derive(Debug, Default)]
struct FakeDispatchInner {
    actions: Vec<InboundDispatchAction>,
    fail_next: bool,
}

#[derive(Debug, Clone, Default)]
pub struct FakeDispatchInbound {
    inner: Arc<Mutex<FakeDispatchInner>>,
}

impl FakeDispatchInbound {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn actions(&self) -> Vec<InboundDispatchAction> {
        self.inner.lock().unwrap().actions.clone()
    }

    pub fn fail_next(&self) {
        self.inner.lock().unwrap().fail_next = true;
    }
}

impl DispatchInbound for FakeDispatchInbound {
    fn dispatch(&self, action: InboundDispatchAction) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next {
            inner.fail_next = false;
            return Err("injected dispatch failure".to_string());
        }
        inner.actions.push(action);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct FakeMethodCatalog {
    accepted: HashSet<String>,
}

impl FakeMethodCatalog {
    pub fn new() -> Self {
        let mut accepted = HashSet::new();
        accepted.insert("chat.send".to_string());
        accepted.insert("status.get".to_string());
        Self { accepted }
    }
}

impl MethodCatalog for FakeMethodCatalog {
    fn accepts(&self, method: &str) -> bool {
        self.accepted.contains(method)
    }
}

#[derive(Debug, Default)]
struct FakeViolationInner {
    violations: Vec<(RuntimeSessionEpoch, String, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct FakeRuntimeViolationSink {
    inner: Arc<Mutex<FakeViolationInner>>,
}

impl FakeRuntimeViolationSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn violations(&self) -> Vec<(RuntimeSessionEpoch, String, String)> {
        self.inner.lock().unwrap().violations.clone()
    }
}

impl RuntimeViolationSink for FakeRuntimeViolationSink {
    fn on_violation(&self, source: &BrokerRuntimeSource, reason: &str) {
        self.inner.lock().unwrap().violations.push((
            source.sender.clone(),
            source.session_token.clone(),
            reason.to_string(),
        ));
    }
}

pub fn runtime_session(display: &str) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: display.to_string(),
        connection_generation: 1,
    }
}

pub fn pin_tuple(connection: &str, runtime_display: &str) -> WebSocketGenerationLifecycleTuple {
    WebSocketGenerationLifecycleTuple {
        router_session_id: format!("session-{runtime_display}"),
        service_id: "example.com/chat".to_string(),
        assembly_identity: skiff_artifact_model::AssemblyIdentity::new(format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "a".repeat(64)
        )),
        assembly_generation: 7,
        websocket_entry_id: format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64)),
        connection_id: connection.to_string(),
    }
}

pub fn acquire_control(
    request_id: &str,
    tuple: &WebSocketGenerationLifecycleTuple,
) -> WebSocketGenerationLifecycleControl {
    WebSocketGenerationLifecycleControl::Acquire {
        schema_version: skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "websocket.generation.lifecycle".to_string(),
        request_id: request_id.to_string(),
        sender: WebSocketGenerationLifecycleSender::Runtime,
        tuple: tuple.clone(),
    }
}

//! Shared fake seams for W-WebSocket tests (C-ws §5.7, C-client-lifecycle
//! §6.7): fake writer/socket, fake runtime responder/session close, fake
//! dispatcher, fake method catalog and fake violation sink.

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::ws::{
    BrokerRuntimeResponse, BrokerRuntimeSource, DispatchInbound, InboundDispatchAction,
    MethodCatalog, PeerWriter, RuntimeResponder, RuntimeViolationSink,
};

#[derive(Debug, Default)]
struct FakePeerWriterInner {
    writes: Vec<String>,
    binary_writes: Vec<Vec<u8>>,
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

    fn write_binary(&self, payload: Vec<u8>) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        if inner.fail_next {
            inner.fail_next = false;
            return Err("injected writer failure".to_string());
        }
        inner.binary_writes.push(payload);
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

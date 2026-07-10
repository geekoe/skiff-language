use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex as StdMutex,
    },
};

use skiff_runtime_model::error::{RuntimeErrorPayload, WirePayload};
use tokio::sync::{mpsc, Notify};

use crate::{HttpResponseMetadata, ResponseError};

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundResponse {
    Start { http_response: HttpResponseMetadata },
    Chunk { seq: u64, payload: Vec<u8> },
    End { payload: Vec<u8> },
    Error(ResponseError),
}

impl OutboundResponse {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Start { .. } => "response.start",
            Self::Chunk { .. } => "response.chunk",
            Self::End { .. } => "response.end",
            Self::Error(_) => "response.error",
        }
    }
}

pub type OutboundResponseReceiver = mpsc::UnboundedReceiver<OutboundResponse>;
pub type OutboundResponseSender = mpsc::UnboundedSender<OutboundResponse>;
pub type OutboundRequestCancelSender =
    Arc<dyn Fn(&str, &str) -> Result<(), OutboundRequestCancelSendError> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundRequestCancelSendError {
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutboundRequestRegistryError {
    LockPoisoned,
    DuplicateRequestId(String),
}

impl fmt::Display for OutboundRequestRegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LockPoisoned => formatter.write_str("outbound request registry lock is poisoned"),
            Self::DuplicateRequestId(request_id) => {
                write!(formatter, "duplicate outbound request id {request_id}")
            }
        }
    }
}

impl Error for OutboundRequestRegistryError {}

impl WirePayload for OutboundRequestRegistryError {
    fn payload(&self) -> RuntimeErrorPayload {
        RuntimeErrorPayload {
            code: "InternalError".to_string(),
            message: self.to_string(),
            status: None,
            details: None,
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[derive(Clone, Default)]
pub struct OutboundRequestRegistry {
    inner: Arc<OutboundRequestRegistryInner>,
}

#[derive(Default)]
struct OutboundRequestRegistryInner {
    pending: StdMutex<HashMap<String, OutboundRequestEntry>>,
    leases_active: AtomicUsize,
    cancel_send_failed_closed: AtomicUsize,
}

#[derive(Clone)]
struct OutboundRequestEntry {
    sender: OutboundResponseSender,
    terminal: OutboundRequestTerminalSignal,
}

pub struct OutboundRequestLease {
    request_id: String,
    registry: OutboundRequestRegistry,
    terminal: OutboundRequestTerminalSignal,
    cancel_sender: Option<OutboundRequestCancelSender>,
    drop_cancel_reason: &'static str,
}

#[derive(Clone, Debug)]
pub struct OutboundRequestTerminalSignal {
    inner: Arc<OutboundRequestTerminalState>,
}

#[derive(Debug)]
struct OutboundRequestTerminalState {
    terminal: AtomicBool,
    notify: Notify,
}

impl OutboundRequestRegistry {
    pub fn insert_with_lease(
        &self,
        request_id: String,
        sender: OutboundResponseSender,
        cancel_sender: Option<OutboundRequestCancelSender>,
        drop_cancel_reason: &'static str,
    ) -> Result<OutboundRequestLease, OutboundRequestRegistryError> {
        let terminal = OutboundRequestTerminalSignal::new();
        let mut pending = self
            .inner
            .pending
            .lock()
            .map_err(|_| OutboundRequestRegistryError::LockPoisoned)?;
        if pending.contains_key(&request_id) {
            return Err(OutboundRequestRegistryError::DuplicateRequestId(request_id));
        }
        pending.insert(
            request_id.clone(),
            OutboundRequestEntry {
                sender,
                terminal: terminal.clone(),
            },
        );
        self.inner.leases_active.fetch_add(1, Ordering::AcqRel);
        Ok(OutboundRequestLease {
            request_id,
            registry: self.clone(),
            terminal,
            cancel_sender,
            drop_cancel_reason,
        })
    }

    pub fn complete(&self, request_id: &str) -> Option<OutboundResponseSender> {
        let entry = self.remove_entry(request_id)?;
        entry.terminal.mark_terminal();
        Some(entry.sender)
    }

    pub fn sender(&self, request_id: &str) -> Option<OutboundResponseSender> {
        self.inner
            .pending
            .lock()
            .ok()?
            .get(request_id)
            .map(|entry| entry.sender.clone())
    }

    pub fn contains(&self, request_id: &str) -> bool {
        self.inner
            .pending
            .lock()
            .is_ok_and(|pending| pending.contains_key(request_id))
    }

    pub fn contains_matching(&self, mut matches: impl FnMut(&str) -> bool) -> bool {
        self.inner
            .pending
            .lock()
            .is_ok_and(|pending| pending.keys().any(|request_id| matches(request_id)))
    }

    pub fn remove(&self, request_id: &str) {
        if let Some(entry) = self.remove_entry(request_id) {
            entry.terminal.mark_terminal();
        }
    }

    pub fn pending_count(&self) -> usize {
        self.inner.pending.lock().map_or(0, |pending| pending.len())
    }

    pub fn active_lease_count(&self) -> usize {
        self.inner.leases_active.load(Ordering::Acquire)
    }

    pub fn cancel_send_failed_closed_count(&self) -> usize {
        self.inner.cancel_send_failed_closed.load(Ordering::Acquire)
    }

    fn remove_entry(&self, request_id: &str) -> Option<OutboundRequestEntry> {
        self.inner.pending.lock().ok()?.remove(request_id)
    }

    fn release_lease(&self) {
        self.inner.leases_active.fetch_sub(1, Ordering::AcqRel);
    }

    fn record_cancel_send_failed_closed(&self) {
        self.inner
            .cancel_send_failed_closed
            .fetch_add(1, Ordering::AcqRel);
    }
}

impl fmt::Debug for OutboundRequestRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OutboundRequestRegistry")
            .field("pending", &self.pending_count())
            .field("leases_active", &self.active_lease_count())
            .field(
                "cancel_send_failed_closed",
                &self.cancel_send_failed_closed_count(),
            )
            .finish()
    }
}

impl OutboundRequestLease {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn terminal_signal(&self) -> OutboundRequestTerminalSignal {
        self.terminal.clone()
    }

    pub fn complete(&self) {
        if self.terminal.mark_terminal() {
            let _ = self.registry.remove_entry(&self.request_id);
        }
    }

    pub fn cancel(&self, reason: &str) {
        if self.terminal.mark_terminal() {
            let _ = self.registry.remove_entry(&self.request_id);
            self.send_cancel(reason);
        }
    }

    fn send_cancel(&self, reason: &str) {
        let Some(cancel_sender) = &self.cancel_sender else {
            return;
        };
        if matches!(
            cancel_sender(&self.request_id, reason),
            Err(OutboundRequestCancelSendError::Closed)
        ) {
            self.registry.record_cancel_send_failed_closed();
        }
    }
}

impl Drop for OutboundRequestLease {
    fn drop(&mut self) {
        if self.terminal.mark_terminal() {
            let _ = self.registry.remove_entry(&self.request_id);
            self.send_cancel(self.drop_cancel_reason);
        }
        self.registry.release_lease();
    }
}

impl OutboundRequestTerminalSignal {
    fn new() -> Self {
        Self {
            inner: Arc::new(OutboundRequestTerminalState {
                terminal: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.inner.terminal.load(Ordering::Acquire)
    }

    pub async fn wait_terminal(&self) {
        loop {
            if self.is_terminal() {
                return;
            }
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.is_terminal() {
                return;
            }
            notified.await;
        }
    }

    fn mark_terminal(&self) -> bool {
        if self
            .inner
            .terminal
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.inner.notify.notify_waiters();
            true
        } else {
            false
        }
    }
}

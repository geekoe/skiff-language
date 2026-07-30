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

use crate::ResponseError;

#[derive(Debug, Clone, PartialEq)]
pub enum OutboundResponse {
    End { payload: Vec<u8> },
    Error(ResponseError),
}

impl OutboundResponse {
    pub fn kind(&self) -> &'static str {
        match self {
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

    /// Atomically claims a pending request's terminal outcome.
    ///
    /// The returned sender belongs to the sole terminal winner. A concurrent
    /// cancellation, disconnect, duplicate, or late response receives `None`.
    pub fn take_terminal_sender(&self, request_id: &str) -> Option<OutboundResponseSender> {
        self.take_terminal_entry(request_id)
            .map(|entry| entry.sender)
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
        let _ = self.take_terminal_sender(request_id);
    }

    pub fn fail_all(&self, error: ResponseError) -> usize {
        let entries = {
            let Ok(mut pending) = self.inner.pending.lock() else {
                return 0;
            };
            pending
                .drain()
                .filter_map(|(_, entry)| entry.terminal.mark_terminal().then_some(entry))
                .collect::<Vec<_>>()
        };
        let count = entries.len();
        for entry in entries {
            let _ = entry.sender.send(OutboundResponse::Error(error.clone()));
        }
        count
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

    fn take_terminal_entry(&self, request_id: &str) -> Option<OutboundRequestEntry> {
        let mut pending = self.inner.pending.lock().ok()?;
        let entry = pending.remove(request_id)?;
        entry.terminal.mark_terminal().then_some(entry)
    }

    fn claim_lease_terminal(
        &self,
        request_id: &str,
        terminal: &OutboundRequestTerminalSignal,
    ) -> bool {
        let Ok(mut pending) = self.inner.pending.lock() else {
            return false;
        };
        if !pending
            .get(request_id)
            .is_some_and(|entry| entry.terminal.is_same(terminal))
        {
            return false;
        }
        pending
            .remove(request_id)
            .is_some_and(|entry| entry.terminal.mark_terminal())
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
        let _ = self
            .registry
            .claim_lease_terminal(&self.request_id, &self.terminal);
    }

    pub fn cancel(&self, reason: &str) -> bool {
        if self
            .registry
            .claim_lease_terminal(&self.request_id, &self.terminal)
        {
            self.send_cancel(reason);
            true
        } else {
            false
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
        if self
            .registry
            .claim_lease_terminal(&self.request_id, &self.terminal)
        {
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

    fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_sender_take_commits_once_before_response_delivery() {
        let registry = OutboundRequestRegistry::default();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let lease = registry
            .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
            .expect("pending request");
        let terminal = lease.terminal_signal();

        let sender = registry
            .take_terminal_sender("pending")
            .expect("first terminal take must win");
        terminal.wait_terminal().await;
        assert!(terminal.is_terminal());
        assert_eq!(registry.pending_count(), 0);
        assert!(registry.take_terminal_sender("pending").is_none());
        assert!(!lease.cancel("late_cancel"));

        sender
            .send(OutboundResponse::End {
                payload: b"response".to_vec(),
            })
            .expect("terminal winner must retain the response sender");
        assert_eq!(
            receiver.recv().await,
            Some(OutboundResponse::End {
                payload: b"response".to_vec()
            })
        );
        drop(lease);
        assert_eq!(registry.active_lease_count(), 0);
    }

    #[test]
    fn cancellation_winner_fences_terminal_sender_take() {
        let registry = OutboundRequestRegistry::default();
        let (sender, _receiver) = mpsc::unbounded_channel();
        let lease = registry
            .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
            .expect("pending request");

        assert!(lease.cancel("caller_cancel"));
        assert!(registry.take_terminal_sender("pending").is_none());
        assert_eq!(registry.pending_count(), 0);
    }

    #[test]
    fn terminal_sender_take_and_cancellation_have_exactly_one_concurrent_winner() {
        use std::sync::Barrier;

        for iteration in 0..128 {
            let registry = OutboundRequestRegistry::default();
            let (sender, _receiver) = mpsc::unbounded_channel();
            let lease = registry
                .insert_with_lease(
                    format!("pending-{iteration}"),
                    sender,
                    None,
                    "caller_cancel",
                )
                .expect("pending request");
            let barrier = Barrier::new(3);

            let (response_won, cancellation_won) = std::thread::scope(|scope| {
                let response = scope.spawn(|| {
                    barrier.wait();
                    registry.take_terminal_sender(lease.request_id()).is_some()
                });
                let cancellation = scope.spawn(|| {
                    barrier.wait();
                    lease.cancel("caller_cancel")
                });
                barrier.wait();
                (
                    response.join().expect("response contender"),
                    cancellation.join().expect("cancellation contender"),
                )
            });

            assert_ne!(response_won, cancellation_won);
            assert_eq!(registry.pending_count(), 0);
        }
    }

    #[tokio::test]
    async fn fail_all_delivers_error_and_commits_each_pending_request() {
        let registry = OutboundRequestRegistry::default();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let lease = registry
            .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
            .expect("pending request");
        let terminal = lease.terminal_signal();

        assert_eq!(
            registry.fail_all(ResponseError {
                code: "ConnectionClosed".to_string(),
                message: "router connection closed".to_string(),
                status: None,
                details: None,
            }),
            1
        );
        terminal.wait_terminal().await;
        assert!(matches!(
            receiver.recv().await,
            Some(OutboundResponse::Error(error))
                if error.code == "ConnectionClosed"
                    && error.message == "router connection closed"
        ));
        assert_eq!(registry.pending_count(), 0);
        assert_eq!(
            registry.fail_all(ResponseError {
                code: "ConnectionClosed".to_string(),
                message: "router connection closed".to_string(),
                status: None,
                details: None,
            }),
            0
        );
        drop(lease);
        assert_eq!(registry.active_lease_count(), 0);
    }
}

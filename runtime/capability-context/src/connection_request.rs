use std::{
    collections::HashMap,
    future,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, Weak,
    },
};

use tokio::{
    sync::oneshot,
    time::{sleep_until, Instant},
};

use crate::CancellationToken;

const REQUEST_ID_PREFIX: &str = "skiff-connection-request-v1:opaque:";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConnectionRequestSession(String);

impl ConnectionRequestSession {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty() || value.trim() != value {
            return Err("connection request session must be a non-empty canonical token".into());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectionRequestCancelReason {
    CallerCancel,
    DeadlineExceeded,
}

impl ConnectionRequestCancelReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CallerCancel => "caller_cancel",
            Self::DeadlineExceeded => "deadline_exceeded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionRequestTerminal {
    Success(Vec<u8>),
    DeadlineExceeded,
    ConnectionUnavailable,
    TransportUnavailable,
    ProtocolError,
    ResourceLimit,
    Remote {
        code: i64,
        message: String,
        data: Option<Vec<u8>>,
    },
    AncestorCancelled,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ConnectionRequestRegistryError {
    PendingLimit { limit: usize },
    CorrelationExhausted,
}

impl std::fmt::Display for ConnectionRequestRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PendingLimit { limit } => {
                write!(
                    formatter,
                    "connection request pending limit {limit} reached"
                )
            }
            Self::CorrelationExhausted => {
                formatter.write_str("connection request correlation id space exhausted")
            }
        }
    }
}

impl std::error::Error for ConnectionRequestRegistryError {}

pub type ConnectionRequestCancelSender =
    Arc<dyn Fn(&str, ConnectionRequestCancelReason) -> Result<(), ()> + Send + Sync>;

pub struct ConnectionRequestRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    capacity: usize,
    next_correlation: AtomicU64,
    pending: Mutex<HashMap<String, Arc<PendingEntry>>>,
    active_leases: AtomicUsize,
    active_timers: AtomicUsize,
}

struct PendingEntry {
    request_id: String,
    session: ConnectionRequestSession,
    sender: Mutex<Option<oneshot::Sender<ConnectionRequestTerminal>>>,
    cancel_sender: ConnectionRequestCancelSender,
    settled: AtomicBool,
    lease_active: AtomicBool,
    timer_active: AtomicBool,
}

pub struct PendingConnectionRequest {
    registry: Weak<RegistryInner>,
    entry: Arc<PendingEntry>,
    receiver: oneshot::Receiver<ConnectionRequestTerminal>,
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    finished: bool,
}

impl ConnectionRequestRegistry {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                capacity,
                next_correlation: AtomicU64::new(1),
                pending: Mutex::new(HashMap::new()),
                active_leases: AtomicUsize::new(0),
                active_timers: AtomicUsize::new(0),
            }),
        }
    }

    pub fn install(
        &self,
        session: ConnectionRequestSession,
        cancellation: CancellationToken,
        deadline: Option<Instant>,
        cancel_sender: ConnectionRequestCancelSender,
    ) -> Result<PendingConnectionRequest, ConnectionRequestRegistryError> {
        let mut pending = lock(&self.inner.pending);
        if pending.len() >= self.inner.capacity {
            return Err(ConnectionRequestRegistryError::PendingLimit {
                limit: self.inner.capacity,
            });
        }
        let correlation = self
            .inner
            .next_correlation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| ConnectionRequestRegistryError::CorrelationExhausted)?;
        let request_id = format!("{REQUEST_ID_PREFIX}{correlation}");
        let (sender, receiver) = oneshot::channel();
        let timer_active = deadline.is_some();
        let entry = Arc::new(PendingEntry {
            request_id: request_id.clone(),
            session,
            sender: Mutex::new(Some(sender)),
            cancel_sender,
            settled: AtomicBool::new(false),
            lease_active: AtomicBool::new(true),
            timer_active: AtomicBool::new(timer_active),
        });
        pending.insert(request_id, entry.clone());
        self.inner.active_leases.fetch_add(1, Ordering::AcqRel);
        if timer_active {
            self.inner.active_timers.fetch_add(1, Ordering::AcqRel);
        }
        drop(pending);

        Ok(PendingConnectionRequest {
            registry: Arc::downgrade(&self.inner),
            entry,
            receiver,
            cancellation,
            deadline,
            finished: false,
        })
    }

    pub fn complete(
        &self,
        session: &ConnectionRequestSession,
        request_id: &str,
        terminal: ConnectionRequestTerminal,
    ) -> bool {
        let entry = {
            let pending = lock(&self.inner.pending);
            pending.get(request_id).cloned()
        };
        let Some(entry) = entry else {
            return false;
        };
        if &entry.session != session {
            return false;
        }
        settle(&self.inner, &entry, terminal)
    }

    pub fn disconnect_session(&self, session: &ConnectionRequestSession) -> usize {
        let entries = {
            let pending = lock(&self.inner.pending);
            pending
                .values()
                .filter(|entry| &entry.session == session)
                .cloned()
                .collect::<Vec<_>>()
        };
        entries
            .into_iter()
            .filter(|entry| {
                settle(
                    &self.inner,
                    entry,
                    ConnectionRequestTerminal::TransportUnavailable,
                )
            })
            .count()
    }

    pub fn pending_count(&self) -> usize {
        lock(&self.inner.pending).len()
    }

    pub fn active_lease_count(&self) -> usize {
        self.inner.active_leases.load(Ordering::Acquire)
    }

    pub fn active_timer_count(&self) -> usize {
        self.inner.active_timers.load(Ordering::Acquire)
    }
}

impl Drop for ConnectionRequestRegistry {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) != 1 {
            return;
        }
        let entries = lock(&self.inner.pending)
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for entry in entries {
            settle(
                &self.inner,
                &entry,
                ConnectionRequestTerminal::TransportUnavailable,
            );
        }
    }
}

impl PendingConnectionRequest {
    pub fn request_id(&self) -> &str {
        &self.entry.request_id
    }

    pub fn session(&self) -> &ConnectionRequestSession {
        &self.entry.session
    }

    pub async fn wait(&mut self) -> ConnectionRequestTerminal {
        if self.finished {
            return ConnectionRequestTerminal::ProtocolError;
        }

        let deadline = self.deadline;
        let deadline_wait = async move {
            match deadline {
                Some(deadline) => sleep_until(deadline).await,
                None => future::pending().await,
            }
        };
        tokio::pin!(deadline_wait);

        let terminal = tokio::select! {
            biased;
            _ = self.cancellation.wait_cancelled() => {
                self.cancel_and_settle(
                    ConnectionRequestCancelReason::CallerCancel,
                    ConnectionRequestTerminal::AncestorCancelled,
                );
                receive_terminal(&mut self.receiver).await
            }
            _ = &mut deadline_wait => {
                self.cancel_and_settle(
                    ConnectionRequestCancelReason::DeadlineExceeded,
                    ConnectionRequestTerminal::DeadlineExceeded,
                );
                receive_terminal(&mut self.receiver).await
            }
            result = &mut self.receiver => {
                result.unwrap_or(ConnectionRequestTerminal::TransportUnavailable)
            }
        };
        self.finished = true;
        self.release_lease();
        terminal
    }

    fn cancel_and_settle(
        &self,
        reason: ConnectionRequestCancelReason,
        terminal: ConnectionRequestTerminal,
    ) {
        let Some(registry) = self.registry.upgrade() else {
            return;
        };
        if settle(&registry, &self.entry, terminal) {
            self.release_lease();
            let _ = (self.entry.cancel_sender)(&self.entry.request_id, reason);
        }
    }

    fn release_lease(&self) {
        if !self.entry.lease_active.swap(false, Ordering::AcqRel) {
            return;
        }
        if let Some(registry) = self.registry.upgrade() {
            registry.active_leases.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for PendingConnectionRequest {
    fn drop(&mut self) {
        if !self.finished {
            self.cancel_and_settle(
                ConnectionRequestCancelReason::CallerCancel,
                ConnectionRequestTerminal::AncestorCancelled,
            );
        }
        self.release_lease();
    }
}

fn settle(
    registry: &RegistryInner,
    entry: &PendingEntry,
    terminal: ConnectionRequestTerminal,
) -> bool {
    if entry
        .settled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }
    {
        let mut pending = lock(&registry.pending);
        if pending
            .get(&entry.request_id)
            .is_some_and(|candidate| std::ptr::eq(candidate.as_ref(), entry))
        {
            pending.remove(&entry.request_id);
        }
    }
    if entry.timer_active.swap(false, Ordering::AcqRel) {
        registry.active_timers.fetch_sub(1, Ordering::AcqRel);
    }
    if let Some(sender) = lock(&entry.sender).take() {
        let _ = sender.send(terminal);
    }
    true
}

async fn receive_terminal(
    receiver: &mut oneshot::Receiver<ConnectionRequestTerminal>,
) -> ConnectionRequestTerminal {
    receiver
        .await
        .unwrap_or(ConnectionRequestTerminal::TransportUnavailable)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

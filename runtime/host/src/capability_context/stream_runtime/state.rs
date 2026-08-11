use std::{
    collections::HashMap,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde_json::Value;
use skiff_runtime_capability_context::{
    CancellationToken, StreamInternalItem, StreamLifetimeGuard, StreamPullSource,
    StreamRuntimeError,
};
use tokio::sync::{mpsc, Mutex as AsyncMutex, Notify};

#[derive(Debug)]
pub(super) enum StreamEvent {
    Item(Value),
    InternalItem(StreamInternalItem),
    End,
    Error(StreamRuntimeError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StreamTerminalReason {
    End,
    Error,
    Cancelled,
    SourceDropped,
}

pub(super) struct StreamState {
    pub(super) scope: Option<u64>,
    pub(super) source: StreamSource,
    pub(super) cancelled: Arc<AtomicBool>,
    pub(super) cancel_notify: Arc<Notify>,
    pub(super) cancellation: Option<CancellationToken>,
    pub(super) lifetime: Mutex<Option<StreamLifetimeGuard>>,
    pub(super) ended: AtomicBool,
}

pub(super) enum StreamSource {
    Channel {
        receiver: AsyncMutex<mpsc::Receiver<StreamEvent>>,
        terminal: Arc<ChannelTerminalState>,
    },
    Pull(AsyncMutex<Box<dyn StreamPullSource>>),
}

#[derive(Debug)]
pub(super) enum ChannelTerminal {
    End,
    Error(StreamRuntimeError),
}

#[derive(Debug)]
pub(super) struct ChannelTerminalState {
    slot: Mutex<ChannelTerminalSlot>,
    pub(super) notify: Notify,
}

#[derive(Debug)]
enum ChannelTerminalSlot {
    Open,
    Pending(ChannelTerminal),
    Consumed,
}

#[derive(Debug, Default)]
pub(super) struct StreamRegistry {
    streams: HashMap<String, Arc<StreamState>>,
    active_scopes: HashMap<u64, usize>,
    owner_closed: bool,
}

impl StreamRegistry {
    pub(super) fn register(&mut self, id: String, state: Arc<StreamState>) -> bool {
        let scope_active = state
            .scope
            .map(|scope| self.active_scopes.contains_key(&scope))
            .unwrap_or(true);
        if self.owner_closed || !scope_active {
            return false;
        }
        self.streams.insert(id, state);
        true
    }

    pub(super) fn get(&self, id: &str) -> Option<Arc<StreamState>> {
        self.streams.get(id).cloned()
    }

    pub(super) fn remove(&mut self, id: &str) -> Option<Arc<StreamState>> {
        self.streams.remove(id)
    }

    pub(super) fn drain_all(&mut self) -> Vec<Arc<StreamState>> {
        self.streams.drain().map(|(_, state)| state).collect()
    }

    pub(super) fn active_count(&self) -> usize {
        self.streams.len()
    }

    pub(super) fn active_count_in_scope(&self, scope: u64) -> usize {
        self.streams
            .values()
            .filter(|state| state.scope == Some(scope))
            .count()
    }

    pub(super) fn open_scope(&mut self, scope: u64) {
        if !self.owner_closed {
            *self.active_scopes.entry(scope).or_default() += 1;
        }
    }

    pub(super) fn close_scope(&mut self, scope: u64) -> Vec<Arc<StreamState>> {
        let Some(open_count) = self.active_scopes.get_mut(&scope) else {
            return Vec::new();
        };
        if *open_count > 1 {
            *open_count -= 1;
            return Vec::new();
        }
        self.active_scopes.remove(&scope);
        let ids = self
            .streams
            .iter()
            .filter(|&(_, state)| state.scope == Some(scope))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        ids.into_iter()
            .filter_map(|id| self.streams.remove(&id))
            .collect()
    }

    pub(super) fn close_owner(&mut self) {
        self.owner_closed = true;
        self.active_scopes.clear();
    }
}

impl StreamState {
    pub(super) fn finish(&self, terminal: StreamTerminalReason) -> bool {
        if self
            .ended
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        self.cancelled.store(true, Ordering::SeqCst);
        if matches!(
            terminal,
            StreamTerminalReason::Cancelled | StreamTerminalReason::SourceDropped
        ) {
            if let Some(cancellation) = &self.cancellation {
                cancellation.cancel();
            }
        }
        self.cancel_notify.notify_waiters();
        self.lifetime
            .lock()
            .expect("stream lifetime mutex poisoned")
            .take();
        true
    }
}

impl Default for ChannelTerminalState {
    fn default() -> Self {
        Self {
            slot: Mutex::new(ChannelTerminalSlot::Open),
            notify: Notify::new(),
        }
    }
}

impl ChannelTerminalState {
    pub(super) fn publish(&self, terminal: ChannelTerminal) {
        let mut slot = self.slot.lock().expect("stream terminal mutex poisoned");
        if matches!(*slot, ChannelTerminalSlot::Open) {
            *slot = ChannelTerminalSlot::Pending(terminal);
            self.notify.notify_waiters();
        }
    }

    pub(super) fn send_if_open(
        &self,
        permit: mpsc::Permit<'_, StreamEvent>,
        event: StreamEvent,
    ) -> bool {
        let slot = self.slot.lock().expect("stream terminal mutex poisoned");
        if !matches!(*slot, ChannelTerminalSlot::Open) {
            return false;
        }
        permit.send(event);
        true
    }

    pub(super) fn take_event(&self) -> Option<StreamEvent> {
        let mut slot = self.slot.lock().expect("stream terminal mutex poisoned");
        let terminal = match std::mem::replace(&mut *slot, ChannelTerminalSlot::Consumed) {
            ChannelTerminalSlot::Pending(terminal) => terminal,
            other => {
                *slot = other;
                return None;
            }
        };
        Some(match terminal {
            ChannelTerminal::End => StreamEvent::End,
            ChannelTerminal::Error(error) => StreamEvent::Error(error),
        })
    }
}

impl fmt::Debug for StreamState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StreamState")
            .field("source", &self.source)
            .field("cancelled", &self.cancelled.load(Ordering::SeqCst))
            .field("ended", &self.ended.load(Ordering::SeqCst))
            .finish()
    }
}

impl fmt::Debug for StreamSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Channel { .. } => formatter.write_str("Channel"),
            Self::Pull(_) => formatter.write_str("Pull"),
        }
    }
}

use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    Arc, Mutex, Weak,
};

use crate::{capability::CallbackCapabilityTableState, ActivationContext, ActivationContextError};

static REQUEST_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackLifetime {
    Request,
    Stream,
}

struct RequestLifecycleState {
    end_requested: bool,
    cancelled: bool,
    open_streams: usize,
    capability_tables: Vec<Weak<CallbackCapabilityTableState>>,
}

pub(crate) struct RequestLifecycle {
    generation: u64,
    request_contexts: AtomicUsize,
    state: Mutex<RequestLifecycleState>,
}

impl RequestLifecycle {
    fn new(generation: u64) -> Self {
        Self {
            generation,
            request_contexts: AtomicUsize::new(1),
            state: Mutex::new(RequestLifecycleState {
                end_requested: false,
                cancelled: false,
                open_streams: 0,
                capability_tables: Vec::new(),
            }),
        }
    }

    pub(crate) fn weak(this: &Arc<Self>) -> Weak<Self> {
        Arc::downgrade(this)
    }

    pub(crate) fn capability_is_active(&self, lifetime: CallbackLifetime) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if state.cancelled {
            return false;
        }
        match lifetime {
            CallbackLifetime::Request => !state.end_requested,
            CallbackLifetime::Stream => state.open_streams > 0,
        }
    }

    pub(crate) fn register_capability_table(&self, table: &Arc<CallbackCapabilityTableState>) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state
            .capability_tables
            .retain(|table| table.strong_count() > 0);
        let table = Arc::downgrade(table);
        if !state
            .capability_tables
            .iter()
            .any(|registered| Weak::ptr_eq(registered, &table))
        {
            state.capability_tables.push(table);
        }
    }

    fn retain_request_context(&self) {
        self.request_contexts
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("request context reference count exhausted");
    }

    fn release_request_context(&self) {
        let previous = self
            .request_contexts
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(1)
            })
            .expect("request context count must not underflow");
        if previous == 1 {
            self.end_request();
        }
    }

    fn end_request(&self) {
        let Some((tables, drain_stream)) = self.with_registered_tables(|state| {
            state.end_requested = true;
            state.open_streams == 0
        }) else {
            return;
        };
        for table in tables {
            table.drain_request_lifetime(self.generation);
            if drain_stream {
                table.drain_stream_lifetime(self.generation);
            }
        }
    }

    fn cancel(&self) {
        let Some((tables, ())) = self.with_registered_tables(|state| {
            state.cancelled = true;
        }) else {
            return;
        };
        for table in tables {
            table.drain_request_generation(self.generation);
        }
    }

    fn open_stream(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.cancelled || state.end_requested {
            return false;
        }
        let Some(open_streams) = state.open_streams.checked_add(1) else {
            return false;
        };
        state.open_streams = open_streams;
        true
    }

    fn close_stream(&self) {
        let Some((tables, drain_stream)) = self.with_registered_tables(|state| {
            state.open_streams = state.open_streams.checked_sub(1).unwrap_or(0);
            state.open_streams == 0
        }) else {
            return;
        };
        if drain_stream {
            for table in tables {
                table.drain_stream_lifetime(self.generation);
            }
        }
    }

    fn with_registered_tables<T>(
        &self,
        transition: impl FnOnce(&mut RequestLifecycleState) -> T,
    ) -> Option<(Vec<Arc<CallbackCapabilityTableState>>, T)> {
        let mut state = self.state.lock().ok()?;
        let result = transition(&mut state);
        let tables = state
            .capability_tables
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        state
            .capability_tables
            .retain(|table| table.strong_count() > 0);
        Some((tables, result))
    }
}

impl std::fmt::Debug for RequestLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestLifecycle")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Drop for RequestLifecycle {
    fn drop(&mut self) {
        let tables = self
            .state
            .get_mut()
            .map(|state| {
                state
                    .capability_tables
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for table in tables {
            table.drain_request_generation(self.generation);
        }
    }
}

#[derive(Debug)]
pub struct RequestActivationContext {
    receiver: Arc<ActivationContext>,
    current: Arc<ActivationContext>,
    generation: u64,
    lifecycle: Arc<RequestLifecycle>,
}

impl RequestActivationContext {
    pub fn begin(receiver: Arc<ActivationContext>) -> Result<Self, ActivationContextError> {
        let generation = REQUEST_GENERATION
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| ActivationContextError::RequestGenerationExhausted)?;
        let lifecycle = Arc::new(RequestLifecycle::new(generation));
        Ok(Self {
            receiver: Arc::clone(&receiver),
            current: receiver,
            generation,
            lifecycle,
        })
    }

    pub fn receiver(&self) -> &Arc<ActivationContext> {
        &self.receiver
    }

    pub fn current(&self) -> &Arc<ActivationContext> {
        &self.current
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn switch_to(
        &self,
        current: Arc<ActivationContext>,
    ) -> Result<Self, ActivationContextError> {
        let receiver_identity = self.receiver.identity();
        let target_identity = current.identity();
        if receiver_identity.assembly_identity != target_identity.assembly_identity
            || receiver_identity.assembly_generation != target_identity.assembly_generation
            || receiver_identity.runtime_replica_id != target_identity.runtime_replica_id
        {
            return Err(ActivationContextError::CrossAssemblyActivationSwitch);
        }
        self.lifecycle.retain_request_context();
        Ok(Self {
            receiver: Arc::clone(&self.receiver),
            current,
            generation: self.generation,
            lifecycle: Arc::clone(&self.lifecycle),
        })
    }

    pub fn restore_receiver(&self) -> Self {
        self.lifecycle.retain_request_context();
        Self {
            receiver: Arc::clone(&self.receiver),
            current: Arc::clone(&self.receiver),
            generation: self.generation,
            lifecycle: Arc::clone(&self.lifecycle),
        }
    }

    pub fn end_request(&self) {
        self.lifecycle.end_request();
    }

    pub fn cancel(&self) {
        self.lifecycle.cancel();
    }

    pub fn open_stream(&self) -> Option<RequestStreamLease> {
        self.lifecycle.open_stream().then(|| RequestStreamLease {
            lifecycle: Arc::clone(&self.lifecycle),
            closed: AtomicBool::new(false),
        })
    }

    pub(crate) fn lifecycle(&self) -> &Arc<RequestLifecycle> {
        &self.lifecycle
    }
}

impl Clone for RequestActivationContext {
    fn clone(&self) -> Self {
        self.lifecycle.retain_request_context();
        Self {
            receiver: Arc::clone(&self.receiver),
            current: Arc::clone(&self.current),
            generation: self.generation,
            lifecycle: Arc::clone(&self.lifecycle),
        }
    }
}

impl Drop for RequestActivationContext {
    fn drop(&mut self) {
        self.lifecycle.release_request_context();
    }
}

#[derive(Debug)]
pub struct RequestStreamLease {
    lifecycle: Arc<RequestLifecycle>,
    closed: AtomicBool,
}

impl RequestStreamLease {
    pub fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            self.lifecycle.close_stream();
        }
    }
}

impl Drop for RequestStreamLease {
    fn drop(&mut self) {
        self.close();
    }
}

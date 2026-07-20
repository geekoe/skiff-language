use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex, Weak,
};

use crate::{ActivationContext, ActivationContextError};

static REQUEST_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackLifetime {
    Request,
    Stream,
}

#[derive(Debug)]
struct RequestLifecycleState {
    end_requested: bool,
    cancelled: bool,
    open_streams: usize,
}

#[derive(Debug)]
pub(crate) struct RequestLifecycle {
    state: Mutex<RequestLifecycleState>,
}

impl RequestLifecycle {
    fn new() -> Self {
        Self {
            state: Mutex::new(RequestLifecycleState {
                end_requested: false,
                cancelled: false,
                open_streams: 0,
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
        if state.cancelled || (state.end_requested && state.open_streams == 0) {
            return false;
        }
        match lifetime {
            CallbackLifetime::Request => true,
            CallbackLifetime::Stream => state.open_streams > 0,
        }
    }

    fn end_request(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.end_requested = true;
        }
    }

    fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cancelled = true;
        }
    }

    fn open_stream(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.cancelled || state.end_requested {
            return false;
        }
        state.open_streams = state.open_streams.saturating_add(1);
        true
    }

    fn close_stream(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.open_streams = state.open_streams.saturating_sub(1);
        }
    }
}

#[derive(Debug, Clone)]
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
        let lifecycle = Arc::new(RequestLifecycle::new());
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
        Ok(Self {
            receiver: Arc::clone(&self.receiver),
            current,
            generation: self.generation,
            lifecycle: Arc::clone(&self.lifecycle),
        })
    }

    pub fn restore_receiver(&self) -> Self {
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

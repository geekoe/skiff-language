#![allow(dead_code)]

pub(crate) mod handoff;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use skiff_artifact_model::ActorImplementationIdentity;
use skiff_runtime_capability_context::ActorInvocationOutcome;
use tokio::sync::oneshot;
use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct ActorMethodOutboundRegistry {
    inner: Arc<Mutex<HashMap<String, ActorMethodOutboundEntry>>>,
}

struct ActorMethodOutboundEntry {
    cancellation_correlation: String,
    expected_epoch: u64,
    expected_implementation_identity: ActorImplementationIdentity,
    response_committed: ActorMethodResponseCommitted,
    sender: oneshot::Sender<Result<ActorInvocationOutcome, ActorInvocationTransportError>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorInvocationTransportError {
    pub code: String,
    pub message: String,
}

pub struct ActorMethodOutboundLease {
    invocation_id: String,
    registry: ActorMethodOutboundRegistry,
    response_committed: ActorMethodResponseCommitted,
    receiver:
        Option<oneshot::Receiver<Result<ActorInvocationOutcome, ActorInvocationTransportError>>>,
}

#[derive(Clone)]
pub(crate) struct ActorMethodResponseCommitted {
    inner: Arc<ActorMethodResponseCommittedState>,
}

struct ActorMethodResponseCommittedState {
    committed: AtomicBool,
    notify: Notify,
}

impl ActorMethodOutboundRegistry {
    pub fn register(
        &self,
        invocation_id: String,
        cancellation_correlation: String,
        expected_epoch: u64,
        expected_implementation_identity: ActorImplementationIdentity,
    ) -> Result<ActorMethodOutboundLease, String> {
        let (sender, receiver) = oneshot::channel();
        let response_committed = ActorMethodResponseCommitted::new();
        let mut entries = self
            .inner
            .lock()
            .map_err(|_| "Actor method outbound registry lock is poisoned".to_string())?;
        if entries.contains_key(&invocation_id) {
            return Err(format!(
                "duplicate Actor method invocation id {invocation_id}"
            ));
        }
        entries.insert(
            invocation_id.clone(),
            ActorMethodOutboundEntry {
                cancellation_correlation,
                expected_epoch,
                expected_implementation_identity,
                response_committed: response_committed.clone(),
                sender,
            },
        );
        Ok(ActorMethodOutboundLease {
            invocation_id,
            registry: self.clone(),
            response_committed,
            receiver: Some(receiver),
        })
    }

    pub fn complete(&self, invocation_id: &str, outcome: ActorInvocationOutcome) -> bool {
        let Some(entry) = self
            .inner
            .lock()
            .ok()
            .and_then(|mut entries| entries.remove(invocation_id))
        else {
            return false;
        };
        entry.response_committed.commit();
        entry.sender.send(Ok(outcome)).is_ok()
    }

    pub fn complete_failure(
        &self,
        invocation_id: &str,
        epoch: u64,
        implementation_identity: &ActorImplementationIdentity,
        error: ActorInvocationTransportError,
    ) -> bool {
        let Ok(mut entries) = self.inner.lock() else {
            return false;
        };
        let Some(entry) = entries.get(invocation_id) else {
            return false;
        };
        if entry.expected_epoch != epoch
            || entry.expected_implementation_identity != *implementation_identity
        {
            return false;
        }
        let Some(entry) = entries.remove(invocation_id) else {
            return false;
        };
        entry.response_committed.commit();
        entry.sender.send(Err(error)).is_ok()
    }

    pub fn cancellation_correlation(&self, invocation_id: &str) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .get(invocation_id)
            .map(|entry| entry.cancellation_correlation.clone())
    }

    pub fn fail_all(&self, error: ActorInvocationTransportError) -> usize {
        let entries = {
            let Ok(mut entries) = self.inner.lock() else {
                return 0;
            };
            entries.drain().map(|(_, entry)| entry).collect::<Vec<_>>()
        };
        let count = entries.len();
        for entry in entries {
            entry.response_committed.commit();
            let _ = entry.sender.send(Err(error.clone()));
        }
        count
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.inner.lock().map_or(0, |entries| entries.len())
    }
}

impl ActorMethodOutboundLease {
    pub(crate) fn response_committed(&self) -> ActorMethodResponseCommitted {
        self.response_committed.clone()
    }

    pub async fn receive(
        &mut self,
    ) -> Result<
        Result<ActorInvocationOutcome, ActorInvocationTransportError>,
        oneshot::error::RecvError,
    > {
        self.receiver
            .take()
            .expect("Actor method outbound lease can only be received once")
            .await
    }
}

impl ActorMethodResponseCommitted {
    fn new() -> Self {
        Self {
            inner: Arc::new(ActorMethodResponseCommittedState {
                committed: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub(crate) async fn wait(&self) {
        loop {
            if self.inner.committed.load(Ordering::Acquire) {
                return;
            }
            let notified = self.inner.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.inner.committed.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }

    fn commit(&self) {
        if self
            .inner
            .committed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.inner.notify.notify_waiters();
        }
    }
}

impl Drop for ActorMethodOutboundLease {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.registry.inner.lock() {
            entries.remove(&self.invocation_id);
        }
    }
}

#[cfg(test)]
mod tests;

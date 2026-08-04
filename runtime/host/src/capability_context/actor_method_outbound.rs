use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use skiff_artifact_model::ActorImplementationIdentity;
use skiff_runtime_capability_context::ActorInvocationOutcome;
use tokio::sync::oneshot;

#[derive(Clone, Default)]
pub struct ActorMethodOutboundRegistry {
    inner: Arc<Mutex<HashMap<String, ActorMethodOutboundEntry>>>,
}

struct ActorMethodOutboundEntry {
    cancellation_correlation: String,
    expected_epoch: u64,
    expected_implementation_identity: ActorImplementationIdentity,
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
    receiver:
        Option<oneshot::Receiver<Result<ActorInvocationOutcome, ActorInvocationTransportError>>>,
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
                sender,
            },
        );
        Ok(ActorMethodOutboundLease {
            invocation_id,
            registry: self.clone(),
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

impl Drop for ActorMethodOutboundLease {
    fn drop(&mut self) {
        if let Ok(mut entries) = self.registry.inner.lock() {
            entries.remove(&self.invocation_id);
        }
    }
}

#[cfg(test)]
mod tests;

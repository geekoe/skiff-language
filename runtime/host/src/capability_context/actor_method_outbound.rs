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
        self.inner
            .lock()
            .ok()
            .and_then(|mut entries| entries.remove(invocation_id))
            .is_some_and(|entry| entry.sender.send(Ok(outcome)).is_ok())
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
        entries
            .remove(invocation_id)
            .is_some_and(|entry| entry.sender.send(Err(error)).is_ok())
    }

    pub fn cancellation_correlation(&self, invocation_id: &str) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .get(invocation_id)
            .map(|entry| entry.cancellation_correlation.clone())
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
mod tests {
    use super::*;

    fn implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new(format!(
            "skiff-actor-implementation-v1:sha256:{}",
            "a".repeat(64)
        ))
    }

    #[tokio::test]
    async fn completes_only_the_exact_actor_invocation() {
        let registry = ActorMethodOutboundRegistry::default();
        let mut lease = registry
            .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
            .unwrap();
        assert_eq!(
            registry.cancellation_correlation("invoke-1").as_deref(),
            Some("cancel-1")
        );
        assert!(!registry.complete("invoke-2", ActorInvocationOutcome::Returned(vec![2])));
        assert!(registry.complete("invoke-1", ActorInvocationOutcome::Returned(vec![1])));
        assert_eq!(
            lease.receive().await.unwrap().unwrap(),
            ActorInvocationOutcome::Returned(vec![1])
        );
    }

    #[test]
    fn dropping_lease_removes_pending_invocation() {
        let registry = ActorMethodOutboundRegistry::default();
        let lease = registry
            .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
            .unwrap();
        drop(lease);
        assert_eq!(registry.cancellation_correlation("invoke-1"), None);
    }

    #[tokio::test]
    async fn transport_failure_is_not_reported_as_actor_error_or_cancellation() {
        let registry = ActorMethodOutboundRegistry::default();
        let mut lease = registry
            .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
            .unwrap();
        assert!(!registry.complete_failure(
            "invoke-1",
            2,
            &implementation(),
            ActorInvocationTransportError {
                code: "runtimeExecutionFailed".into(),
                message: "stale".into(),
            }
        ));
        assert!(registry.complete_failure(
            "invoke-1",
            1,
            &implementation(),
            ActorInvocationTransportError {
                code: "runtimeExecutionFailed".into(),
                message: "boom".into(),
            }
        ));
        assert_eq!(
            lease.receive().await.unwrap().unwrap_err(),
            ActorInvocationTransportError {
                code: "runtimeExecutionFailed".into(),
                message: "boom".into(),
            }
        );
    }
}

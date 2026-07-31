use std::{collections::HashMap, sync::Mutex};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_transport::actor_method::ActorMethodCancelReason;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorOwnerCancellationReason {
    Cancelled,
    DeadlineExceeded,
}

impl From<ActorMethodCancelReason> for ActorOwnerCancellationReason {
    fn from(value: ActorMethodCancelReason) -> Self {
        match value {
            ActorMethodCancelReason::Cancelled => Self::Cancelled,
            ActorMethodCancelReason::DeadlineExceeded => Self::DeadlineExceeded,
        }
    }
}

#[derive(Clone)]
struct ActiveInvocation {
    cancellation_correlation: String,
    cancellation: CancellationToken,
    reason: Option<ActorOwnerCancellationReason>,
}

#[derive(Default)]
pub struct ActorOwnerInvocationRegistry {
    active: Mutex<HashMap<String, ActiveInvocation>>,
}

impl ActorOwnerInvocationRegistry {
    pub fn register(
        &self,
        invocation_id: String,
        cancellation_correlation: String,
    ) -> Option<CancellationToken> {
        let cancellation = CancellationToken::new();
        let entry = ActiveInvocation {
            cancellation_correlation,
            cancellation: cancellation.clone(),
            reason: None,
        };
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        if active.contains_key(&invocation_id) {
            return None;
        }
        active.insert(invocation_id, entry);
        Some(cancellation)
    }

    pub fn cancel(
        &self,
        invocation_id: &str,
        cancellation_correlation: &str,
        reason: ActorOwnerCancellationReason,
    ) -> bool {
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        let Some(invocation) = active.get_mut(invocation_id) else {
            return false;
        };
        if invocation.cancellation_correlation != cancellation_correlation {
            return false;
        }
        if invocation.reason.is_none() {
            invocation.reason = Some(reason);
            invocation.cancellation.cancel();
        }
        true
    }

    pub fn finish(&self, invocation_id: &str) -> Option<ActorOwnerCancellationReason> {
        self.active
            .lock()
            .expect("Actor owner invocation registry lock poisoned")
            .remove(invocation_id)
            .and_then(|invocation| invocation.reason)
    }

    pub fn cancel_session(&self) -> usize {
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        let count = active.len();
        for invocation in active.values_mut() {
            invocation
                .reason
                .get_or_insert(ActorOwnerCancellationReason::Cancelled);
            invocation.cancellation.cancel();
        }
        active.clear();
        count
    }
}

#[cfg(test)]
mod tests;

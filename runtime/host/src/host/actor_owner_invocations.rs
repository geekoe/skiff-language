use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use skiff_runtime_capability_context::CancellationToken;
use skiff_runtime_transport::actor_method::ActorMethodCancelReason;

use crate::capability_context::TestRequestRevoker;

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
    generation: u64,
    router_session_id: String,
    cancellation_correlation: String,
    cancellation: CancellationToken,
    reason: Option<ActorOwnerCancellationReason>,
    test_request_revoker: Option<TestRequestRevoker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActorOwnerInvocationIdentity {
    invocation_id: String,
    generation: u64,
    router_session_id: String,
    cancellation_correlation: String,
}

pub(super) struct ActorOwnerInvocationRegistration {
    identity: ActorOwnerInvocationIdentity,
    cancellation: CancellationToken,
}

impl ActorOwnerInvocationRegistration {
    pub(super) fn identity(&self) -> &ActorOwnerInvocationIdentity {
        &self.identity
    }

    pub(super) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

#[derive(Default)]
pub struct ActorOwnerInvocationRegistry {
    active: Mutex<HashMap<String, ActiveInvocation>>,
    next_generation: AtomicU64,
}

impl ActorOwnerInvocationRegistry {
    /// Registers one invocation and returns the only identity that may finish it or fire its
    /// Host-owned deadline. The generation prevents a stale task from acting on a reused wire id.
    #[cfg(test)]
    pub(super) fn register(
        &self,
        invocation_id: String,
        router_session_id: String,
        cancellation_correlation: String,
    ) -> Option<ActorOwnerInvocationRegistration> {
        self.register_with_test_revoker(
            invocation_id,
            router_session_id,
            cancellation_correlation,
            None,
        )
    }

    pub(super) fn register_with_test_revoker(
        &self,
        invocation_id: String,
        router_session_id: String,
        cancellation_correlation: String,
        test_request_revoker: Option<TestRequestRevoker>,
    ) -> Option<ActorOwnerInvocationRegistration> {
        let cancellation = CancellationToken::new();
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        if active.contains_key(&invocation_id) {
            return None;
        }
        let generation = self
            .next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("Actor owner invocation generation exhausted")
            + 1;
        let identity = ActorOwnerInvocationIdentity {
            invocation_id: invocation_id.clone(),
            generation,
            router_session_id: router_session_id.clone(),
            cancellation_correlation: cancellation_correlation.clone(),
        };
        let entry = ActiveInvocation {
            generation,
            router_session_id,
            cancellation_correlation,
            cancellation: cancellation.clone(),
            reason: None,
            test_request_revoker,
        };
        active.insert(invocation_id, entry);
        Some(ActorOwnerInvocationRegistration {
            identity,
            cancellation,
        })
    }

    /// Applies a wire cancellation only to the invocation owned by the receiving Router session.
    pub(super) fn cancel_for_session(
        &self,
        invocation_id: &str,
        router_session_id: &str,
        cancellation_correlation: &str,
        reason: ActorOwnerCancellationReason,
    ) -> bool {
        let test_request_revoker = {
            let mut active = self
                .active
                .lock()
                .expect("Actor owner invocation registry lock poisoned");
            let Some(invocation) = active.get_mut(invocation_id) else {
                return false;
            };
            if invocation.router_session_id != router_session_id
                || invocation.cancellation_correlation != cancellation_correlation
            {
                return false;
            }
            cancel_invocation(invocation, reason);
            invocation.test_request_revoker.clone()
        };
        if let Some(revoker) = test_request_revoker {
            revoker.revoke();
        }
        true
    }

    /// Applies an internal cancellation, such as a deadline, to one exact registration.
    pub(super) fn cancel_registered(
        &self,
        identity: &ActorOwnerInvocationIdentity,
        reason: ActorOwnerCancellationReason,
    ) -> bool {
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        let Some(invocation) = active.get_mut(&identity.invocation_id) else {
            return false;
        };
        if !invocation.matches(identity) {
            return false;
        }
        cancel_invocation(invocation, reason);
        true
    }

    pub(super) fn finish(
        &self,
        identity: &ActorOwnerInvocationIdentity,
    ) -> Option<ActorOwnerCancellationReason> {
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        if !active
            .get(&identity.invocation_id)
            .is_some_and(|invocation| invocation.matches(identity))
        {
            return None;
        }
        active
            .remove(&identity.invocation_id)
            .and_then(|invocation| invocation.reason)
    }

    #[cfg(test)]
    pub(crate) fn contains(&self, invocation_id: &str) -> bool {
        self.active
            .lock()
            .expect("Actor owner invocation registry lock poisoned")
            .contains_key(invocation_id)
    }

    pub(super) fn cancel_session(&self, router_session_id: &str) -> usize {
        let mut active = self
            .active
            .lock()
            .expect("Actor owner invocation registry lock poisoned");
        let mut count = 0;
        active.retain(|_, invocation| {
            if invocation.router_session_id != router_session_id {
                return true;
            }
            count += 1;
            cancel_invocation(invocation, ActorOwnerCancellationReason::Cancelled);
            false
        });
        count
    }
}

impl ActiveInvocation {
    fn matches(&self, identity: &ActorOwnerInvocationIdentity) -> bool {
        self.generation == identity.generation
            && self.router_session_id == identity.router_session_id
            && self.cancellation_correlation == identity.cancellation_correlation
    }
}

fn cancel_invocation(invocation: &mut ActiveInvocation, reason: ActorOwnerCancellationReason) {
    if invocation.reason.is_none() {
        invocation.reason = Some(reason);
        invocation.cancellation.cancel();
    }
}

#[cfg(test)]
mod tests;

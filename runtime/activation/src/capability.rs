use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use skiff_runtime_model::value::CallbackCapabilityCarrier;

use crate::{
    request_context::{CallbackLifetime, RequestLifecycle},
    ActivationContext, RequestActivationContext,
};

pub type CallbackCapabilityPayload = Arc<dyn Any + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallbackCapabilityKey {
    request_generation: u64,
    opaque_capability_id: String,
}

struct CallbackCapabilityEntry {
    contract: String,
    payload: CallbackCapabilityPayload,
    lifecycle: Weak<RequestLifecycle>,
    lifetime: CallbackLifetime,
    state: CallbackCapabilityEntryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallbackCapabilityEntryState {
    Active,
    Expired,
}

pub struct CallbackCapabilityTable {
    owner_runtime_replica_id: String,
    owner_activation_id: String,
    entries: Mutex<HashMap<CallbackCapabilityKey, CallbackCapabilityEntry>>,
    owner_available: Mutex<bool>,
}

impl std::fmt::Debug for CallbackCapabilityTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallbackCapabilityTable")
            .field("owner_runtime_replica_id", &self.owner_runtime_replica_id)
            .field("owner_activation_id", &self.owner_activation_id)
            .finish_non_exhaustive()
    }
}

impl CallbackCapabilityTable {
    pub(crate) fn new(owner_runtime_replica_id: String, owner_activation_id: String) -> Self {
        Self {
            owner_runtime_replica_id,
            owner_activation_id,
            entries: Mutex::new(HashMap::new()),
            owner_available: Mutex::new(true),
        }
    }

    pub fn register(
        &self,
        owner: &ActivationContext,
        request: &RequestActivationContext,
        interface_or_adapter_contract: impl Into<String>,
        opaque_capability_id: impl Into<String>,
        lifetime: CallbackLifetime,
        payload: CallbackCapabilityPayload,
    ) -> Result<CallbackCapabilityCarrier, CallbackCapabilityError> {
        if request.current().activation_id() != owner.activation_id()
            || owner.activation_id().as_str() != self.owner_activation_id
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        if !request.lifecycle().capability_is_active(lifetime) {
            return Err(CallbackCapabilityError::CapabilityExpired);
        }
        if !*self
            .owner_available
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        let contract = interface_or_adapter_contract.into();
        let opaque_capability_id = opaque_capability_id.into();
        if contract.is_empty() || opaque_capability_id.is_empty() {
            return Err(CallbackCapabilityError::InvalidCapabilityRegistration);
        }
        let key = CallbackCapabilityKey {
            request_generation: request.generation(),
            opaque_capability_id: opaque_capability_id.clone(),
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        if entries.contains_key(&key) {
            return Err(CallbackCapabilityError::DuplicateCapability);
        }
        entries.insert(
            key,
            CallbackCapabilityEntry {
                contract: contract.clone(),
                payload,
                lifecycle: RequestLifecycle::weak(request.lifecycle()),
                lifetime,
                state: CallbackCapabilityEntryState::Active,
            },
        );
        Ok(CallbackCapabilityCarrier::new(
            &self.owner_runtime_replica_id,
            &self.owner_activation_id,
            request.generation(),
            contract,
            opaque_capability_id,
        ))
    }

    pub fn lookup(
        &self,
        carrier: &CallbackCapabilityCarrier,
    ) -> Result<CallbackCapabilityPayload, CallbackCapabilityError> {
        if carrier.owner_runtime_replica_id() != self.owner_runtime_replica_id
            || carrier.owner_activation_id() != self.owner_activation_id
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        if !*self
            .owner_available
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        let key = CallbackCapabilityKey {
            request_generation: carrier.request_generation(),
            opaque_capability_id: carrier.opaque_capability_id().to_string(),
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        let entry = entries
            .get_mut(&key)
            .ok_or(CallbackCapabilityError::CapabilityUnavailable)?;
        if entry.contract != carrier.interface_or_adapter_contract() {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        if entry.state == CallbackCapabilityEntryState::Expired {
            return Err(CallbackCapabilityError::CapabilityExpired);
        }
        let active = entry
            .lifecycle
            .upgrade()
            .is_some_and(|lifecycle| lifecycle.capability_is_active(entry.lifetime));
        if !active {
            entry.state = CallbackCapabilityEntryState::Expired;
            return Err(CallbackCapabilityError::CapabilityExpired);
        }
        Ok(Arc::clone(&entry.payload))
    }

    pub fn expire(
        &self,
        request_generation: u64,
        opaque_capability_id: &str,
    ) -> Result<(), CallbackCapabilityError> {
        let key = CallbackCapabilityKey {
            request_generation,
            opaque_capability_id: opaque_capability_id.to_string(),
        };
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        let entry = entries
            .get_mut(&key)
            .ok_or(CallbackCapabilityError::CapabilityUnavailable)?;
        entry.state = CallbackCapabilityEntryState::Expired;
        Ok(())
    }

    pub fn mark_owner_unavailable(&self) {
        if let Ok(mut owner_available) = self.owner_available.lock() {
            *owner_available = false;
        }
    }

    pub fn active_entry_count(&self) -> usize {
        if !self
            .owner_available
            .lock()
            .map(|owner| *owner)
            .unwrap_or(false)
        {
            return 0;
        }
        self.entries.lock().map_or(0, |mut entries| {
            for entry in entries.values_mut() {
                if entry.state == CallbackCapabilityEntryState::Active
                    && !entry
                        .lifecycle
                        .upgrade()
                        .is_some_and(|lifecycle| lifecycle.capability_is_active(entry.lifetime))
                {
                    entry.state = CallbackCapabilityEntryState::Expired;
                }
            }
            entries
                .values()
                .filter(|entry| entry.state == CallbackCapabilityEntryState::Active)
                .count()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CallbackCapabilityError {
    #[error("CapabilityExpired")]
    CapabilityExpired,
    #[error("CapabilityUnavailable")]
    CapabilityUnavailable,
    #[error("callback capability id is already registered for this request generation")]
    DuplicateCapability,
    #[error("callback capability contract and opaque id must be non-empty")]
    InvalidCapabilityRegistration,
}

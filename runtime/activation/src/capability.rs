use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, Weak},
};

use skiff_runtime_model::value::CallbackCapabilityCarrier;

use crate::{
    request_context::{CallbackLifetime, RequestLifecycle},
    ActivationContext, RequestActivationContext,
};

pub type CallbackCapabilityPayload = Arc<dyn Any + Send + Sync>;

/// Tombstones are retained only to keep recently expired opaque routes terminal.
/// The per-activation bound prevents unbounded growth across request generations.
pub const CALLBACK_CAPABILITY_TOMBSTONE_LIMIT: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallbackCapabilityKey {
    request_generation: u64,
    opaque_capability_id: String,
}

struct ActiveCallbackCapabilityEntry {
    contract: String,
    payload: CallbackCapabilityPayload,
    lifecycle: Weak<RequestLifecycle>,
    lifetime: CallbackLifetime,
}

#[derive(Debug)]
struct CallbackCapabilityTombstone {
    owner_activation_id: String,
    request_generation: u64,
    opaque_capability_id: String,
    contract: String,
}

#[derive(Default)]
struct CallbackCapabilityEntries {
    owner_available: bool,
    active: HashMap<CallbackCapabilityKey, ActiveCallbackCapabilityEntry>,
    tombstones: HashMap<CallbackCapabilityKey, CallbackCapabilityTombstone>,
    tombstone_order: VecDeque<CallbackCapabilityKey>,
}

pub(crate) struct CallbackCapabilityTableState {
    owner_runtime_replica_id: String,
    owner_activation_id: String,
    entries: Mutex<CallbackCapabilityEntries>,
}

#[derive(Clone)]
pub struct CallbackCapabilityTable {
    state: Arc<CallbackCapabilityTableState>,
}

impl std::fmt::Debug for CallbackCapabilityTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CallbackCapabilityTable")
            .field(
                "owner_runtime_replica_id",
                &self.state.owner_runtime_replica_id,
            )
            .field("owner_activation_id", &self.state.owner_activation_id)
            .finish_non_exhaustive()
    }
}

impl CallbackCapabilityTable {
    pub(crate) fn new(owner_runtime_replica_id: String, owner_activation_id: String) -> Self {
        Self {
            state: Arc::new(CallbackCapabilityTableState {
                owner_runtime_replica_id,
                owner_activation_id,
                entries: Mutex::new(CallbackCapabilityEntries {
                    owner_available: true,
                    ..CallbackCapabilityEntries::default()
                }),
            }),
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
            || owner.activation_id().as_str() != self.state.owner_activation_id
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        if !request.lifecycle().capability_is_active(lifetime) {
            return Err(CallbackCapabilityError::CapabilityExpired);
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
        {
            let mut entries = self
                .state
                .entries
                .lock()
                .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
            if !entries.owner_available {
                return Err(CallbackCapabilityError::CapabilityUnavailable);
            }
            if entries.active.contains_key(&key) || entries.tombstones.contains_key(&key) {
                return Err(CallbackCapabilityError::DuplicateCapability);
            }
            entries.active.insert(
                key.clone(),
                ActiveCallbackCapabilityEntry {
                    contract: contract.clone(),
                    payload,
                    lifecycle: RequestLifecycle::weak(request.lifecycle()),
                    lifetime,
                },
            );
        }

        request.lifecycle().register_capability_table(&self.state);

        // Close the insert/terminal race: a terminal event before the weak table
        // registration is observed here and transitions the new entry immediately.
        if !request.lifecycle().capability_is_active(lifetime) {
            self.state.expire_key(&key)?;
            return Err(CallbackCapabilityError::CapabilityExpired);
        }
        if !self.state.owner_is_available()? {
            self.state.expire_key(&key)?;
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }

        Ok(CallbackCapabilityCarrier::new(
            &self.state.owner_runtime_replica_id,
            &self.state.owner_activation_id,
            request.generation(),
            contract,
            opaque_capability_id,
        ))
    }

    pub fn lookup(
        &self,
        carrier: &CallbackCapabilityCarrier,
    ) -> Result<CallbackCapabilityPayload, CallbackCapabilityError> {
        if carrier.owner_runtime_replica_id() != self.state.owner_runtime_replica_id
            || carrier.owner_activation_id() != self.state.owner_activation_id
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        let key = CallbackCapabilityKey {
            request_generation: carrier.request_generation(),
            opaque_capability_id: carrier.opaque_capability_id().to_string(),
        };
        let mut entries = self
            .state
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        if !entries.owner_available {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }

        if let Some(entry) = entries.active.get(&key) {
            if entry.contract != carrier.interface_or_adapter_contract() {
                return Err(CallbackCapabilityError::CapabilityUnavailable);
            }
            let active = entry
                .lifecycle
                .upgrade()
                .is_some_and(|lifecycle| lifecycle.capability_is_active(entry.lifetime));
            if active {
                return Ok(Arc::clone(&entry.payload));
            }
            let expired =
                move_active_to_tombstone(&mut entries, &key, &self.state.owner_activation_id);
            drop(entries);
            drop(expired);
            return Err(CallbackCapabilityError::CapabilityExpired);
        }

        if entries.tombstones.get(&key).is_some_and(|tombstone| {
            tombstone.owner_activation_id == carrier.owner_activation_id()
                && tombstone.request_generation == carrier.request_generation()
                && tombstone.opaque_capability_id == carrier.opaque_capability_id()
                && tombstone.contract == carrier.interface_or_adapter_contract()
        }) {
            return Err(CallbackCapabilityError::CapabilityExpired);
        }
        Err(CallbackCapabilityError::CapabilityUnavailable)
    }

    /// Explicitly expires one registration. Repeated expiry is idempotent while
    /// its bounded tombstone remains present.
    pub fn expire(
        &self,
        request_generation: u64,
        opaque_capability_id: &str,
    ) -> Result<(), CallbackCapabilityError> {
        self.state.expire_key(&CallbackCapabilityKey {
            request_generation,
            opaque_capability_id: opaque_capability_id.to_string(),
        })
    }

    /// Revokes a projected carrier after an enclosing materialization fails.
    /// It never reconstructs a payload and is safe to call more than once.
    pub fn revoke(
        &self,
        carrier: &CallbackCapabilityCarrier,
    ) -> Result<(), CallbackCapabilityError> {
        if carrier.owner_runtime_replica_id() != self.state.owner_runtime_replica_id
            || carrier.owner_activation_id() != self.state.owner_activation_id
        {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        let key = CallbackCapabilityKey {
            request_generation: carrier.request_generation(),
            opaque_capability_id: carrier.opaque_capability_id().to_string(),
        };
        let entries = self
            .state
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        let contract_matches = entries
            .active
            .get(&key)
            .map(|entry| entry.contract.as_str())
            .or_else(|| {
                entries
                    .tombstones
                    .get(&key)
                    .map(|tombstone| tombstone.contract.as_str())
            })
            .is_some_and(|contract| contract == carrier.interface_or_adapter_contract());
        drop(entries);
        if !contract_matches {
            return Err(CallbackCapabilityError::CapabilityUnavailable);
        }
        self.state.expire_key(&key)
    }

    pub fn mark_owner_unavailable(&self) {
        self.state.mark_owner_unavailable();
    }

    pub fn active_entry_count(&self) -> usize {
        self.state.sweep_inactive();
        self.state
            .entries
            .lock()
            .map(|entries| entries.active.len())
            .unwrap_or(0)
    }

    pub fn tombstone_count(&self) -> usize {
        self.state
            .entries
            .lock()
            .map(|entries| entries.tombstones.len())
            .unwrap_or(0)
    }
}

impl CallbackCapabilityTableState {
    fn owner_is_available(&self) -> Result<bool, CallbackCapabilityError> {
        self.entries
            .lock()
            .map(|entries| entries.owner_available)
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)
    }

    fn expire_key(&self, key: &CallbackCapabilityKey) -> Result<(), CallbackCapabilityError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| CallbackCapabilityError::CapabilityUnavailable)?;
        if entries.tombstones.contains_key(key) {
            return Ok(());
        }
        let expired = move_active_to_tombstone(&mut entries, key, &self.owner_activation_id)
            .ok_or(CallbackCapabilityError::CapabilityUnavailable)?;
        drop(entries);
        drop(expired);
        Ok(())
    }

    pub(crate) fn drain_request_generation(&self, request_generation: u64) {
        self.drain_matching(request_generation, |_| true);
    }

    pub(crate) fn drain_request_lifetime(&self, request_generation: u64) {
        self.drain_matching(request_generation, |lifetime| {
            lifetime == CallbackLifetime::Request
        });
    }

    pub(crate) fn drain_stream_lifetime(&self, request_generation: u64) {
        self.drain_matching(request_generation, |lifetime| {
            lifetime == CallbackLifetime::Stream
        });
    }

    fn drain_matching(
        &self,
        request_generation: u64,
        matches_lifetime: impl Fn(CallbackLifetime) -> bool,
    ) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let keys = entries
            .active
            .iter()
            .filter(|(key, entry)| {
                key.request_generation == request_generation && matches_lifetime(entry.lifetime)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let expired = keys
            .iter()
            .filter_map(|key| {
                move_active_to_tombstone(&mut entries, key, &self.owner_activation_id)
            })
            .collect::<Vec<_>>();
        drop(entries);
        drop(expired);
    }

    fn mark_owner_unavailable(&self) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        entries.owner_available = false;
        let keys = entries.active.keys().cloned().collect::<Vec<_>>();
        let expired = keys
            .iter()
            .filter_map(|key| {
                move_active_to_tombstone(&mut entries, key, &self.owner_activation_id)
            })
            .collect::<Vec<_>>();
        drop(entries);
        drop(expired);
    }

    fn sweep_inactive(&self) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let keys = entries
            .active
            .iter()
            .filter(|(_, entry)| {
                !entry
                    .lifecycle
                    .upgrade()
                    .is_some_and(|lifecycle| lifecycle.capability_is_active(entry.lifetime))
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let expired = keys
            .iter()
            .filter_map(|key| {
                move_active_to_tombstone(&mut entries, key, &self.owner_activation_id)
            })
            .collect::<Vec<_>>();
        drop(entries);
        drop(expired);
    }
}

fn move_active_to_tombstone(
    entries: &mut CallbackCapabilityEntries,
    key: &CallbackCapabilityKey,
    owner_activation_id: &str,
) -> Option<ActiveCallbackCapabilityEntry> {
    let entry = entries.active.remove(key)?;
    if entries.tombstones.len() == CALLBACK_CAPABILITY_TOMBSTONE_LIMIT {
        if let Some(expired_key) = entries.tombstone_order.pop_front() {
            entries.tombstones.remove(&expired_key);
        }
    }
    entries.tombstone_order.push_back(key.clone());
    entries.tombstones.insert(
        key.clone(),
        CallbackCapabilityTombstone {
            owner_activation_id: owner_activation_id.to_string(),
            request_generation: key.request_generation,
            opaque_capability_id: key.opaque_capability_id.clone(),
            contract: entry.contract.clone(),
        },
    );
    Some(entry)
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

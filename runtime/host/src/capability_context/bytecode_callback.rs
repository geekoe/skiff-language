//! C6 same-Runtime callback capability table and host projection hooks.
//!
//! The table is the only authority that can turn an opaque
//! [`CallbackCapabilityCarrier`] back into an in-process callback adapter.
//! Cross-Runtime and expired carriers fail closed here; no Router reverse
//! transport is implemented.

use std::{
    any::Any,
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use skiff_artifact_model::PackageSchemaTypeRef;
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableCapabilityHooks, ServiceLinkableCapabilityProjection,
    ServiceLinkableCapabilityRequest, ServiceLinkableMaterializationError,
};
use skiff_runtime_model::{
    callback_projection::{CallbackInvocationState, CallbackLifetime},
    request_heap::RequestHeap,
    runtime_value::{CallbackCapabilityCarrier, InterfaceValue, RuntimeValue},
};
use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

const CALLBACK_TOMBSTONE_LIMIT: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BytecodeCallbackError {
    #[error("callback capability is unavailable")]
    CapabilityUnavailable,
    #[error("callback capability is expired or cancelled")]
    CapabilityExpired,
    #[error("callback capability was cancelled")]
    Cancelled,
    #[error("callback capability owner runtime {actual} does not match this runtime {expected}")]
    CrossRuntimeRejected { expected: String, actual: String },
    #[error("callback capability owner activation {actual} does not match {expected}")]
    WrongOwner { expected: String, actual: String },
    #[error("callback capability contract does not match")]
    WrongContract,
    #[error("callback capability id is already registered for this request generation")]
    DuplicateCapability,
    #[error("callback capability contract and opaque id must be non-empty")]
    InvalidRegistration,
    #[error("callback capability table owner identity is not configured")]
    MissingOwnerIdentity,
}

pub(crate) type CallbackCapabilityPayload = Arc<dyn Any + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CallbackCapabilityKey {
    request_generation: u64,
    opaque_capability_id: String,
}

struct ActiveCallbackCapabilityEntry {
    owner_activation_id: String,
    contract: String,
    payload: CallbackCapabilityPayload,
    state: CallbackInvocationState,
}

#[derive(Debug, Clone)]
struct CallbackCapabilityTombstone {
    owner_activation_id: String,
    request_generation: u64,
    opaque_capability_id: String,
    contract: String,
}

#[derive(Default)]
struct CallbackCapabilityEntries {
    active: HashMap<CallbackCapabilityKey, ActiveCallbackCapabilityEntry>,
    tombstones: HashMap<CallbackCapabilityKey, CallbackCapabilityTombstone>,
    tombstone_order: VecDeque<CallbackCapabilityKey>,
}

#[derive(Clone)]
pub struct BytecodeCallbackCapabilityTable {
    runtime_replica_id: String,
    owner_activation_id: String,
    state: Arc<Mutex<CallbackCapabilityEntries>>,
}

impl BytecodeCallbackCapabilityTable {
    pub fn new(
        runtime_replica_id: impl Into<String>,
        owner_activation_id: impl Into<String>,
    ) -> Self {
        Self {
            runtime_replica_id: runtime_replica_id.into(),
            owner_activation_id: owner_activation_id.into(),
            state: Arc::new(Mutex::new(CallbackCapabilityEntries::default())),
        }
    }

    pub fn runtime_replica_id(&self) -> &str {
        &self.runtime_replica_id
    }

    pub fn owner_activation_id(&self) -> &str {
        &self.owner_activation_id
    }

    pub fn register(
        &self,
        request_generation: u64,
        lifetime: CallbackLifetime,
        contract: impl Into<String>,
        opaque_capability_id: impl Into<String>,
        payload: CallbackCapabilityPayload,
    ) -> Result<CallbackCapabilityCarrier, BytecodeCallbackError> {
        let contract = contract.into();
        let opaque_capability_id = opaque_capability_id.into();
        if self.runtime_replica_id.is_empty()
            || self.owner_activation_id.is_empty()
            || contract.is_empty()
            || opaque_capability_id.is_empty()
        {
            return Err(BytecodeCallbackError::InvalidRegistration);
        }
        let key = CallbackCapabilityKey {
            request_generation,
            opaque_capability_id: opaque_capability_id.clone(),
        };
        {
            let mut entries = self
                .state
                .lock()
                .map_err(|_| BytecodeCallbackError::CapabilityUnavailable)?;
            if entries.active.contains_key(&key) || entries.tombstones.contains_key(&key) {
                return Err(BytecodeCallbackError::DuplicateCapability);
            }
            entries.active.insert(
                key,
                ActiveCallbackCapabilityEntry {
                    owner_activation_id: self.owner_activation_id.clone(),
                    contract: contract.clone(),
                    payload,
                    state: CallbackInvocationState::new(request_generation, lifetime),
                },
            );
        }
        Ok(CallbackCapabilityCarrier::new(
            &self.runtime_replica_id,
            &self.owner_activation_id,
            request_generation,
            contract,
            opaque_capability_id,
        ))
    }

    pub fn lookup(
        &self,
        carrier: &CallbackCapabilityCarrier,
    ) -> Result<CallbackCapabilityPayload, BytecodeCallbackError> {
        if carrier.owner_runtime_replica_id() != self.runtime_replica_id {
            return Err(BytecodeCallbackError::CrossRuntimeRejected {
                expected: self.runtime_replica_id.clone(),
                actual: carrier.owner_runtime_replica_id().to_string(),
            });
        }
        if carrier.owner_activation_id() != self.owner_activation_id {
            return Err(BytecodeCallbackError::WrongOwner {
                expected: self.owner_activation_id.clone(),
                actual: carrier.owner_activation_id().to_string(),
            });
        }
        let key = CallbackCapabilityKey {
            request_generation: carrier.request_generation(),
            opaque_capability_id: carrier.opaque_capability_id().to_string(),
        };
        let mut entries = self
            .state
            .lock()
            .map_err(|_| BytecodeCallbackError::CapabilityUnavailable)?;
        if let Some(entry) = entries.active.get(&key) {
            if entry.contract != carrier.interface_or_adapter_contract()
                || entry.owner_activation_id != self.owner_activation_id
            {
                return Err(BytecodeCallbackError::WrongContract);
            }
            if entry.state.is_active() {
                return Ok(Arc::clone(&entry.payload));
            }
            let expired = move_active_to_tombstone(&mut entries, &key, &self.owner_activation_id);
            drop(expired);
            return Err(BytecodeCallbackError::CapabilityExpired);
        }
        if entries.tombstones.get(&key).is_some_and(|tombstone| {
            tombstone.owner_activation_id == carrier.owner_activation_id()
                && tombstone.request_generation == carrier.request_generation()
                && tombstone.opaque_capability_id == carrier.opaque_capability_id()
                && tombstone.contract == carrier.interface_or_adapter_contract()
        }) {
            return Err(BytecodeCallbackError::CapabilityExpired);
        }
        Err(BytecodeCallbackError::CapabilityUnavailable)
    }

    pub fn cancel(&self, carrier: &CallbackCapabilityCarrier) -> Result<(), BytecodeCallbackError> {
        self.expire_with_terminal(carrier, BytecodeCallbackError::Cancelled)
    }

    pub fn expire(&self, carrier: &CallbackCapabilityCarrier) -> Result<(), BytecodeCallbackError> {
        self.expire_with_terminal(carrier, BytecodeCallbackError::CapabilityExpired)
    }

    fn expire_with_terminal(
        &self,
        carrier: &CallbackCapabilityCarrier,
        terminal: BytecodeCallbackError,
    ) -> Result<(), BytecodeCallbackError> {
        if carrier.owner_runtime_replica_id() != self.runtime_replica_id {
            return Err(BytecodeCallbackError::CrossRuntimeRejected {
                expected: self.runtime_replica_id.clone(),
                actual: carrier.owner_runtime_replica_id().to_string(),
            });
        }
        let key = CallbackCapabilityKey {
            request_generation: carrier.request_generation(),
            opaque_capability_id: carrier.opaque_capability_id().to_string(),
        };
        let mut entries = self
            .state
            .lock()
            .map_err(|_| BytecodeCallbackError::CapabilityUnavailable)?;
        let Some(entry) = entries.active.get_mut(&key) else {
            return Err(terminal);
        };
        if entry.contract != carrier.interface_or_adapter_contract() {
            return Err(BytecodeCallbackError::WrongContract);
        }
        entry.state.expire();
        let expired = move_active_to_tombstone(&mut entries, &key, &self.owner_activation_id);
        drop(expired);
        Ok(())
    }

    pub fn expire_generation(&self, request_generation: u64) {
        self.expire_matching(request_generation, |_| true);
    }

    pub fn expire_lifetime(&self, request_generation: u64, lifetime: CallbackLifetime) {
        self.expire_matching(request_generation, |state| state.lifetime() == lifetime);
    }

    fn expire_matching(
        &self,
        request_generation: u64,
        matches_lifetime: impl Fn(&CallbackInvocationState) -> bool,
    ) {
        let Ok(mut entries) = self.state.lock() else {
            return;
        };
        let keys = entries
            .active
            .iter()
            .filter(|(key, entry)| {
                key.request_generation == request_generation
                    && entry.state.is_active()
                    && matches_lifetime(&entry.state)
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(entry) = entries.active.get_mut(&key) {
                entry.state.expire();
            }
            let expired = move_active_to_tombstone(&mut entries, &key, &self.owner_activation_id);
            drop(expired);
        }
    }

    pub fn active_count(&self) -> usize {
        self.state
            .lock()
            .map(|entries| entries.active.len())
            .unwrap_or(0)
    }

    pub fn tombstone_count(&self) -> usize {
        self.state
            .lock()
            .map(|entries| entries.tombstones.len())
            .unwrap_or(0)
    }
}

fn move_active_to_tombstone(
    entries: &mut CallbackCapabilityEntries,
    key: &CallbackCapabilityKey,
    owner_activation_id: &str,
) -> Option<ActiveCallbackCapabilityEntry> {
    let entry = entries.active.remove(key)?;
    if entries.tombstones.len() == CALLBACK_TOMBSTONE_LIMIT {
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

/// Projection hooks that turn a source-local interface into an opaque
/// same-Runtime callback capability.
#[derive(Clone)]
pub struct BytecodeCallbackCapabilityHooks {
    table: BytecodeCallbackCapabilityTable,
    request_generation: u64,
    next_opaque_id: Arc<std::sync::atomic::AtomicU64>,
}

impl BytecodeCallbackCapabilityHooks {
    pub fn new(table: BytecodeCallbackCapabilityTable, request_generation: u64) -> Self {
        Self {
            table,
            request_generation,
            next_opaque_id: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    pub fn table(&self) -> &BytecodeCallbackCapabilityTable {
        &self.table
    }

    /// Registers an exact host-owned callback execution payload and returns a
    /// rollback-protected projection for the destination boundary allocation.
    pub fn register_payload(
        &self,
        lifetime: CallbackLifetime,
        contract: impl Into<String>,
        receiver_interface_abi_id: impl Into<String>,
        payload: CallbackCapabilityPayload,
    ) -> Result<ServiceLinkableCapabilityProjection, BytecodeCallbackError> {
        let contract = contract.into();
        let receiver_interface_abi_id = receiver_interface_abi_id.into();
        if contract.is_empty() || receiver_interface_abi_id.is_empty() {
            return Err(BytecodeCallbackError::InvalidRegistration);
        }
        let opaque_id = format!(
            "callback:{}:{}",
            self.request_generation,
            self.next_opaque_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let carrier = self.table.register(
            self.request_generation,
            lifetime,
            contract,
            opaque_id,
            payload,
        )?;
        let rollback_carrier = carrier.clone();
        let table = self.table.clone();
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                carrier,
                receiver_interface_abi_id,
                move || {
                    let _ = table.expire(&rollback_carrier);
                },
            ),
        )
    }

    fn project(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
        native: bool,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        let lifetime = CallbackLifetime::from_boundary(request.lifetime).map_err(|error| {
            ServiceLinkableMaterializationError::InvalidContractPlan {
                message: error.to_string(),
            }
        })?;
        let interface = callback_interface_value(request.value, request.source_heap)?;
        let adapter = if native {
            let package_schema = package_schema_type(request.ty)?;
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                request.ty,
                package_schema,
                &callback_operations(request.ty, request.package_schema_records)?,
                interface,
                request.package_schema_records,
                request.source_heap,
            )
        } else {
            let package_schema = package_schema_type(request.ty)?;
            InProcessCallbackAdapter::from_local_interface(
                package_schema,
                interface,
                &callback_operations(request.ty, request.package_schema_records)?,
                request.package_schema_records,
                request.source_heap,
            )
        }
        .map_err(callback_adapter_error)?;
        let canonical = adapter
            .canonical_contract_identity()
            .map_err(callback_adapter_error)?;
        let opaque_id = format!(
            "callback:{}:{}",
            self.request_generation,
            self.next_opaque_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let carrier = self
            .table
            .register(
                self.request_generation,
                lifetime,
                canonical,
                opaque_id,
                Arc::new(adapter),
            )
            .map_err(capability_error)?;
        let rollback_carrier = carrier.clone();
        let table = self.table.clone();
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                carrier,
                interface.interface(),
                move || {
                    let _ = table.expire(&rollback_carrier);
                },
            ),
        )
    }
}

impl ServiceLinkableCapabilityHooks for BytecodeCallbackCapabilityHooks {
    fn project_callback_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        self.project(request, false)
    }

    fn project_native_adapter_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError> {
        self.project(request, true)
    }
}

fn callback_interface_value<'a>(
    value: &'a RuntimeValue,
    heap: &'a RequestHeap,
) -> Result<&'a InterfaceValue, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    };
    match heap.get(*handle) {
        Ok(skiff_runtime_model::value::HeapNode::Interface(interface)) => Ok(interface),
        _ => Err(ServiceLinkableMaterializationError::TypeMismatch),
    }
}

fn package_schema_type(
    ty: &skiff_artifact_model::ContractTypeRef,
) -> Result<PackageSchemaTypeRef, ServiceLinkableMaterializationError> {
    let skiff_artifact_model::ContractTypeRef::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = ty
    else {
        return Err(ServiceLinkableMaterializationError::InvalidContractPlan {
            message: "callback hook only receives package schema types".to_string(),
        });
    };
    Ok(PackageSchemaTypeRef {
        package_id: package_id.clone(),
        stable_schema_key: stable_schema_key.clone(),
        package_schema_type_id: package_schema_type_id.clone(),
    })
}

fn callback_operations(
    ty: &skiff_artifact_model::ContractTypeRef,
    records: &skiff_runtime_boundary::package_schema_records::PackageSchemaRecords,
) -> Result<
    std::collections::BTreeMap<String, skiff_artifact_model::BoundaryCallbackOperation>,
    ServiceLinkableMaterializationError,
> {
    let reference = package_schema_type(ty)?;
    let record = records
        .get(&reference.package_schema_type_id)
        .ok_or_else(|| ServiceLinkableMaterializationError::MissingSchema {
            package_schema_type_id: reference.package_schema_type_id.clone(),
        })?;
    let skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { operations } =
        &record.canonical_descriptor.descriptor
    else {
        return Err(ServiceLinkableMaterializationError::InvalidContractPlan {
            message: "package schema type is not a callback interface".to_string(),
        });
    };
    Ok(operations.clone())
}

fn callback_adapter_error(
    error: skiff_runtime_native::callback_adapter::CallbackAdapterError,
) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::RuntimeModel {
        message: error.to_string(),
    }
}

fn capability_error(_error: BytecodeCallbackError) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::InvalidProjectedCapability
}

#[cfg(test)]
mod tests;

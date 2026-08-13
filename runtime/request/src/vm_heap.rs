//! RequestHeap-backed implementation of the narrow VM heap port.
//!
//! The adapter owns the stable `VmHandle` registry and ordinary snapshot share
//! accounting. RequestHeap remains the allocation arena for arrays, objects,
//! maps and representation carrier cells.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::atomic::{AtomicU64, Ordering},
    sync::{Arc, Mutex},
};

use skiff_runtime_model::{
    error::RuntimeModelError as RuntimeError,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapHandle, HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::CatchIdentity,
    vm_heap::{
        VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment, VmMapEntry,
        VmRecordField,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};
use skiff_runtime_scheduler::{
    OwnerCreationError, ResourceOwnerCreationGuard, ResourceOwnerLease, ResourceOwnerRegistration,
};

const DOMAIN_SHIFT: u64 = 56;
const DOMAIN_MASK: u64 = (u8::MAX as u64) << DOMAIN_SHIFT;
const SERIAL_MASK: u64 = !DOMAIN_MASK;
const MAX_SERIAL: u64 = SERIAL_MASK;

static NEXT_DOMAIN: AtomicU64 = AtomicU64::new(1);

fn next_domain() -> u8 {
    NEXT_DOMAIN.fetch_add(1, Ordering::Relaxed) as u8
}

fn encode_handle(domain: u8, serial: u64) -> VmHandle {
    VmHandle::new((u64::from(domain) << DOMAIN_SHIFT) | serial)
}

struct LiveEntry {
    heap_handle: HeapHandle,
    compact_type_tag: CompactTypeTag,
    flags: ValueFlags,
    snapshot_owners: usize,
    owner_transfers: usize,
}

/// Shared registry of native resources and handles released by either the
/// heap or the adapter executor.
struct ResourceRegistry {
    live: HashMap<VmHandle, ResourceEntry>,
    released: HashSet<VmHandle>,
}

impl ResourceRegistry {
    /// Registers a live resource before any VM operation can validate it.
    fn register(&mut self, handle: VmHandle, entry: ResourceEntry) {
        self.released.remove(&handle);
        self.live.insert(handle, entry);
    }

    fn metadata(&self, handle: VmHandle) -> Option<(CompactTypeTag, ValueFlags)> {
        self.live
            .get(&handle)
            .map(|entry| (entry.compact_type_tag, entry.flags))
    }

    pub(crate) fn contains_live(&self, handle: VmHandle) -> bool {
        self.live.contains_key(&handle)
    }

    fn is_released(&self, handle: VmHandle) -> bool {
        self.released.contains(&handle)
    }

    /// Removes a live entry and marks the handle released. A released handle
    /// is idempotent for later VM release, but no longer live.
    fn remove_live(&mut self, handle: VmHandle) -> Option<ResourceEntry> {
        let entry = self.live.remove(&handle)?;
        self.released.insert(handle);
        Some(entry)
    }
}

/// Opaque resource registry permanently bound to one request inventory.
#[derive(Clone)]
pub struct ResourceTable {
    registry: Arc<Mutex<ResourceRegistry>>,
    owners: ResourceOwnerRegistration,
}

impl ResourceTable {
    pub fn new(owners: ResourceOwnerRegistration) -> Self {
        Self {
            registry: Arc::new(Mutex::new(ResourceRegistry {
                live: HashMap::new(),
                released: HashSet::new(),
            })),
            owners,
        }
    }

    pub fn register(
        &self,
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
        cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), RegisterResourceError> {
        let owner = self
            .owners
            .prepare()
            .map_err(RegisterResourceError::OwnerCreation)?;
        self.register_with_guard(owner, handle, compact_type_tag, flags, cancel)
    }

    fn register_with_guard(
        &self,
        owner: ResourceOwnerCreationGuard<'_>,
        handle: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
        cancel: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), RegisterResourceError> {
        // The non-cloneable guard already owns the inventory lock. The table
        // is deliberately inaccessible until that fixed lock order exists.
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry.contains_live(handle) {
            return Err(RegisterResourceError::OccupiedHandle);
        }
        owner
            .install(|owner_lease| {
                registry.register(
                    handle,
                    ResourceEntry::new(compact_type_tag, flags, cancel, owner_lease),
                );
            })
            .map_err(RegisterResourceError::OwnerCreation)
    }

    pub(crate) fn remove_live(&self, handle: VmHandle) -> Option<ResourceEntry> {
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = registry.remove_live(handle);
        drop(registry);
        entry
    }

    #[cfg(test)]
    fn contains_live(&self, handle: VmHandle) -> bool {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains_live(handle)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegisterResourceError {
    OccupiedHandle,
    OwnerCreation(OwnerCreationError),
}

impl std::fmt::Display for RegisterResourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OccupiedHandle => formatter.write_str("resource handle is already occupied"),
            Self::OwnerCreation(error) => error.fmt(formatter),
        }
    }
}

/// An entry in the shared resource table.
pub struct ResourceEntry {
    /// VM slot metadata that must match a live resource reference.
    compact_type_tag: CompactTypeTag,
    flags: ValueFlags,
    /// Cancels the underlying native resource (e.g., HTTP stream).
    cancel: Arc<dyn Fn() + Send + Sync>,
    owner_lease: ResourceOwnerLease,
}

impl ResourceEntry {
    pub(crate) fn new(
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
        cancel: Arc<dyn Fn() + Send + Sync>,
        owner_lease: ResourceOwnerLease,
    ) -> Self {
        Self {
            compact_type_tag,
            flags,
            cancel,
            owner_lease,
        }
    }

    /// Ends the native resource while its inventory lease is still live.
    pub fn cancel(self) {
        let Self {
            cancel,
            owner_lease,
            ..
        } = self;
        (cancel)();
        drop(owner_lease);
    }
}

pub struct RequestVmHeap {
    heap: RequestHeap,
    domain: u8,
    next_serial: u64,
    live: HashMap<VmHandle, LiveEntry>,
    handles_by_heap: HashMap<HeapHandle, VmHandle>,
    released_heap_handles: HashMap<HeapHandle, VmHandle>,
    array_slots: HashMap<HeapHandle, Vec<ValueSlot>>,
    object_slots: HashMap<HeapHandle, BTreeMap<String, ValueSlot>>,
    map_slots: HashMap<
        HeapHandle,
        BTreeMap<skiff_runtime_model::runtime_value::RuntimeValueKey, (ValueSlot, ValueSlot)>,
    >,
    representation_slots: HashMap<HeapHandle, ValueSlot>,
    resource_table: Option<ResourceTable>,
}

impl RequestVmHeap {
    pub fn new(limits: RequestHeapLimits) -> Self {
        Self::with_domain(next_domain(), 0, limits)
    }

    pub fn new_with_epoch(epoch: u32, limits: RequestHeapLimits) -> Self {
        Self::with_domain(next_domain(), epoch, limits)
    }

    pub fn with_domain(domain: u8, epoch: u32, limits: RequestHeapLimits) -> Self {
        Self {
            heap: RequestHeap::new_with_epoch(epoch, limits),
            domain,
            next_serial: 1,
            live: HashMap::new(),
            handles_by_heap: HashMap::new(),
            released_heap_handles: HashMap::new(),
            array_slots: HashMap::new(),
            object_slots: HashMap::new(),
            map_slots: HashMap::new(),
            representation_slots: HashMap::new(),
            resource_table: None,
        }
    }

    pub fn request_heap(&self) -> &RequestHeap {
        &self.heap
    }

    pub fn request_heap_mut(&mut self) -> &mut RequestHeap {
        &mut self.heap
    }

    pub fn epoch(&self) -> u32 {
        self.heap.epoch()
    }

    pub fn limits(&self) -> &RequestHeapLimits {
        self.heap.limits()
    }

    /// Attaches a shared resource table so this heap can validate and release
    /// resource references created by the adapter executor.
    pub fn set_resource_table(&mut self, table: ResourceTable) {
        self.resource_table = Some(table);
    }

    pub fn resource_table(&self) -> Option<&ResourceTable> {
        self.resource_table.as_ref()
    }

    /// Wraps an existing RequestHeap handle in a live VM slot.
    pub fn heap_ref(
        &mut self,
        heap_handle: HeapHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        if let Some(vm_handle) = self.handles_by_heap.get(&heap_handle).copied() {
            let entry = self
                .live
                .get(&vm_handle)
                .ok_or_else(|| Self::stale(heap_handle))?;
            if entry.compact_type_tag != compact_type_tag || entry.flags != flags {
                return Err(VmHeapError::InvalidValueMetadata);
            }
            return Ok(Self::slot_for_entry(vm_handle, entry));
        }
        if let Some(vm_handle) = self.released_heap_handles.get(&heap_handle) {
            return Err(Self::invalid_handle(
                *vm_handle,
                VmHandleInvalidReason::StaleGenerationOrEpoch,
            ));
        }
        self.register_handle(heap_handle, compact_type_tag, flags)
    }

    /// Allocates a RequestHeap local carrier cell containing a string value.
    ///
    /// Strings do not have a fixed-width immediate slot, so the adapter keeps
    /// them as request-local carrier cells. This is also the representation
    /// payload path used by string representation map keys.
    pub fn alloc_string(&mut self, value: impl Into<String>) -> Result<ValueSlot, VmHeapError> {
        let carrier = RuntimeValueCarrier::unidentified(RuntimeValue::String(value.into()));
        self.ensure_serial_available(VmHeapOperation::AllocateRepresentation)?;
        let handle = self
            .heap
            .alloc_local_carrier_cell(carrier)
            .map_err(|error| self.map_error(error, VmHeapOperation::AllocateRepresentation))?;
        self.register_handle(handle, CompactTypeTag::new(0), ValueFlags::new(0))
    }

    fn ensure_serial_available(&self, operation: VmHeapOperation) -> Result<(), VmHeapError> {
        if self.next_serial > MAX_SERIAL {
            return Err(VmHeapError::ResourceLimitExceeded {
                operation,
                limit: usize::try_from(MAX_SERIAL).unwrap_or(usize::MAX),
                current: usize::try_from(self.next_serial).unwrap_or(usize::MAX),
                requested_delta: 1,
            });
        }
        Ok(())
    }

    fn alloc_serial(&mut self, operation: VmHeapOperation) -> Result<u64, VmHeapError> {
        if self.next_serial > MAX_SERIAL {
            return Err(VmHeapError::ResourceLimitExceeded {
                operation,
                limit: usize::try_from(MAX_SERIAL).unwrap_or(usize::MAX),
                current: usize::try_from(self.next_serial).unwrap_or(usize::MAX),
                requested_delta: 1,
            });
        }
        let serial = self.next_serial;
        self.next_serial += 1;
        Ok(serial)
    }

    fn register_handle(
        &mut self,
        heap_handle: HeapHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, VmHeapOperation::ValidateLive))?;
        let serial = self.alloc_serial(VmHeapOperation::ValidateLive)?;
        let vm_handle = encode_handle(self.domain, serial);
        self.live.insert(
            vm_handle,
            LiveEntry {
                heap_handle,
                compact_type_tag,
                flags,
                snapshot_owners: 1,
                owner_transfers: 0,
            },
        );
        self.handles_by_heap.insert(heap_handle, vm_handle);
        self.released_heap_handles.remove(&heap_handle);
        Ok(ValueSlot::request_heap_ref(
            vm_handle,
            compact_type_tag,
            flags,
        ))
    }

    fn slot_for_entry(vm_handle: VmHandle, entry: &LiveEntry) -> ValueSlot {
        ValueSlot::request_heap_ref(vm_handle, entry.compact_type_tag, entry.flags)
    }

    fn stale(heap_handle: HeapHandle) -> VmHeapError {
        VmHeapError::InvalidHandle {
            kind: ValueKind::RequestHeapRef,
            handle: encode_handle(0, u64::from(heap_handle.index())),
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
        }
    }

    fn stale_resource(handle: VmHandle) -> VmHeapError {
        VmHeapError::InvalidHandle {
            kind: ValueKind::ResourceRef,
            handle,
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
        }
    }

    fn live_resource(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        let handle = value
            .as_resource_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        let table = self
            .resource_table
            .as_ref()
            .ok_or(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ValidateLive,
                kind: ValueKind::ResourceRef,
            })?;
        let guard = table
            .registry
            .lock()
            .map_err(|_| VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ValidateLive,
                message: "resource table lock poisoned".to_string(),
            })?;
        match guard.metadata(handle) {
            Some((compact_type_tag, flags))
                if compact_type_tag == value.compact_type_tag() && flags == value.flags() =>
            {
                Ok(())
            }
            Some(_) => Err(VmHeapError::InvalidValueMetadata),
            None => Err(Self::stale_resource(handle)),
        }
    }

    fn release_resource_inner(&self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        let handle = owner
            .as_resource_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        let table = self
            .resource_table
            .as_ref()
            .ok_or(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseResource,
                kind: ValueKind::ResourceRef,
            })?;
        let mut guard = table
            .registry
            .lock()
            .map_err(|_| VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseResource,
                message: "resource table lock poisoned".to_string(),
            })?;
        if guard.is_released(handle) {
            return Ok(());
        }
        if let Some((compact_type_tag, flags)) = guard.metadata(handle) {
            if compact_type_tag != owner.compact_type_tag() || flags != owner.flags() {
                return Err(VmHeapError::InvalidValueMetadata);
            }
        }
        let entry = guard
            .remove_live(handle)
            .ok_or_else(|| Self::stale_resource(handle))?;
        drop(guard);
        entry.cancel();
        Ok(())
    }

    fn domain_of(handle: VmHandle) -> u8 {
        ((handle.get() & DOMAIN_MASK) >> DOMAIN_SHIFT) as u8
    }

    fn invalid_handle(handle: VmHandle, reason: VmHandleInvalidReason) -> VmHeapError {
        VmHeapError::InvalidHandle {
            kind: ValueKind::RequestHeapRef,
            handle,
            reason,
        }
    }

    fn live_entry(&self, value: &ValueSlot) -> Result<&LiveEntry, VmHeapError> {
        let handle = value
            .as_request_heap_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        if Self::domain_of(handle) != self.domain {
            return Err(Self::invalid_handle(
                handle,
                VmHandleInvalidReason::WrongDomain,
            ));
        }
        let entry = self.live.get(&handle).ok_or_else(|| {
            Self::invalid_handle(handle, VmHandleInvalidReason::StaleGenerationOrEpoch)
        })?;
        if entry.compact_type_tag != value.compact_type_tag() || entry.flags != value.flags() {
            return Err(VmHeapError::InvalidValueMetadata);
        }
        self.heap.get(entry.heap_handle).map_err(|_| {
            Self::invalid_handle(handle, VmHandleInvalidReason::StaleGenerationOrEpoch)
        })?;
        Ok(entry)
    }

    fn live_entry_mut(&mut self, value: &ValueSlot) -> Result<&mut LiveEntry, VmHeapError> {
        let handle = value
            .as_request_heap_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        if Self::domain_of(handle) != self.domain {
            return Err(Self::invalid_handle(
                handle,
                VmHandleInvalidReason::WrongDomain,
            ));
        }
        let entry = self.live.get_mut(&handle).ok_or_else(|| {
            Self::invalid_handle(handle, VmHandleInvalidReason::StaleGenerationOrEpoch)
        })?;
        if entry.compact_type_tag != value.compact_type_tag() || entry.flags != value.flags() {
            return Err(VmHeapError::InvalidValueMetadata);
        }
        self.heap.get(entry.heap_handle).map_err(|_| {
            Self::invalid_handle(handle, VmHandleInvalidReason::StaleGenerationOrEpoch)
        })?;
        Ok(entry)
    }

    fn request_handle(
        &self,
        value: &ValueSlot,
        operation: VmHeapOperation,
    ) -> Result<HeapHandle, VmHeapError> {
        let Some(kind) = value.kind() else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        if kind != ValueKind::RequestHeapRef {
            return Err(VmHeapError::OperationKindMismatch { operation, kind });
        }
        Ok(self.live_entry(value)?.heap_handle)
    }

    fn map_error(&self, error: RuntimeError, operation: VmHeapOperation) -> VmHeapError {
        match error {
            RuntimeError::ResourceLimitExceeded {
                limit,
                current,
                requested_delta,
                ..
            } => VmHeapError::ResourceLimitExceeded {
                operation,
                limit,
                current,
                requested_delta,
            },
            RuntimeError::Decode(message) => {
                VmHeapError::HeapOperationFailed { operation, message }
            }
            RuntimeError::Json(error) => VmHeapError::HeapOperationFailed {
                operation,
                message: error.to_string(),
            },
        }
    }

    fn ensure_node_kind(
        &self,
        heap_handle: HeapHandle,
        operation: VmHeapOperation,
        expected: impl Fn(&HeapNode) -> bool,
    ) -> Result<(), VmHeapError> {
        let node = self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?;
        if expected(node) {
            Ok(())
        } else {
            Err(VmHeapError::OperationKindMismatch {
                operation,
                kind: ValueKind::RequestHeapRef,
            })
        }
    }

    fn runtime_carrier_for_slot(
        &self,
        value: &ValueSlot,
        operation: VmHeapOperation,
    ) -> Result<RuntimeValueCarrier, VmHeapError> {
        self.validate_live(value)?;
        let runtime_value = match value.kind() {
            Some(ValueKind::Null) => RuntimeValue::Null,
            Some(ValueKind::Bool) => {
                RuntimeValue::Bool(value.as_bool().ok_or(VmHeapError::InvalidValueMetadata)?)
            }
            Some(ValueKind::Number) => {
                RuntimeValue::Number(value.as_number().ok_or(VmHeapError::InvalidValueMetadata)?)
            }
            Some(ValueKind::Date) => {
                RuntimeValue::Date(value.as_date().ok_or(VmHeapError::InvalidValueMetadata)?)
            }
            Some(ValueKind::Integer) => RuntimeValue::Null,
            Some(ValueKind::RequestHeapRef) => {
                RuntimeValue::Heap(self.live_entry(value)?.heap_handle)
            }
            Some(ValueKind::ResourceRef) => {
                // ResourceRefs are not representable as RequestHeap carriers.
                // The sidecar slot maps keep the exact slot; this JSON-safe
                // placeholder prevents serialization from exposing the handle.
                RuntimeValue::Null
            }
            Some(kind) => return Err(VmHeapError::OperationKindMismatch { operation, kind }),
            None => return Err(VmHeapError::InvalidValueMetadata),
        };
        Ok(RuntimeValueCarrier::unidentified(runtime_value))
    }

    fn slot_from_carrier(
        &self,
        carrier: &RuntimeValueCarrier,
        operation: VmHeapOperation,
    ) -> Result<ValueSlot, VmHeapError> {
        match carrier.value() {
            RuntimeValue::Null => Ok(ValueSlot::null()),
            RuntimeValue::Bool(value) => Ok(ValueSlot::bool(*value)),
            RuntimeValue::Number(value) => Ok(ValueSlot::number(*value)),
            RuntimeValue::Date(value) => Ok(ValueSlot::date(*value)),
            RuntimeValue::Heap(handle) => {
                let vm_handle = self.handles_by_heap.get(handle).copied().ok_or_else(|| {
                    VmHeapError::HeapOperationFailed {
                        operation,
                        message: "heap child has no live VM handle".to_string(),
                    }
                })?;
                let entry =
                    self.live
                        .get(&vm_handle)
                        .ok_or_else(|| VmHeapError::HeapOperationFailed {
                            operation,
                            message: "heap child is not live".to_string(),
                        })?;
                Ok(Self::slot_for_entry(vm_handle, entry))
            }
            RuntimeValue::String(_) | RuntimeValue::ActorRef(_) => {
                Err(VmHeapError::HeapOperationFailed {
                    operation,
                    message: "non-fixed-width runtime value cannot be projected as a VM slot"
                        .to_string(),
                })
            }
        }
    }

    fn map_key_from_slot(
        &self,
        key: &ValueSlot,
        operation: VmHeapOperation,
    ) -> Result<skiff_runtime_model::runtime_value::RuntimeValueKey, VmHeapError> {
        self.validate_live(key)?;
        let Some(vm_handle) = key.as_request_heap_ref() else {
            return Err(VmHeapError::HeapOperationFailed {
                operation,
                message: "map key must be a request-local string or string representation"
                    .to_string(),
            });
        };
        let entry = self.live_entry(key)?;
        if let Some(payload) = self.representation_slots.get(&entry.heap_handle) {
            return self.map_key_from_slot(payload, operation);
        }
        let carrier = self
            .heap
            .local_carrier_cell(entry.heap_handle)
            .map_err(|error| self.map_error(error, operation))?;
        match carrier.as_ref() {
            Some(carrier) if matches!(carrier.value(), RuntimeValue::String(_)) => {
                let RuntimeValue::String(value) = carrier.value() else {
                    unreachable!("matched string carrier");
                };
                Ok(skiff_runtime_model::runtime_value::RuntimeValueKey::string(
                    value.clone(),
                ))
            }
            _ => Err(VmHeapError::HeapOperationFailed {
                operation,
                message: format!("{vm_handle:?} is not a string map key"),
            }),
        }
    }

    fn ensure_selector_count(
        &self,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
        operation: VmHeapOperation,
    ) -> Result<(), VmHeapError> {
        let expected = segments
            .iter()
            .filter(|segment| {
                matches!(
                    segment,
                    VmHeapPathSegment::ArrayIndex | VmHeapPathSegment::MapKey
                )
            })
            .count();
        if selectors.len() == expected {
            Ok(())
        } else {
            Err(VmHeapError::HeapOperationFailed {
                operation,
                message: format!(
                    "writable path selector count {expected} does not match {}",
                    selectors.len()
                ),
            })
        }
    }

    fn set_array_element(
        &mut self,
        array: &ValueSlot,
        index: usize,
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        let heap_handle = self.request_handle(array, VmHeapOperation::SetWritablePath)?;
        self.array_get(array, index)?;
        self.ensure_node_kind(heap_handle, VmHeapOperation::SetWritablePath, |node| {
            matches!(node, HeapNode::Array(_))
        })?;
        let carrier = self.runtime_carrier_for_slot(&value, VmHeapOperation::SetWritablePath)?;
        self.heap
            .set_array_item_carrier(heap_handle, index, carrier)
            .map_err(|error| self.map_error(error, VmHeapOperation::SetWritablePath))?;
        if let Some(slots) = self.array_slots.get_mut(&heap_handle) {
            if let Some(slot) = slots.get_mut(index) {
                *slot = value;
            }
        }
        Ok(())
    }

    fn set_record_field(
        &mut self,
        record: &ValueSlot,
        field: &str,
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        let heap_handle = self.request_handle(record, VmHeapOperation::SetWritablePath)?;
        self.record_field(record, field)?;
        let carrier = self.runtime_carrier_for_slot(&value, VmHeapOperation::SetWritablePath)?;
        self.heap
            .set_object_field_carrier(heap_handle, field.to_string(), carrier)
            .map_err(|error| self.map_error(error, VmHeapOperation::SetWritablePath))?;
        self.object_slots
            .entry(heap_handle)
            .or_default()
            .insert(field.to_string(), value);
        Ok(())
    }
}

impl VmHeap for RequestVmHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        let Some(kind) = value.kind() else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        match kind {
            ValueKind::Null => value.as_null().ok_or(VmHeapError::InvalidValueMetadata),
            ValueKind::Bool => value
                .as_bool()
                .map(|_| ())
                .ok_or(VmHeapError::InvalidValueMetadata),
            ValueKind::Number | ValueKind::Integer | ValueKind::Date => Ok(()),
            ValueKind::RequestHeapRef => self.live_entry(value).map(|_| ()),
            ValueKind::ResourceRef => self.live_resource(value),
            kind => Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ValidateLive,
                kind,
            }),
        }
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        match source.kind() {
            Some(
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date,
            ) => {
                self.validate_live(source)?;
                Ok(*source)
            }
            Some(ValueKind::RequestHeapRef) => {
                self.live_entry_mut(source)?.snapshot_owners += 1;
                Ok(*source)
            }
            Some(kind) => Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::SnapshotShare,
                kind,
            }),
            None => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        match source.kind() {
            Some(
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date,
            ) => {
                self.validate_live(source)?;
                Ok(*source)
            }
            Some(ValueKind::RequestHeapRef) => {
                self.live_entry_mut(source)?.owner_transfers += 1;
                Ok(*source)
            }
            Some(ValueKind::ResourceRef) => {
                self.live_resource(source)?;
                Ok(*source)
            }
            Some(kind) => Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::TransferOwner,
                kind,
            }),
            None => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        match owner.kind() {
            Some(
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date,
            ) => Ok(()),
            Some(ValueKind::RequestHeapRef) => {
                let vm_handle = owner
                    .as_request_heap_ref()
                    .ok_or(VmHeapError::InvalidValueMetadata)?;
                let heap_handle = self.live_entry_mut(owner)?.heap_handle;
                let remove = {
                    let entry = self.live.get_mut(&vm_handle).ok_or_else(|| {
                        Self::invalid_handle(
                            vm_handle,
                            VmHandleInvalidReason::StaleGenerationOrEpoch,
                        )
                    })?;
                    entry.snapshot_owners = entry.snapshot_owners.checked_sub(1).ok_or(
                        VmHeapError::OwnershipViolation {
                            kind: ValueKind::RequestHeapRef,
                            handle: vm_handle,
                        },
                    )?;
                    entry.snapshot_owners == 0
                };
                if remove {
                    let entry = self.live.remove(&vm_handle).ok_or_else(|| {
                        Self::invalid_handle(
                            vm_handle,
                            VmHandleInvalidReason::StaleGenerationOrEpoch,
                        )
                    })?;
                    self.handles_by_heap.remove(&entry.heap_handle);
                    self.released_heap_handles
                        .insert(entry.heap_handle, vm_handle);
                    self.array_slots.remove(&heap_handle);
                    self.object_slots.remove(&heap_handle);
                    self.map_slots.remove(&heap_handle);
                    self.representation_slots.remove(&heap_handle);
                }
                Ok(())
            }
            Some(kind) => Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseSnapshot,
                kind,
            }),
            None => Err(VmHeapError::InvalidValueMetadata),
        }
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        let Some(kind) = owner.kind() else {
            return Err(VmHeapError::InvalidValueMetadata);
        };
        if kind == ValueKind::ResourceRef {
            return self.release_resource_inner(owner);
        }
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseResource,
            kind,
        })
    }

    fn allocate_array(
        &mut self,
        elements: &[ValueSlot],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::AllocateArray;
        for element in elements {
            self.validate_live(element)?;
        }
        let mut carriers = Vec::with_capacity(elements.len());
        for element in elements {
            carriers.push(self.runtime_carrier_for_slot(element, operation)?);
        }
        self.ensure_serial_available(operation)?;
        let heap_handle = self
            .heap
            .alloc_array_carriers(carriers)
            .map_err(|error| self.map_error(error, operation))?;
        let slot = self.register_handle(heap_handle, compact_type_tag, flags)?;
        self.array_slots.insert(heap_handle, elements.to_vec());
        Ok(slot)
    }

    fn allocate_map(
        &mut self,
        entries: &[VmMapEntry],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::AllocateMap;
        let mut carriers = BTreeMap::new();
        let mut slots = BTreeMap::new();
        for entry in entries {
            self.validate_live(&entry.key)?;
            self.validate_live(&entry.value)?;
            let key = self.map_key_from_slot(&entry.key, operation)?;
            carriers.insert(
                key.clone(),
                self.runtime_carrier_for_slot(&entry.value, operation)?,
            );
            slots.insert(key, (entry.key, entry.value));
        }
        self.ensure_serial_available(operation)?;
        let heap_handle = self
            .heap
            .alloc_map_carriers(carriers)
            .map_err(|error| self.map_error(error, operation))?;
        let slot = self.register_handle(heap_handle, compact_type_tag, flags)?;
        self.map_slots.insert(heap_handle, slots);
        Ok(slot)
    }

    fn allocate_record(
        &mut self,
        fields: &[VmRecordField],
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::AllocateRecord;
        let mut carriers = BTreeMap::new();
        let mut slots = BTreeMap::new();
        for field in fields {
            if field.name.is_empty() {
                return Err(VmHeapError::HeapOperationFailed {
                    operation,
                    message: "record field name must not be empty".to_string(),
                });
            }
            self.validate_live(&field.value)?;
            if slots.insert(field.name.clone(), field.value).is_some() {
                return Err(VmHeapError::HeapOperationFailed {
                    operation,
                    message: format!("duplicate record field {}", field.name),
                });
            }
            carriers.insert(
                field.name.clone(),
                self.runtime_carrier_for_slot(&field.value, operation)?,
            );
        }
        self.ensure_serial_available(operation)?;
        let heap_handle = self
            .heap
            .alloc_object_carriers(carriers)
            .map_err(|error| self.map_error(error, operation))?;
        let slot = self.register_handle(heap_handle, compact_type_tag, flags)?;
        self.object_slots.insert(heap_handle, slots);
        Ok(slot)
    }

    fn allocate_representation(
        &mut self,
        payload: &ValueSlot,
        identity: CatchIdentity,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::AllocateRepresentation;
        self.validate_live(payload)?;
        let carrier = RuntimeValueCarrier::identified(
            self.runtime_carrier_for_slot(payload, operation)?
                .into_value(),
            identity,
        );
        self.ensure_serial_available(operation)?;
        let heap_handle = self
            .heap
            .alloc_local_carrier_cell(carrier)
            .map_err(|error| self.map_error(error, operation))?;
        let slot = self.register_handle(heap_handle, compact_type_tag, flags)?;
        self.representation_slots.insert(heap_handle, *payload);
        Ok(slot)
    }

    fn alloc_bytes(&mut self, value: Vec<u8>) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::AllocateArray;
        self.ensure_serial_available(operation)?;
        let handle = self
            .heap
            .alloc_bytes(value)
            .map_err(|error| self.map_error(error, operation))?;
        self.register_handle(handle, CompactTypeTag::new(0), ValueFlags::new(0))
    }

    fn alloc_string(&mut self, value: String) -> Result<ValueSlot, VmHeapError> {
        let carrier = RuntimeValueCarrier::unidentified(RuntimeValue::String(value));
        let operation = VmHeapOperation::AllocateRepresentation;
        self.ensure_serial_available(operation)?;
        let heap_handle = self
            .heap
            .alloc_local_carrier_cell(carrier)
            .map_err(|error| self.map_error(error, operation))?;
        self.register_handle(heap_handle, CompactTypeTag::new(0), ValueFlags::new(0))
    }

    fn string_value(&self, value: &ValueSlot) -> Result<String, VmHeapError> {
        let operation = VmHeapOperation::RepresentationPayload;
        let heap_handle = self.request_handle(value, operation)?;
        let carrier = self
            .heap
            .local_carrier_cell(heap_handle)
            .map_err(|error| self.map_error(error, operation))?;
        match carrier.as_ref() {
            Some(carrier) if matches!(carrier.value(), RuntimeValue::String(_)) => {
                let RuntimeValue::String(value) = carrier.value() else {
                    unreachable!("matched string carrier");
                };
                Ok(value.clone())
            }
            _ => Err(VmHeapError::HeapOperationFailed {
                operation,
                message: "value is not a string carrier".to_string(),
            }),
        }
    }

    fn bytes_value(&self, value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        let operation = VmHeapOperation::RepresentationPayload;
        let heap_handle = self.request_handle(value, operation)?;
        match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Bytes(bytes) => Ok(bytes.as_slice().to_vec()),
            _ => Err(VmHeapError::HeapOperationFailed {
                operation,
                message: "value is not a bytes heap node".to_string(),
            }),
        }
    }

    fn array_get(&self, array: &ValueSlot, index: usize) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::ArrayGet;
        let heap_handle = self.request_handle(array, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Array(_))
        })?;
        if let Some(slots) = self.array_slots.get(&heap_handle) {
            return slots
                .get(index)
                .copied()
                .ok_or_else(|| VmHeapError::HeapOperationFailed {
                    operation,
                    message: format!("array index {index} is out of bounds"),
                });
        }
        let carrier = self
            .heap
            .array_item_carrier(heap_handle, index)
            .map_err(|error| self.map_error(error, operation))?
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: format!("array index {index} is out of bounds"),
            })?;
        self.slot_from_carrier(&carrier, operation)
    }

    fn array_len(&self, array: &ValueSlot) -> Result<usize, VmHeapError> {
        let operation = VmHeapOperation::ArrayLen;
        let heap_handle = self.request_handle(array, operation)?;
        match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Array(items) => Ok(items.len()),
            _ => Err(VmHeapError::OperationKindMismatch {
                operation,
                kind: ValueKind::RequestHeapRef,
            }),
        }
    }

    fn map_get(&self, map: &ValueSlot, key: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::MapGet;
        let key = self.map_key_from_slot(key, operation)?;
        let heap_handle = self.request_handle(map, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Map(_))
        })?;
        if let Some(slots) = self.map_slots.get(&heap_handle) {
            return slots.get(&key).map(|(_, value)| *value).ok_or_else(|| {
                VmHeapError::HeapOperationFailed {
                    operation,
                    message: "map key is absent".to_string(),
                }
            });
        }
        let carrier = self
            .heap
            .map_entry_carrier(heap_handle, &key)
            .map_err(|error| self.map_error(error, operation))?
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: "map key is absent".to_string(),
            })?;
        self.slot_from_carrier(&carrier, operation)
    }

    fn map_len(&self, map: &ValueSlot) -> Result<usize, VmHeapError> {
        let operation = VmHeapOperation::MapLen;
        let heap_handle = self.request_handle(map, operation)?;
        match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Map(map) => Ok(map.len()),
            _ => Err(VmHeapError::OperationKindMismatch {
                operation,
                kind: ValueKind::RequestHeapRef,
            }),
        }
    }

    fn map_entry_at(&self, map: &ValueSlot, ordinal: usize) -> Result<VmMapEntry, VmHeapError> {
        let operation = VmHeapOperation::MapEntryAt;
        let heap_handle = self.request_handle(map, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Map(_))
        })?;
        let map = match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Map(map) => map,
            _ => {
                return Err(VmHeapError::OperationKindMismatch {
                    operation,
                    kind: ValueKind::RequestHeapRef,
                })
            }
        };
        let (key, _) = map
            .iter()
            .nth(ordinal)
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: format!("map ordinal {ordinal} is out of bounds"),
            })?;
        let (key_slot, value_slot) = self
            .map_slots
            .get(&heap_handle)
            .and_then(|slots| slots.get(key).copied())
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: "map sidecar entry is absent".to_string(),
            })?;
        Ok(VmMapEntry {
            key: key_slot,
            value: value_slot,
        })
    }

    fn record_field(&self, record: &ValueSlot, field: &str) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::RecordField;
        let heap_handle = self.request_handle(record, operation)?;
        if let Some(slots) = self.object_slots.get(&heap_handle) {
            return slots
                .get(field)
                .copied()
                .ok_or_else(|| VmHeapError::HeapOperationFailed {
                    operation,
                    message: format!("record field {field:?} is absent"),
                });
        }
        let carrier = self
            .heap
            .object_field_carrier(heap_handle, field)
            .map_err(|error| self.map_error(error, operation))?
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: format!("record field {field:?} is absent"),
            })?;
        self.slot_from_carrier(&carrier, operation)
    }

    fn get_dense_field(
        &self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::RecordField;
        let heap_handle = self.request_handle(record, operation)?;
        if let Some(slots) = self.object_slots.get(&heap_handle) {
            return slots.values().nth(field_ordinal).copied().ok_or_else(|| {
                VmHeapError::HeapOperationFailed {
                    operation,
                    message: format!("record ordinal {field_ordinal} is out of bounds"),
                }
            });
        }
        match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Object(object) => {
                let value = object
                    .fields()
                    .values()
                    .nth(field_ordinal)
                    .cloned()
                    .ok_or_else(|| VmHeapError::HeapOperationFailed {
                        operation,
                        message: format!("record ordinal {field_ordinal} is out of bounds"),
                    })?;
                self.slot_from_carrier(&RuntimeValueCarrier::unidentified(value), operation)
            }
            _ => Err(VmHeapError::OperationKindMismatch {
                operation,
                kind: ValueKind::RequestHeapRef,
            }),
        }
    }

    fn representation_payload(&self, representation: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::RepresentationPayload;
        let heap_handle = self.request_handle(representation, operation)?;
        self.representation_slots
            .get(&heap_handle)
            .copied()
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: "value is not a representation carrier cell".to_string(),
            })
    }

    fn array_push_owned(&mut self, array: &ValueSlot, value: ValueSlot) -> Result<(), VmHeapError> {
        let operation = VmHeapOperation::ArrayPushOwned;
        let heap_handle = self.request_handle(array, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Array(_))
        })?;
        self.validate_live(&value)?;
        let carrier = self.runtime_carrier_for_slot(&value, operation)?;
        self.heap
            .push_array_item_carrier(heap_handle, carrier)
            .map_err(|error| self.map_error(error, operation))?;
        self.array_slots.entry(heap_handle).or_default().push(value);
        Ok(())
    }

    fn map_put_owned(
        &mut self,
        map: &ValueSlot,
        key: ValueSlot,
        value: ValueSlot,
    ) -> Result<bool, VmHeapError> {
        let operation = VmHeapOperation::MapPutOwned;
        let key_value = self.map_key_from_slot(&key, operation)?;
        let heap_handle = self.request_handle(map, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Map(_))
        })?;
        self.validate_live(&value)?;
        let carrier = self.runtime_carrier_for_slot(&value, operation)?;
        let existed = self
            .heap
            .set_map_entry_carrier(heap_handle, key_value.clone(), carrier)
            .map_err(|error| self.map_error(error, operation))?;
        self.map_slots
            .entry(heap_handle)
            .or_default()
            .insert(key_value, (key, value));
        Ok(existed)
    }

    fn set_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        let operation = VmHeapOperation::SetWritablePath;
        if segments.is_empty() {
            return Err(VmHeapError::HeapOperationFailed {
                operation,
                message: "writable path must contain at least one segment".to_string(),
            });
        }
        self.ensure_selector_count(segments, selectors, operation)?;
        self.validate_live(root)?;
        self.validate_live(&value)?;
        let mut current = *root;
        let mut selector_index = 0;
        for (segment_index, segment) in segments.iter().enumerate() {
            let terminal = segment_index + 1 == segments.len();
            match segment {
                VmHeapPathSegment::DenseField { field } => {
                    if terminal {
                        self.set_record_field(&current, field, value)?;
                    } else {
                        current = self.record_field(&current, field)?;
                    }
                }
                VmHeapPathSegment::ArrayIndex => {
                    let selector = selectors.get(selector_index).ok_or_else(|| {
                        VmHeapError::HeapOperationFailed {
                            operation,
                            message: "missing array selector".to_string(),
                        }
                    })?;
                    selector_index += 1;
                    let index = usize::try_from(
                        selector
                            .as_integer()
                            .ok_or(VmHeapError::InvalidValueMetadata)?,
                    )
                    .map_err(|_| VmHeapError::InvalidValueMetadata)?;
                    if terminal {
                        self.set_array_element(&current, index, value)?;
                    } else {
                        current = self.array_get(&current, index)?;
                    }
                }
                VmHeapPathSegment::MapKey => {
                    let selector = selectors.get(selector_index).ok_or_else(|| {
                        VmHeapError::HeapOperationFailed {
                            operation,
                            message: "missing map selector".to_string(),
                        }
                    })?;
                    selector_index += 1;
                    if terminal {
                        self.map_put_owned(&current, *selector, value)?;
                    } else {
                        current = self.map_get(&current, selector)?;
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod trait_dispatch_tests {
    use super::*;

    #[test]
    fn alloc_bytes_dispatches_through_the_vm_heap_trait_object() {
        let mut heap = RequestVmHeap::with_domain(9, 0, RequestHeapLimits::default());
        let heap: &mut dyn VmHeap = &mut heap;
        let slot = heap
            .alloc_bytes(vec![1, 2, 3])
            .expect("RequestVmHeap must implement alloc_bytes on the heap trait object");
        assert_eq!(slot.kind(), Some(ValueKind::RequestHeapRef));
        assert_eq!(heap.bytes_value(&slot), Ok(vec![1, 2, 3]));
    }
}

#[cfg(test)]
mod tests;

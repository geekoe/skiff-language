//! RequestHeap-backed implementation of the narrow VM heap port.
//!
//! The heap owns the stable `VmHandle` registry and ordinary snapshot share
//! accounting. RequestHeap remains the allocation arena for arrays, objects,
//! maps and representation carrier cells.

use std::{
    collections::{BTreeMap, HashMap},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_runtime_model::{
    error::RuntimeModelError as RuntimeError,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{HeapHandle, HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::CatchIdentity,
    vm_heap::{
        PinnedWritablePathSegment, VmContainerElement, VmContainerElements, VmContainerShape,
        VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation, VmHeapPathSegment, VmMapEntry,
        VmRecordField, WritablePathPreparation,
    },
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};
use skiff_runtime_scheduler::{
    RequestResourceHandle, RequestResourceLookupError, RequestResourceTable,
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

struct PreparedOwnerConsume {
    remaining_owners: HashMap<VmHandle, usize>,
    removals: Vec<(VmHandle, HeapHandle)>,
    resource_releases: Vec<RequestResourceHandle>,
}

pub struct RequestVmHeap {
    heap: RequestHeap,
    resources: Option<RequestResourceTable>,
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
}

impl RequestVmHeap {
    pub fn new(limits: RequestHeapLimits) -> Self {
        Self::with_domain(next_domain(), 0, limits)
    }

    pub fn new_with_epoch(epoch: u32, limits: RequestHeapLimits) -> Self {
        Self::with_domain(next_domain(), epoch, limits)
    }

    pub fn with_domain(domain: u8, epoch: u32, limits: RequestHeapLimits) -> Self {
        Self::with_domain_and_resources(domain, epoch, limits, None)
    }

    /// Constructs the production request heap bound to the exact scheduler
    /// resource table created by the same request execution context.
    pub(crate) fn for_execution(
        resources: RequestResourceTable,
        limits: RequestHeapLimits,
    ) -> Self {
        Self::with_domain_and_resources(next_domain(), 0, limits, Some(resources))
    }

    fn with_domain_and_resources(
        domain: u8,
        epoch: u32,
        limits: RequestHeapLimits,
        resources: Option<RequestResourceTable>,
    ) -> Self {
        Self {
            heap: RequestHeap::new_with_epoch(epoch, limits),
            resources,
            domain,
            next_serial: 1,
            live: HashMap::new(),
            handles_by_heap: HashMap::new(),
            released_heap_handles: HashMap::new(),
            array_slots: HashMap::new(),
            object_slots: HashMap::new(),
            map_slots: HashMap::new(),
            representation_slots: HashMap::new(),
        }
    }

    fn resources(&self, operation: VmHeapOperation) -> Result<&RequestResourceTable, VmHeapError> {
        self.resources
            .as_ref()
            .ok_or(VmHeapError::OperationKindMismatch {
                operation,
                kind: ValueKind::ResourceRef,
            })
    }

    fn map_resource_lookup(route: VmHandle, error: RequestResourceLookupError) -> VmHeapError {
        match error {
            RequestResourceLookupError::WrongOwner => VmHeapError::InvalidHandle {
                kind: ValueKind::ResourceRef,
                handle: route,
                reason: VmHandleInvalidReason::WrongDomain,
            },
            RequestResourceLookupError::UnknownSlot
            | RequestResourceLookupError::StaleGeneration => VmHeapError::InvalidHandle {
                kind: ValueKind::ResourceRef,
                handle: route,
                reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            },
            RequestResourceLookupError::RouteAlreadyClaimed
            | RequestResourceLookupError::VmRouteAlreadyAdmitted
            | RequestResourceLookupError::VmMetadataMismatch => VmHeapError::InvalidValueMetadata,
        }
    }

    fn validate_resource_slot(
        &self,
        value: &ValueSlot,
        operation: VmHeapOperation,
    ) -> Result<RequestResourceHandle, VmHeapError> {
        let route = value
            .as_resource_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        self.resources(operation)?
            .validate_vm_route_metadata(route, value.compact_type_tag(), value.flags())
            .map_err(|error| Self::map_resource_lookup(route, error))
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

    fn snapshot_owner_count(&self, container: &ValueSlot) -> Result<usize, VmHeapError> {
        Ok(self.live_entry(container)?.snapshot_owners)
    }

    fn ensure_exclusive_owner(
        &self,
        container: &ValueSlot,
        _operation: VmHeapOperation,
    ) -> Result<(), VmHeapError> {
        let vm_handle = container
            .as_request_heap_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        let owners = self.snapshot_owner_count(container)?;
        if owners > 1 {
            return Err(VmHeapError::OwnershipViolation {
                kind: ValueKind::RequestHeapRef,
                handle: vm_handle,
            });
        }
        Ok(())
    }

    fn restore_shares(&mut self, shared: &[ValueSlot]) -> Result<(), VmHeapError> {
        for slot in shared {
            self.release_snapshot(slot)?;
        }
        Ok(())
    }

    fn take_container_children(&mut self, heap_handle: HeapHandle) -> Vec<ValueSlot> {
        let mut children = Vec::new();
        if let Some(slots) = self.array_slots.remove(&heap_handle) {
            children.extend(slots);
        }
        if let Some(slots) = self.object_slots.remove(&heap_handle) {
            children.extend(slots.into_values());
        }
        if let Some(slots) = self.map_slots.remove(&heap_handle) {
            for (_, (key, value)) in slots {
                children.push(key);
                children.push(value);
            }
        }
        if let Some(payload) = self.representation_slots.remove(&heap_handle) {
            children.push(payload);
        }
        children
    }

    fn container_children_except(
        &self,
        heap_handle: HeapHandle,
        excluded_object_field: Option<&str>,
    ) -> Vec<ValueSlot> {
        let mut children = Vec::new();
        if let Some(slots) = self.array_slots.get(&heap_handle) {
            children.extend(slots.iter().copied());
        }
        if let Some(slots) = self.object_slots.get(&heap_handle) {
            children.extend(
                slots
                    .iter()
                    .filter(|(field, _)| excluded_object_field != Some(field.as_str()))
                    .map(|(_, value)| *value),
            );
        }
        if let Some(slots) = self.map_slots.get(&heap_handle) {
            for (key, value) in slots.values() {
                children.push(*key);
                children.push(*value);
            }
        }
        if let Some(payload) = self.representation_slots.get(&heap_handle) {
            children.push(*payload);
        }
        children
    }

    /// Validates the entire recursive remainder drop before the selected
    /// field is physically detached. Owner counts are simulated per handle,
    /// so malformed duplicate edges, cycles, stale children, and resources
    /// that this heap cannot terminate all fail without changing state.
    fn prepare_owner_consume(
        &self,
        root: ValueSlot,
        root_heap_handle: HeapHandle,
        excluded_root_field: &str,
    ) -> Result<PreparedOwnerConsume, VmHeapError> {
        let operation = VmHeapOperation::TakeDenseField;
        let mut remaining_owners = HashMap::new();
        let mut removals = Vec::new();
        let mut resource_releases = Vec::new();
        let mut seen_resources = std::collections::HashSet::new();
        let mut pending = vec![root];
        while let Some(owner) = pending.pop() {
            let Some(kind) = owner.kind() else {
                return Err(VmHeapError::InvalidValueMetadata);
            };
            match kind {
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date => self.validate_live(&owner)?,
                ValueKind::RequestHeapRef => {
                    let handle = owner
                        .as_request_heap_ref()
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    let entry = self.live_entry(&owner)?;
                    let heap_handle = entry.heap_handle;
                    let actual_owners = entry.snapshot_owners;
                    let remaining = remaining_owners.entry(handle).or_insert(actual_owners);
                    *remaining =
                        remaining
                            .checked_sub(1)
                            .ok_or(VmHeapError::OwnershipViolation {
                                kind: ValueKind::RequestHeapRef,
                                handle,
                            })?;
                    if *remaining == 0 {
                        removals.push((handle, heap_handle));
                        let excluded =
                            (heap_handle == root_heap_handle).then_some(excluded_root_field);
                        pending.extend(self.container_children_except(heap_handle, excluded));
                    }
                }
                ValueKind::ResourceRef => {
                    let route = owner
                        .as_resource_ref()
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    let handle = self.validate_resource_slot(&owner, operation)?;
                    if !seen_resources.insert(route) {
                        return Err(VmHeapError::OwnershipViolation {
                            kind: ValueKind::ResourceRef,
                            handle: route,
                        });
                    }
                    resource_releases.push(handle);
                }
                kind => {
                    return Err(VmHeapError::OperationKindMismatch { operation, kind });
                }
            }
        }
        Ok(PreparedOwnerConsume {
            remaining_owners,
            removals,
            resource_releases,
        })
    }

    /// Applies only facts frozen by `prepare_owner_consume`. The caller holds
    /// `&mut self`, so no live entry can change between prepare and commit.
    fn commit_owner_consume(&mut self, prepared: PreparedOwnerConsume) {
        for (handle, remaining) in &prepared.remaining_owners {
            if *remaining == 0 {
                continue;
            }
            self.live
                .get_mut(handle)
                .expect("prepared live owner remains installed")
                .snapshot_owners = *remaining;
        }
        for (handle, heap_handle) in prepared.removals {
            let entry = self
                .live
                .remove(&handle)
                .expect("prepared live removal remains installed");
            debug_assert_eq!(entry.heap_handle, heap_handle);
            drop(self.take_container_children(heap_handle));
            self.handles_by_heap.remove(&heap_handle);
            self.released_heap_handles.insert(heap_handle, handle);
        }
        if !prepared.resource_releases.is_empty() {
            let resources = self
                .resources
                .as_ref()
                .expect("prepared resource releases retain the request table");
            for handle in prepared.resource_releases {
                resources
                    .release(&handle)
                    .expect("a fully prevalidated exact resource release cannot fail");
            }
        }
    }

    fn release_replaced_slot(&mut self, slot: &ValueSlot) -> Result<(), VmHeapError> {
        match slot.kind() {
            Some(ValueKind::RequestHeapRef) => self.release_snapshot(slot),
            Some(ValueKind::ResourceRef) => self.release_resource(slot),
            _ => Ok(()),
        }
    }

    fn replace_child_slot(
        &mut self,
        container: &ValueSlot,
        segment: &PinnedWritablePathSegment,
        value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        let operation = VmHeapOperation::CommitWritablePath;
        let heap_handle = self.request_handle(container, operation)?;
        let carrier = self.runtime_carrier_for_slot(&value, operation)?;
        let old = match segment {
            PinnedWritablePathSegment::DenseField { field } => {
                let old = self
                    .object_slots
                    .get(&heap_handle)
                    .and_then(|slots| slots.get(field))
                    .copied();
                self.heap
                    .set_object_field_carrier(heap_handle, field.clone(), carrier)
                    .map_err(|error| self.map_error(error, operation))?;
                self.object_slots
                    .entry(heap_handle)
                    .or_default()
                    .insert(field.clone(), value);
                old
            }
            PinnedWritablePathSegment::ArrayIndex { index } => {
                let old = self
                    .array_slots
                    .get(&heap_handle)
                    .and_then(|slots| slots.get(*index))
                    .copied();
                if old.is_none() {
                    return Err(VmHeapError::HeapOperationFailed {
                        operation,
                        message: format!("array index {index} is out of bounds"),
                    });
                }
                self.heap
                    .set_array_item_carrier(heap_handle, *index, carrier)
                    .map_err(|error| self.map_error(error, operation))?;
                self.array_slots
                    .entry(heap_handle)
                    .or_default()
                    .get_mut(*index)
                    .map(|slot| *slot = value)
                    .ok_or_else(|| VmHeapError::HeapOperationFailed {
                        operation,
                        message: format!("array index {index} is out of bounds"),
                    })?;
                old
            }
            PinnedWritablePathSegment::MapKey { .. } => {
                return Err(VmHeapError::OperationKindMismatch {
                    operation,
                    kind: ValueKind::RequestHeapRef,
                });
            }
        };
        if let Some(old) = old {
            self.release_replaced_slot(&old)?;
        }
        Ok(())
    }

    fn clone_container(&mut self, container: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::CommitWritablePath;
        let heap_handle = self.request_handle(container, operation)?;
        let node = self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?;
        let (children, names): (Vec<ValueSlot>, Option<Vec<String>>) = match node {
            HeapNode::Array(_) => {
                let slots = self.array_slots.get(&heap_handle).cloned().ok_or_else(|| {
                    VmHeapError::HeapOperationFailed {
                        operation,
                        message: "array container has no slot sidecar".to_string(),
                    }
                })?;
                (slots, None)
            }
            HeapNode::Object(_) => {
                let slots = self
                    .object_slots
                    .get(&heap_handle)
                    .cloned()
                    .ok_or_else(|| VmHeapError::HeapOperationFailed {
                        operation,
                        message: "record container has no slot sidecar".to_string(),
                    })?;
                let names = slots.keys().cloned().collect::<Vec<_>>();
                (slots.into_values().collect(), Some(names))
            }
            _ => {
                return Err(VmHeapError::OperationKindMismatch {
                    operation,
                    kind: ValueKind::RequestHeapRef,
                });
            }
        };
        for child in &children {
            self.validate_live(child)?;
        }
        let mut shared = Vec::with_capacity(children.len());
        for child in &children {
            shared.push(self.snapshot_share(child)?);
        }
        let carriers = children
            .iter()
            .map(|child| self.runtime_carrier_for_slot(child, operation))
            .collect::<Result<Vec<_>, _>>()?;
        self.ensure_serial_available(operation)?;
        let allocated = match &names {
            None => self
                .heap
                .alloc_array_carriers(carriers)
                .map_err(|error| self.map_error(error, operation)),
            Some(names) => {
                let fields = names
                    .iter()
                    .cloned()
                    .zip(carriers)
                    .collect::<BTreeMap<_, _>>();
                self.heap
                    .alloc_object_carriers(fields)
                    .map_err(|error| self.map_error(error, operation))
            }
        };
        let new_handle = match allocated {
            Ok(handle) => handle,
            Err(error) => {
                self.restore_shares(&shared)?;
                return Err(error);
            }
        };
        let slot =
            match self.register_handle(new_handle, container.compact_type_tag(), container.flags())
            {
                Ok(slot) => slot,
                Err(error) => {
                    self.restore_shares(&shared)?;
                    return Err(error);
                }
            };
        match names {
            None => {
                self.array_slots.insert(new_handle, shared);
            }
            Some(names) => {
                self.object_slots
                    .insert(new_handle, names.into_iter().zip(shared).collect());
            }
        }
        Ok(slot)
    }

    fn commit_copy_on_write(
        &mut self,
        prepared: WritablePathPreparation,
        value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        let segments = prepared.segments().to_vec();
        let containers = prepared.containers().to_vec();
        let new_root = self.clone_container(&containers[0])?;
        let mut new_current = new_root;
        for (segment_index, segment) in segments.iter().enumerate() {
            let terminal = segment_index + 1 == segments.len();
            let replacement = if terminal {
                value
            } else {
                self.clone_container(&containers[segment_index + 1])?
            };
            self.replace_child_slot(&new_current, segment, replacement)?;
            if !terminal {
                new_current = replacement;
            }
        }
        Ok(new_root)
    }

    fn prepare_writable_path_impl(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        let operation = VmHeapOperation::PrepareWritablePath;
        if segments.is_empty() {
            return Err(VmHeapError::HeapOperationFailed {
                operation,
                message: "writable path must contain at least one segment".to_string(),
            });
        }
        self.ensure_selector_count(segments, selectors, operation)?;
        self.validate_live(root)?;
        for selector in selectors {
            self.validate_live(selector)?;
        }
        let mut resolved = Vec::with_capacity(segments.len());
        let mut containers = Vec::with_capacity(segments.len());
        let mut current = *root;
        let mut selector_index = 0usize;
        for (segment_index, segment) in segments.iter().enumerate() {
            containers.push(current);
            let terminal = segment_index + 1 == segments.len();
            let child = match segment {
                VmHeapPathSegment::DenseField { field } => {
                    resolved.push(PinnedWritablePathSegment::DenseField {
                        field: field.clone(),
                    });
                    self.record_field(&current, field)?
                }
                VmHeapPathSegment::ArrayIndex => {
                    let selector = selectors.get(selector_index).ok_or_else(|| {
                        VmHeapError::HeapOperationFailed {
                            operation,
                            message: "missing array selector".to_string(),
                        }
                    })?;
                    selector_index += 1;
                    let index = skiff_runtime_model::vm_heap::collection_index(selector)
                        .ok_or(VmHeapError::InvalidValueMetadata)?;
                    resolved.push(PinnedWritablePathSegment::ArrayIndex { index });
                    self.array_get(&current, index)?
                }
                VmHeapPathSegment::MapKey => {
                    let selector = selectors.get(selector_index).ok_or_else(|| {
                        VmHeapError::HeapOperationFailed {
                            operation,
                            message: "missing map selector".to_string(),
                        }
                    })?;
                    selector_index += 1;
                    resolved.push(PinnedWritablePathSegment::MapKey { key: *selector });
                    self.map_get(&current, selector)?
                }
            };
            if terminal {
                return WritablePathPreparation::new(
                    *root,
                    resolved.into_boxed_slice(),
                    containers.into_boxed_slice(),
                    Some(child),
                );
            }
            current = child;
        }
        Err(VmHeapError::HeapOperationFailed {
            operation,
            message: "writable path resolution did not terminate".to_string(),
        })
    }
}

impl VmHeap for RequestVmHeap {
    fn admit_resource_ref(
        &mut self,
        route: VmHandle,
        compact_type_tag: CompactTypeTag,
        flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        self.resources(VmHeapOperation::ValidateLive)?
            .admit_vm_route(route, compact_type_tag, flags)
            .map_err(|error| Self::map_resource_lookup(route, error))?;
        Ok(ValueSlot::resource_ref(route, compact_type_tag, flags))
    }

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
            ValueKind::ResourceRef => self
                .validate_resource_slot(value, VmHeapOperation::ValidateLive)
                .map(|_| ()),
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
                self.validate_resource_slot(source, VmHeapOperation::TransferOwner)?;
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
                    let children = self.take_container_children(heap_handle);
                    self.handles_by_heap.remove(&entry.heap_handle);
                    self.released_heap_handles
                        .insert(entry.heap_handle, vm_handle);
                    // Recursive snapshot drop: nested aggregates owned by this
                    // container lose exactly one owner each. The guard skips
                    // already-released children so a self-referential container
                    // cannot recurse forever.
                    for child in children {
                        if self.validate_live(&child).is_ok() {
                            self.release_snapshot(&child)?;
                        }
                    }
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
        if kind != ValueKind::ResourceRef {
            return Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseResource,
                kind,
            });
        }
        let route = owner
            .as_resource_ref()
            .ok_or(VmHeapError::InvalidValueMetadata)?;
        self.resources(VmHeapOperation::ReleaseResource)?
            .release_vm_route_metadata(route, owner.compact_type_tag(), owner.flags())
            .map_err(|error| Self::map_resource_lookup(route, error))?;
        Ok(())
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

    fn take_dense_field(
        &mut self,
        record: &ValueSlot,
        field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::TakeDenseField;
        let heap_handle = self.request_handle(record, operation)?;
        self.ensure_exclusive_owner(record, operation)?;
        self.ensure_node_kind(heap_handle, operation, |node| {
            matches!(node, HeapNode::Object(_))
        })?;

        let (field, value) = self
            .object_slots
            .get(&heap_handle)
            .and_then(|slots| slots.iter().nth(field_ordinal))
            .map(|(field, value)| (field.clone(), *value))
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: format!("record ordinal {field_ordinal} is out of bounds"),
            })?;
        if self
            .heap
            .object_field_carrier(heap_handle, &field)
            .map_err(|error| self.map_error(error, operation))?
            .is_none()
        {
            return Err(VmHeapError::HeapOperationFailed {
                operation,
                message: format!("record field {field:?} is absent from the physical object"),
            });
        }
        match value.kind() {
            Some(ValueKind::RequestHeapRef) => {
                self.live_entry(&value)?;
            }
            Some(
                ValueKind::Null
                | ValueKind::Bool
                | ValueKind::Number
                | ValueKind::Integer
                | ValueKind::Date,
            ) => self.validate_live(&value)?,
            Some(ValueKind::ResourceRef) => {
                self.validate_resource_slot(&value, operation)?;
            }
            Some(kind) => {
                return Err(VmHeapError::OperationKindMismatch { operation, kind });
            }
            None => return Err(VmHeapError::InvalidValueMetadata),
        }
        let prepared = self.prepare_owner_consume(*record, heap_handle, &field)?;

        // Detach the authoritative VM slot first. RequestHeap deletion has a
        // prepare-before-mutate contract; if it rejects the target, restoring
        // this sidecar is infallible and the logical record is unchanged.
        let detached = self
            .object_slots
            .get_mut(&heap_handle)
            .and_then(|slots| slots.remove(&field))
            .ok_or_else(|| VmHeapError::HeapOperationFailed {
                operation,
                message: format!("record field {field:?} sidecar disappeared"),
            })?;
        debug_assert!(detached == value);
        let deleted = self
            .heap
            .delete_object_field(heap_handle, &field)
            .map_err(|error| self.map_error(error, operation));
        match deleted {
            Ok(true) => {}
            Ok(false) => {
                self.object_slots
                    .get_mut(&heap_handle)
                    .expect("validated object sidecar remains installed")
                    .insert(field.clone(), detached);
                return Err(VmHeapError::HeapOperationFailed {
                    operation,
                    message: format!("record field {field:?} was not physically present"),
                });
            }
            Err(error) => {
                self.object_slots
                    .get_mut(&heap_handle)
                    .expect("validated object sidecar remains installed")
                    .insert(field, detached);
                return Err(error);
            }
        }

        // No fallible work is allowed beyond the physical detach. The exact
        // recursive owner transition was frozen before any mutation.
        self.commit_owner_consume(prepared);
        Ok(detached)
    }

    fn container_elements(
        &self,
        container: &ValueSlot,
    ) -> Result<VmContainerElements, VmHeapError> {
        let operation = VmHeapOperation::ContainerElements;
        let heap_handle = self.request_handle(container, operation)?;
        match self
            .heap
            .get(heap_handle)
            .map_err(|error| self.map_error(error, operation))?
        {
            HeapNode::Array(_) => {
                let elements = self
                    .array_slots
                    .get(&heap_handle)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|value| VmContainerElement { field: None, value })
                    .collect();
                Ok(VmContainerElements {
                    shape: VmContainerShape::Array,
                    elements,
                })
            }
            HeapNode::Object(_) => {
                let elements = self
                    .object_slots
                    .get(&heap_handle)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|(field, value)| VmContainerElement {
                        field: Some(field),
                        value,
                    })
                    .collect();
                Ok(VmContainerElements {
                    shape: VmContainerShape::Record,
                    elements,
                })
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
        self.ensure_exclusive_owner(array, operation)?;
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
        self.ensure_exclusive_owner(map, operation)?;
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

    fn prepare_writable_path(
        &mut self,
        root: &ValueSlot,
        segments: &[VmHeapPathSegment],
        selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        self.prepare_writable_path_impl(root, segments, selectors)
    }

    fn commit_writable_path(
        &mut self,
        prepared: WritablePathPreparation,
        value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        let operation = VmHeapOperation::CommitWritablePath;
        self.validate_live(&prepared.root())?;
        self.validate_live(&value)?;
        let exclusive = prepared
            .containers()
            .iter()
            .all(|container| self.snapshot_owner_count(container) == Ok(1));
        if exclusive {
            let terminal =
                *prepared
                    .containers()
                    .last()
                    .ok_or_else(|| VmHeapError::HeapOperationFailed {
                        operation,
                        message: "writable path preparation has no terminal container".to_string(),
                    })?;
            let segment =
                prepared
                    .segments()
                    .last()
                    .ok_or_else(|| VmHeapError::HeapOperationFailed {
                        operation,
                        message: "writable path preparation has no terminal segment".to_string(),
                    })?;
            self.replace_child_slot(&terminal, segment, value)?;
            Ok(prepared.root())
        } else {
            self.commit_copy_on_write(prepared, value)
        }
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

//! Heap-neutral value operations used by the bytecode VM.
//!
//! [`ValueSlot`] is `Copy` so the VM can move its fixed-width physical bits
//! efficiently. Copying those bits does not create another semantic owner.
//! The verified VM interprets image-local lifecycle plans and selects one of
//! the synchronous physical primitives below. The heap keeps all generation,
//! domain, share, edit, and affine ownership state private. No artifact plan,
//! native adapter, callback, future, or ownership token crosses this port.

use crate::{
    service_error::CatchIdentity,
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};
use std::{any::Any, fmt, num::NonZeroU64, sync::Arc};

/// Debug projection for [`ValueSlot`], which intentionally has no `Debug`
/// impl of its own. Exposing only kind and handle keeps the projection free of
/// any artifact/type identity that a heap-neutral model must not interpret.
struct ValueSlotDebug<'a>(&'a ValueSlot);

impl fmt::Debug for ValueSlotDebug<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ValueSlot")
            .field("kind", &self.0.kind())
            .field("handle", &self.0.as_handle())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct VmMapEntry {
    pub key: ValueSlot,
    pub value: ValueSlot,
}

#[derive(Clone, PartialEq, Eq)]
pub struct VmRecordField {
    pub name: String,
    pub value: ValueSlot,
}

/// One immediate child of an aggregate container, for boundary traversal.
#[derive(Clone, PartialEq, Eq)]
pub struct VmContainerElement {
    /// `None` for array elements; the canonical field name for record fields.
    pub field: Option<String>,
    pub value: ValueSlot,
}

/// The canonical aggregate shape of one container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmContainerShape {
    Array,
    Record,
}

/// Immediate children plus the exact container shape of one aggregate.
#[derive(Clone, PartialEq, Eq)]
pub struct VmContainerElements {
    pub shape: VmContainerShape,
    pub elements: Vec<VmContainerElement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmHeapPathSegment {
    DenseField { field: String },
    ArrayIndex,
    MapKey,
}

/// One fully resolved writable path segment pinned by
/// [`VmHeap::prepare_writable_path`].
///
/// Resolution consumes the positionally ordered selectors: every `ArrayIndex`
/// and `MapKey` resolves against the next selector at prepare time, so commit
/// never re-resolves an intermediate path fact.
#[derive(Clone, PartialEq, Eq)]
pub enum PinnedWritablePathSegment {
    DenseField { field: String },
    ArrayIndex { index: usize },
    MapKey { key: ValueSlot },
}

/// Reads one canonical collection index from a selector slot.
///
/// Array and map ordinal indices are `integer`-or-`number` by the canonical
/// `CollectionIndex` input class: an integer immediate is accepted as-is, and
/// an integral non-negative finite number is accepted as the same index. Any
/// other value shape is not a collection index.
pub fn collection_index(selector: &ValueSlot) -> Option<usize> {
    if let Some(index) = selector.as_integer() {
        return usize::try_from(index).ok();
    }
    let number = selector.as_number()?;
    if !number.is_finite() || number < 0.0 || number.fract() != 0.0 {
        return None;
    }
    if number >= (usize::MAX as f64) {
        return None;
    }
    Some(number as usize)
}

impl fmt::Debug for PinnedWritablePathSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DenseField { field } => formatter
                .debug_struct("DenseField")
                .field("field", field)
                .finish(),
            Self::ArrayIndex { index } => formatter
                .debug_struct("ArrayIndex")
                .field("index", index)
                .finish(),
            Self::MapKey { key } => formatter
                .debug_struct("MapKey")
                .field("key", &ValueSlotDebug(key))
                .finish(),
        }
    }
}

/// Single-use pinned path facts produced by [`VmHeap::prepare_writable_path`]
/// and consumed exactly once by [`VmHeap::commit_writable_path`].
///
/// The type is deliberately opaque to the VM: the fiber holds it without
/// inspecting its contents, and it is intentionally not `Clone` so a pin can
/// never be split into two competing commits. The concrete heap owns every
/// meaning; the model only carries the neutral facts every implementation
/// pins: the owned root, the fully resolved segment chain, the container slot
/// each segment applies to, and the terminal leaf being replaced.
pub struct WritablePathPreparation {
    root: ValueSlot,
    segments: Box<[PinnedWritablePathSegment]>,
    containers: Box<[ValueSlot]>,
    leaf: Option<ValueSlot>,
}

impl fmt::Debug for WritablePathPreparation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let containers = self
            .containers
            .iter()
            .map(ValueSlotDebug)
            .collect::<Vec<_>>();
        formatter
            .debug_struct("WritablePathPreparation")
            .field("root", &ValueSlotDebug(&self.root))
            .field("segments", &self.segments)
            .field("containers", &containers)
            .field("leaf", &self.leaf.as_ref().map(ValueSlotDebug))
            .finish()
    }
}

impl WritablePathPreparation {
    /// Pins one non-empty resolved path. `containers[i]` is the aggregate slot
    /// that `segments[i]` applies to; `containers[0]` must equal `root`.
    pub fn new(
        root: ValueSlot,
        segments: Box<[PinnedWritablePathSegment]>,
        containers: Box<[ValueSlot]>,
        leaf: Option<ValueSlot>,
    ) -> Result<Self, VmHeapError> {
        if segments.is_empty() {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::PrepareWritablePath,
                message: "writable path preparation must pin at least one segment".to_string(),
            });
        }
        if segments.len() != containers.len() {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::PrepareWritablePath,
                message: format!(
                    "writable path preparation pins {} segments for {} containers",
                    segments.len(),
                    containers.len()
                ),
            });
        }
        if containers.first().copied() != Some(root) {
            return Err(VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::PrepareWritablePath,
                message: "writable path preparation root does not match its first container"
                    .to_string(),
            });
        }
        Ok(Self {
            root,
            segments,
            containers,
            leaf,
        })
    }

    /// The owned root slot pinned by prepare.
    pub fn root(&self) -> ValueSlot {
        self.root
    }

    /// The fully resolved segment chain, in path order.
    pub fn segments(&self) -> &[PinnedWritablePathSegment] {
        &self.segments
    }

    /// The container slot each segment applies to; `containers[0] == root`.
    pub fn containers(&self) -> &[ValueSlot] {
        &self.containers
    }

    /// The terminal leaf being replaced, when the terminal container held one.
    pub fn leaf(&self) -> Option<ValueSlot> {
        self.leaf
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmHeapOperation {
    ValidateLive,
    AllocateArray,
    AllocateMap,
    AllocateRecord,
    AllocateRepresentation,
    AllocateLocalInterface,
    ArrayGet,
    ArrayLen,
    MapGet,
    MapLen,
    MapEntryAt,
    RecordField,
    TakeDenseField,
    RepresentationPayload,
    LocalInterfacePayload,
    LocalInterfaceTable,
    ContainerElements,
    ArrayPushOwned,
    MapPutOwned,
    PrepareWritablePath,
    CommitWritablePath,
    SnapshotShare,
    TransferOwner,
    ReleaseSnapshot,
    ReleaseResource,
}

/// Exact opaque local-interface table facts carried by one heap carrier.
///
/// The concrete linked table lives behind `exact` so heap-neutral model code
/// can check the same indexed identity the VM sees while the request heap can
/// still recover the exact linked table for a local child dispatch. The exact
/// value is deliberately opaque to this module: no artifact-specific type is
/// allowed to cross the heap port.
#[derive(Clone)]
pub struct VmLocalInterfaceTable {
    table_index: u32,
    concrete_type: u32,
    method_count: usize,
    exact: Arc<dyn Any + Send + Sync>,
}

impl VmLocalInterfaceTable {
    pub const fn new(
        table_index: u32,
        concrete_type: u32,
        method_count: usize,
        exact: Arc<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            table_index,
            concrete_type,
            method_count,
            exact,
        }
    }

    pub const fn table_index(&self) -> u32 {
        self.table_index
    }

    pub const fn concrete_type(&self) -> u32 {
        self.concrete_type
    }

    pub const fn method_count(&self) -> usize {
        self.method_count
    }

    pub const fn exact(&self) -> &Arc<dyn Any + Send + Sync> {
        &self.exact
    }
}

impl fmt::Debug for VmLocalInterfaceTable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VmLocalInterfaceTable")
            .field("table_index", &self.table_index)
            .field("concrete_type", &self.concrete_type)
            .field("method_count", &self.method_count)
            .finish_non_exhaustive()
    }
}

/// Request-scoped identity of one owner-local VM heap.
///
/// Domain identities are minted by the request memory ledger, never by a
/// global counter. The type is non-wrapping and intentionally wider than the
/// old 8-bit request-heap domain so a request can never reuse a stale domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HeapDomainId(NonZeroU64);

impl HeapDomainId {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub fn try_new(value: u64) -> Option<Self> {
        Some(Self(NonZeroU64::new(value)?))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Epoch stamped on one owner-local heap at mint time.
///
/// Child heap carriers use this to reject stale handles after a whole-heap
/// replacement, independently of request-scoped domain identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HeapEpoch(u32);

impl HeapEpoch {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, thiserror::Error)]
pub enum VmHandleInvalidReason {
    #[error("handle belongs to another heap domain")]
    WrongDomain,
    #[error("handle generation or epoch is stale")]
    StaleGenerationOrEpoch,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VmHeapError {
    #[error("value slot metadata is invalid")]
    InvalidValueMetadata,
    #[error("invalid {kind:?} handle {handle:?}: {reason}")]
    InvalidHandle {
        kind: ValueKind,
        handle: VmHandle,
        reason: VmHandleInvalidReason,
    },
    #[error("{operation:?} is not a valid physical operation for {kind:?}")]
    OperationKindMismatch {
        operation: VmHeapOperation,
        kind: ValueKind,
    },
    #[error("{kind:?} handle {handle:?} does not own the required physical state")]
    OwnershipViolation { kind: ValueKind, handle: VmHandle },
    #[error("{operation:?} failed: {message}")]
    HeapOperationFailed {
        operation: VmHeapOperation,
        message: String,
    },
    #[error(
        "heap resource limit exceeded during {operation:?}: limit {limit}, current {current}, requested delta {requested_delta}"
    )]
    ResourceLimitExceeded {
        operation: VmHeapOperation,
        limit: usize,
        current: usize,
        requested_delta: usize,
    },
}

/// Narrow value/heap port consumed by the VM dispatch loop.
///
/// Implementations own stable-handle domain and generation checks. Every
/// method must fail closed for foreign or stale handles even when bytecode has
/// already passed semantic verification. An error from a mutating method must
/// leave logical ownership, liveness, and share state unchanged. Every
/// `CompactTypeTag` parameter carries one present, exact linked type index;
/// absence is represented only by an immediate [`ValueSlot`], never by a tag
/// sentinel.
pub trait VmHeap {
    /// Optional concrete-heap projection for request-owned DB child
    /// composition. The VM core never uses this; only the request boundary can
    /// downcast a heap it already owns.
    fn as_any(&self) -> Option<&dyn Any> {
        None
    }

    /// Mutable concrete-heap projection for the request-owned DB child
    /// boundary. The VM core never uses this.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }

    /// Validates metadata and the liveness/domain of any referenced handle.
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError>;

    /// Admits one scheduler-minted opaque route as the sole VM `ResourceRef`.
    ///
    /// The neutral heap port carries only fixed-width route metadata. Concrete
    /// request composition must validate it against the request's single
    /// scheduler-owned resource authority before returning the slot.
    fn admit_resource_ref(
        &mut self,
        _route: VmHandle,
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ValidateLive,
            kind: ValueKind::ResourceRef,
        })
    }

    /// Performs the heap-local share transition for an ordinary snapshot.
    ///
    /// The returned slot may have identical bits, but represents a second
    /// semantic snapshot only after this method succeeds. This method never
    /// invokes an explicit clone adapter; immediate values are an unchanged
    /// physical no-op. On error it must not change heap state, and `source`
    /// remains the sole caller-owned value.
    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError>;

    /// Performs the heap-local transition for a logical owner move.
    ///
    /// On success the verified VM must atomically install the returned slot and
    /// mark its source storage dead. Borrowing this `Copy` type does not perform
    /// that commit or authorize a second owner. Immediate values require no heap
    /// mutation. On error heap state is unchanged and the caller retains
    /// `source` as its logical owner.
    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError>;

    /// Releases one ordinary snapshot owner from heap-local share accounting.
    ///
    /// Each successful call releases exactly one logical owner; the verified VM
    /// must call it once per live owner even when multiple slots contain the
    /// same bits. Releasing the final owner of an aggregate must preflight its
    /// complete owned carrier graph, including exact resource routes, before
    /// changing any owner count, sidecar, route, or liveness entry. The caller
    /// clears its slot only after success. On error heap/resource state is
    /// unchanged, the caller retains `owner`, and retrying the same operation
    /// is safe.
    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError>;

    /// Releases one exact ResourceTable owner without relying on GC finalizers.
    ///
    /// Release is exact and idempotent. The caller clears its slot only after
    /// success. On error heap/resource state is unchanged, the caller retains
    /// `owner`, and retrying the same operation is safe. Native lifecycle
    /// adapters are scheduled outside this port and outside any heap lock.
    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError>;

    /// Allocates an array whose elements are one-to-one child slots.
    fn allocate_array(
        &mut self,
        _elements: &[ValueSlot],
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateArray,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates a canonical map from key/value child slots.
    fn allocate_map(
        &mut self,
        _entries: &[VmMapEntry],
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateMap,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates a dense record from named field slots.
    fn allocate_record(
        &mut self,
        _fields: &[VmRecordField],
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateRecord,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates a nominal representation wrapper over one payload slot.
    fn allocate_representation(
        &mut self,
        _payload: &ValueSlot,
        _identity: CatchIdentity,
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateRepresentation,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates bytes carrying the exact verified concrete type metadata.
    fn alloc_typed_bytes(
        &mut self,
        _value: Vec<u8>,
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateArray,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates a string carrying the exact verified concrete type metadata.
    fn alloc_typed_string(
        &mut self,
        _value: String,
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateRepresentation,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads the string payload of one request-local string carrier cell.
    fn string_value(&self, _value: &ValueSlot) -> Result<String, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RepresentationPayload,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads the bytes payload of one request-local bytes heap node.
    fn bytes_value(&self, _value: &ValueSlot) -> Result<Vec<u8>, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RepresentationPayload,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads one array element. Out-of-range reads fail closed.
    fn array_get(&self, _array: &ValueSlot, _index: usize) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ArrayGet,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads an array length.
    fn array_len(&self, _array: &ValueSlot) -> Result<usize, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ArrayLen,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads a canonical map entry by exact key.
    fn map_get(&self, _map: &ValueSlot, _key: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::MapGet,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads a canonical map length.
    fn map_len(&self, _map: &ValueSlot) -> Result<usize, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::MapLen,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads one canonical map entry by ordinal. The ordinal is a runtime
    /// internal snapshot index, not a source-level array index.
    fn map_entry_at(&self, _map: &ValueSlot, _ordinal: usize) -> Result<VmMapEntry, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::MapEntryAt,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads one dense record field by canonical field name.
    fn record_field(&self, _record: &ValueSlot, _field: &str) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RecordField,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads the payload of one representation wrapper.
    fn representation_payload(
        &self,
        _representation: &ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RepresentationPayload,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Allocates one local-interface carrier over the concrete payload slot.
    ///
    /// The carrier stores the exact opaque linked table plus one live payload
    /// owner. Allocation validates payload liveness and stores the table
    /// identity so later child dispatch cannot confuse carriers from another
    /// table, concrete type, or method surface.
    fn allocate_local_interface(
        &mut self,
        _payload: &ValueSlot,
        _table: VmLocalInterfaceTable,
        _compact_type_tag: CompactTypeTag,
        _flags: ValueFlags,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::AllocateLocalInterface,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads the payload owner of one local-interface carrier.
    fn local_interface_payload(&self, _carrier: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::LocalInterfacePayload,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads the checked identity of one local-interface carrier.
    fn local_interface_table(
        &self,
        _carrier: &ValueSlot,
    ) -> Result<VmLocalInterfaceTable, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::LocalInterfaceTable,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Appends one owned value to an owned array.
    fn array_push_owned(
        &mut self,
        _array: &ValueSlot,
        _value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ArrayPushOwned,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Upserts one owned key/value pair. Returns whether the key already
    /// existed before this operation.
    fn map_put_owned(
        &mut self,
        _map: &ValueSlot,
        _key: ValueSlot,
        _value: ValueSlot,
    ) -> Result<bool, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::MapPutOwned,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Pins the intermediate path facts for one owned writable path before the
    /// right-hand side is evaluated.
    ///
    /// `selectors` is ordered by `segments`: every `ArrayIndex` and `MapKey`
    /// consumes the next selector. Dense-field segments consume no selector.
    /// Prepare validates liveness, ownership, and segment shape and resolves
    /// every selector into a concrete [`PinnedWritablePathSegment`]. On error
    /// heap state is unchanged and no observable side effect of the
    /// right-hand side has happened yet.
    fn prepare_writable_path(
        &mut self,
        _root: &ValueSlot,
        _segments: &[VmHeapPathSegment],
        _selectors: &[ValueSlot],
    ) -> Result<WritablePathPreparation, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::PrepareWritablePath,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Atomically applies the pinned writable path and returns the replacement
    /// root slot.
    ///
    /// When every pinned container has exactly one snapshot owner the leaf is
    /// written in place; otherwise the affected containers are cloned
    /// (copy-on-write) and a new root is returned while every alias keeps its
    /// original aggregate unchanged. A failure leaves the old chain and all
    /// owner counts intact. `prepared` is consumed exactly once.
    fn commit_writable_path(
        &mut self,
        _prepared: WritablePathPreparation,
        _value: ValueSlot,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::CommitWritablePath,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Reads one dense record field by canonical field ordinal.
    fn get_dense_field(
        &self,
        _record: &ValueSlot,
        _field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::RecordField,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Physically removes one dense field and consumes the whole record owner.
    ///
    /// This is the narrow primitive used by the verified privileged-affine
    /// projection opcode. On success the selected field becomes the caller's
    /// sole returned owner, the record handle is stale, and every remaining
    /// field has been dropped with the record. On error the record and all of
    /// its fields remain unchanged.
    fn take_dense_field(
        &mut self,
        _record: &ValueSlot,
        _field_ordinal: usize,
    ) -> Result<ValueSlot, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::TakeDenseField,
            kind: ValueKind::RequestHeapRef,
        })
    }

    /// Enumerates the immediate children of one aggregate container in
    /// canonical order for boundary materialization.
    ///
    /// Arrays yield positionally ordered elements with `field == None`;
    /// records yield fields with their canonical name. A non-container value
    /// fails closed without leaking the physical node representation.
    fn container_elements(
        &self,
        _container: &ValueSlot,
    ) -> Result<VmContainerElements, VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ContainerElements,
            kind: ValueKind::RequestHeapRef,
        })
    }
}

#[cfg(test)]
mod tests;

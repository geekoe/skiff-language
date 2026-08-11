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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VmHeapPathSegment {
    DenseField { field: String },
    ArrayIndex,
    MapKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmHeapOperation {
    ValidateLive,
    AllocateArray,
    AllocateMap,
    AllocateRecord,
    AllocateRepresentation,
    ArrayGet,
    ArrayLen,
    MapGet,
    MapLen,
    MapEntryAt,
    RecordField,
    RepresentationPayload,
    ArrayPushOwned,
    MapPutOwned,
    SetWritablePath,
    SnapshotShare,
    TransferOwner,
    ReleaseSnapshot,
    ReleaseResource,
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
/// leave logical ownership, liveness, and share state unchanged.
pub trait VmHeap {
    /// Validates metadata and the liveness/domain of any referenced handle.
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError>;

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
    /// same bits. The caller clears its slot only after success. On error heap
    /// state is unchanged, the caller retains `owner`, and retrying the same
    /// operation is safe.
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

    /// Mutates one already-live owned path transactionally.
    ///
    /// `selectors` is ordered by `segments`: every `ArrayIndex` and `MapKey`
    /// consumes the next selector. Dense-field segments consume no selector.
    fn set_writable_path(
        &mut self,
        _root: &ValueSlot,
        _segments: &[VmHeapPathSegment],
        _selectors: &[ValueSlot],
        _value: ValueSlot,
    ) -> Result<(), VmHeapError> {
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::SetWritablePath,
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
}

#[cfg(test)]
mod tests;

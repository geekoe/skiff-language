//! Heap-neutral value operations used by the bytecode VM.
//!
//! [`ValueSlot`] is `Copy` so the VM can move its fixed-width physical bits
//! efficiently. Copying those bits does not create another semantic owner.
//! The verified VM interprets image-local lifecycle plans and selects one of
//! the synchronous physical primitives below. The heap keeps all generation,
//! domain, share, edit, and affine ownership state private. No artifact plan,
//! native adapter, callback, future, or ownership token crosses this port.

use crate::vm_value::{ValueKind, ValueSlot, VmHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmHeapOperation {
    ValidateLive,
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
}

#[cfg(test)]
mod tests;

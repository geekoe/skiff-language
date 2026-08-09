//! Heap-neutral value operations used by the bytecode VM.
//!
//! [`ValueSlot`] is `Copy` so the VM can move its fixed-width physical bits
//! efficiently. Copying those bits does not create another semantic owner.
//! Verified slot liveness and [`ValueTransferPlanKind`] select the operation,
//! while the heap keeps all generation, domain, share, edit, and affine
//! ownership state private. No ownership token is exposed for callers to forge.

pub use skiff_artifact_model::ValueTransferPlanKind;

use crate::vm_value::{ValueKind, ValueSlot, VmHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VmHeapOperation {
    ValidateLive,
    Snapshot,
    Transfer,
    Drop,
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
    #[error("{operation:?} is not permitted for {kind:?} under {plan:?}")]
    TransferPlanViolation {
        operation: VmHeapOperation,
        plan: ValueTransferPlanKind,
        kind: ValueKind,
    },
    #[error("{kind:?} handle {handle:?} does not own the required affine token")]
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
/// already passed semantic verification.
pub trait VmHeap {
    /// Validates metadata and the liveness/domain of any referenced handle.
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError>;

    /// Produces a logical snapshot or an explicitly cloned lease.
    ///
    /// `SnapshotShare` may return identical slot bits after recording the
    /// semantic share transition. `ExplicitCloneLease` may return a new handle.
    /// `MoveOnly` and `AffineResource` must fail closed on this copy path.
    fn snapshot(
        &mut self,
        value: &ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<ValueSlot, VmHeapError>;

    /// Transfers the logical owner represented by `value`.
    ///
    /// On success the verified VM must mark the source slot dead and clear its
    /// storage as part of the same instruction. Passing this `Copy` type by
    /// value does not perform that source mutation and does not authorize a
    /// second owner. On error the caller retains the original logical owner.
    fn transfer(
        &mut self,
        value: ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<ValueSlot, VmHeapError>;

    /// Executes the linked drop plan for one logical owner.
    ///
    /// Resource release must be exact and idempotent; tracing collection is not
    /// a substitute for this operation. On success the caller must clear the
    /// dropped slot. An error leaves the logical owner with the caller and must
    /// be safe to retry.
    fn drop_value(
        &mut self,
        value: ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<(), VmHeapError>;
}

#[cfg(test)]
mod tests;

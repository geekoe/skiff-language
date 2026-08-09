use std::collections::{HashMap, HashSet};

use super::{VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation};
use crate::{
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};

type PhysicalKey = (ValueKind, VmHandle);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PhysicalState {
    snapshot_owners: usize,
    owner_transfers: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct MutationEvent {
    operation: VmHeapOperation,
    kind: ValueKind,
    handle: Option<VmHandle>,
}

struct FakeHeap {
    domain: u8,
    live: HashMap<PhysicalKey, PhysicalState>,
    released_resources: HashSet<PhysicalKey>,
    events: Vec<MutationEvent>,
    fail_next: Option<VmHeapOperation>,
}

impl FakeHeap {
    fn new(domain: u8) -> Self {
        Self {
            domain,
            live: HashMap::new(),
            released_resources: HashSet::new(),
            events: Vec::new(),
            fail_next: None,
        }
    }

    fn register(&mut self, value: &ValueSlot) {
        if let (Some(kind), Some(handle)) = (value.kind(), value.as_handle()) {
            self.live.insert(
                (kind, handle),
                PhysicalState {
                    snapshot_owners: 1,
                    owner_transfers: 0,
                },
            );
        }
    }

    fn state(&self, value: &ValueSlot) -> Option<PhysicalState> {
        let (kind, Some(handle)) = Self::parts(value).ok()? else {
            return None;
        };
        self.live.get(&(kind, handle)).copied()
    }

    fn arm_failure(&mut self, operation: VmHeapOperation) {
        self.fail_next = Some(operation);
    }

    fn domain_of(handle: VmHandle) -> u8 {
        (handle.get() >> 56) as u8
    }

    fn parts(value: &ValueSlot) -> Result<(ValueKind, Option<VmHandle>), VmHeapError> {
        value
            .kind()
            .map(|kind| (kind, value.as_handle()))
            .ok_or(VmHeapError::InvalidValueMetadata)
    }

    fn validate_identity(&self, kind: ValueKind, handle: VmHandle) -> Result<(), VmHeapError> {
        if Self::domain_of(handle) != self.domain {
            return Err(VmHeapError::InvalidHandle {
                kind,
                handle,
                reason: VmHandleInvalidReason::WrongDomain,
            });
        }
        Ok(())
    }

    fn stale(kind: ValueKind, handle: VmHandle) -> VmHeapError {
        VmHeapError::InvalidHandle {
            kind,
            handle,
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
        }
    }

    fn fail_if_armed(&mut self, operation: VmHeapOperation) -> Result<(), VmHeapError> {
        if self.fail_next == Some(operation) {
            self.fail_next = None;
            return Err(VmHeapError::ResourceLimitExceeded {
                operation,
                limit: 64,
                current: 64,
                requested_delta: 1,
            });
        }
        Ok(())
    }

    fn record(&mut self, operation: VmHeapOperation, kind: ValueKind, handle: Option<VmHandle>) {
        self.events.push(MutationEvent {
            operation,
            kind,
            handle,
        });
    }

    fn release_snapshot_owner(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        let (kind, handle) = Self::parts(owner)?;
        if kind == ValueKind::ResourceRef {
            return Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseSnapshot,
                kind,
            });
        }
        let Some(handle) = handle else {
            return Ok(());
        };
        self.validate_identity(kind, handle)?;
        let key = (kind, handle);
        if !self.live.contains_key(&key) {
            return Err(Self::stale(kind, handle));
        }
        self.fail_if_armed(VmHeapOperation::ReleaseSnapshot)?;
        let remove_entry = {
            let Some(state) = self.live.get_mut(&key) else {
                return Err(VmHeapError::OwnershipViolation { kind, handle });
            };
            state.snapshot_owners -= 1;
            state.snapshot_owners == 0
        };
        if remove_entry {
            self.live.remove(&key);
        }
        self.record(VmHeapOperation::ReleaseSnapshot, kind, Some(handle));
        Ok(())
    }

    fn release_resource_owner(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        let (kind, handle) = Self::parts(owner)?;
        if kind != ValueKind::ResourceRef {
            return Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseResource,
                kind,
            });
        }
        let Some(handle) = handle else {
            return Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ReleaseResource,
                kind,
            });
        };
        self.validate_identity(kind, handle)?;
        let key = (kind, handle);
        if self.released_resources.contains(&key) {
            return Ok(());
        }
        if !self.live.contains_key(&key) {
            return Err(Self::stale(kind, handle));
        }
        self.fail_if_armed(VmHeapOperation::ReleaseResource)?;
        self.live.remove(&key);
        self.released_resources.insert(key);
        self.record(VmHeapOperation::ReleaseResource, kind, Some(handle));
        Ok(())
    }
}

impl VmHeap for FakeHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        let (kind, Some(handle)) = Self::parts(value)? else {
            return Ok(());
        };
        self.validate_identity(kind, handle)?;
        if !self.live.contains_key(&(kind, handle)) {
            return Err(Self::stale(kind, handle));
        }
        Ok(())
    }

    fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        let (kind, handle) = Self::parts(source)?;
        if kind == ValueKind::ResourceRef {
            return Err(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::SnapshotShare,
                kind,
            });
        }
        let Some(handle) = handle else {
            return Ok(*source);
        };
        self.fail_if_armed(VmHeapOperation::SnapshotShare)?;
        let Some(state) = self.live.get_mut(&(kind, handle)) else {
            return Err(VmHeapError::OwnershipViolation { kind, handle });
        };
        state.snapshot_owners += 1;
        self.record(VmHeapOperation::SnapshotShare, kind, Some(handle));
        Ok(*source)
    }

    fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(source)?;
        let (kind, handle) = Self::parts(source)?;
        let Some(handle) = handle else {
            return Ok(*source);
        };
        self.fail_if_armed(VmHeapOperation::TransferOwner)?;
        let Some(state) = self.live.get_mut(&(kind, handle)) else {
            return Err(VmHeapError::OwnershipViolation { kind, handle });
        };
        state.owner_transfers += 1;
        self.record(VmHeapOperation::TransferOwner, kind, Some(handle));
        Ok(*source)
    }

    fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_snapshot_owner(owner)
    }

    fn release_resource(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
        self.release_resource_owner(owner)
    }
}

fn handle(domain: u8, index: u64) -> VmHandle {
    VmHandle::new((u64::from(domain) << 56) | index)
}

fn request_ref(domain: u8, index: u64) -> ValueSlot {
    ValueSlot::request_heap_ref(
        handle(domain, index),
        CompactTypeTag::new(17),
        ValueFlags::new(3),
    )
}

fn resource_ref(domain: u8, index: u64) -> ValueSlot {
    ValueSlot::resource_ref(
        handle(domain, index),
        CompactTypeTag::new(21),
        ValueFlags::new(1),
    )
}

#[test]
fn live_validation_rejects_foreign_and_stale_handles() {
    let live = request_ref(4, 1);
    let foreign = request_ref(5, 1);
    let stale = request_ref(4, 2);
    let mut heap = FakeHeap::new(4);
    heap.register(&live);

    assert_eq!(heap.validate_live(&live), Ok(()));
    assert_eq!(heap.validate_live(&ValueSlot::integer(3)), Ok(()));
    assert_eq!(
        heap.validate_live(&foreign),
        Err(VmHeapError::InvalidHandle {
            kind: ValueKind::RequestHeapRef,
            handle: handle(5, 1),
            reason: VmHandleInvalidReason::WrongDomain,
        })
    );
    assert_eq!(
        heap.validate_live(&stale),
        Err(VmHeapError::InvalidHandle {
            kind: ValueKind::RequestHeapRef,
            handle: handle(4, 2),
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
        })
    );
}

#[test]
fn physical_mutations_reject_invalid_handles_without_state_change() {
    let live = request_ref(4, 1);
    let foreign = request_ref(5, 1);
    let stale = request_ref(4, 2);
    let foreign_resource = resource_ref(5, 3);
    let mut heap = FakeHeap::new(4);
    heap.register(&live);
    let original = heap.state(&live);

    assert!(matches!(
        heap.snapshot_share(&foreign),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));
    assert!(matches!(
        heap.transfer_owner(&stale),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
    assert!(matches!(
        heap.release_snapshot(&foreign),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));
    assert!(matches!(
        heap.release_resource(&foreign_resource),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::WrongDomain,
            ..
        })
    ));
    assert_eq!(heap.state(&live), original);
    assert!(heap.events.is_empty());
}

#[test]
fn snapshot_share_and_transfer_owner_commit_physical_mutations() {
    let value = request_ref(7, 9);
    let mut heap = FakeHeap::new(7);
    heap.register(&value);

    let snapshot = heap.snapshot_share(&value);
    assert!(matches!(snapshot, Ok(slot) if slot == value));
    let state = heap.state(&value);
    assert!(state.is_some_and(|state| { state.snapshot_owners == 2 && state.owner_transfers == 0 }));

    let transferred = heap.transfer_owner(&value);
    assert!(matches!(transferred, Ok(slot) if slot == value));
    let state = heap.state(&value);
    assert!(state.is_some_and(|state| { state.snapshot_owners == 2 && state.owner_transfers == 1 }));
    assert_eq!(heap.events.len(), 2);
    assert_eq!(heap.events[0].operation, VmHeapOperation::SnapshotShare);
    assert_eq!(heap.events[0].kind, ValueKind::RequestHeapRef);
    assert_eq!(heap.events[0].handle, Some(handle(7, 9)));
    assert_eq!(heap.events[1].operation, VmHeapOperation::TransferOwner);
}

#[test]
fn physical_primitives_reject_wrong_kinds_without_adapter_fallback() {
    let value = request_ref(6, 1);
    let resource = resource_ref(6, 2);
    let mut heap = FakeHeap::new(6);
    heap.register(&value);
    heap.register(&resource);

    assert!(matches!(
        heap.snapshot_share(&resource),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::SnapshotShare,
            kind: ValueKind::ResourceRef,
        })
    ));
    assert_eq!(
        heap.release_snapshot(&resource),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseSnapshot,
            kind: ValueKind::ResourceRef,
        })
    );
    assert_eq!(
        heap.release_resource(&value),
        Err(VmHeapError::OperationKindMismatch {
            operation: VmHeapOperation::ReleaseResource,
            kind: ValueKind::RequestHeapRef,
        })
    );
    assert!(heap.events.is_empty());
}

#[test]
fn failed_mutations_leave_owner_and_physical_state_unchanged() {
    let value = request_ref(9, 1);
    let mut heap = FakeHeap::new(9);
    heap.register(&value);
    let original = heap.state(&value);

    heap.arm_failure(VmHeapOperation::SnapshotShare);
    assert!(matches!(
        heap.snapshot_share(&value),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: VmHeapOperation::SnapshotShare,
            ..
        })
    ));
    assert_eq!(heap.state(&value), original);
    assert_eq!(heap.validate_live(&value), Ok(()));

    heap.arm_failure(VmHeapOperation::TransferOwner);
    assert!(matches!(
        heap.transfer_owner(&value),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: VmHeapOperation::TransferOwner,
            ..
        })
    ));
    assert_eq!(heap.state(&value), original);
    assert_eq!(heap.validate_live(&value), Ok(()));
    assert!(heap.events.is_empty());
}

#[test]
fn snapshot_release_counts_owners_and_failed_release_is_retryable() {
    let value = request_ref(3, 10);
    let mut heap = FakeHeap::new(3);
    heap.register(&value);
    assert_eq!(heap.snapshot_share(&value), Ok(value));

    heap.arm_failure(VmHeapOperation::ReleaseSnapshot);
    assert!(matches!(
        heap.release_snapshot(&value),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: VmHeapOperation::ReleaseSnapshot,
            ..
        })
    ));
    assert_eq!(heap.validate_live(&value), Ok(()));
    assert_eq!(heap.release_snapshot(&value), Ok(()));
    assert_eq!(heap.validate_live(&value), Ok(()));
    assert_eq!(heap.release_snapshot(&value), Ok(()));
    assert!(matches!(
        heap.validate_live(&value),
        Err(VmHeapError::InvalidHandle {
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            ..
        })
    ));
    assert_eq!(
        heap.events
            .iter()
            .filter(|event| event.operation == VmHeapOperation::ReleaseSnapshot)
            .count(),
        2
    );
}

#[test]
fn resource_release_is_exact_and_idempotent() {
    let resource = resource_ref(3, 11);
    let mut heap = FakeHeap::new(3);
    heap.register(&resource);

    heap.arm_failure(VmHeapOperation::ReleaseResource);
    assert!(matches!(
        heap.release_resource(&resource),
        Err(VmHeapError::ResourceLimitExceeded {
            operation: VmHeapOperation::ReleaseResource,
            ..
        })
    ));
    assert_eq!(heap.validate_live(&resource), Ok(()));
    assert!(heap.events.is_empty());

    assert_eq!(heap.release_resource(&resource), Ok(()));
    assert_eq!(heap.release_resource(&resource), Ok(()));
    assert_eq!(
        heap.events
            .iter()
            .filter(|event| event.operation == VmHeapOperation::ReleaseResource)
            .count(),
        1
    );
}

struct FakeRoots(Vec<ValueSlot>);

impl VmRootSource for FakeRoots {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for root in &self.0 {
            visitor.visit_root(root)?;
        }
        Ok(())
    }
}

struct ValidatingVisitor<'heap> {
    heap: &'heap FakeHeap,
    visited: Vec<ValueSlot>,
}

impl VmRootVisitor for ValidatingVisitor<'_> {
    fn visit_root(&mut self, root: &ValueSlot) -> Result<(), VmHeapError> {
        self.heap.validate_live(root)?;
        self.visited.push(*root);
        Ok(())
    }
}

fn walk_roots(
    source: &dyn VmRootSource,
    visitor: &mut dyn VmRootVisitor,
) -> Result<(), VmHeapError> {
    source.visit_roots(visitor)
}

#[test]
fn root_sources_preserve_value_slot_boundaries_and_propagate_errors() {
    let live = request_ref(8, 1);
    let stale = request_ref(8, 2);
    let mut heap = FakeHeap::new(8);
    heap.register(&live);

    let roots = FakeRoots(vec![ValueSlot::bool(true), live]);
    let mut visitor = ValidatingVisitor {
        heap: &heap,
        visited: Vec::new(),
    };
    assert_eq!(walk_roots(&roots, &mut visitor), Ok(()));
    assert_eq!(visitor.visited, roots.0);

    let stale_roots = FakeRoots(vec![stale]);
    assert_eq!(
        walk_roots(&stale_roots, &mut visitor),
        Err(VmHeapError::InvalidHandle {
            kind: ValueKind::RequestHeapRef,
            handle: handle(8, 2),
            reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
        })
    );
}

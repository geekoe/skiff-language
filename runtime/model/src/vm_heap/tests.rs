use std::collections::HashSet;

use super::{ValueTransferPlanKind, VmHandleInvalidReason, VmHeap, VmHeapError, VmHeapOperation};
use crate::{
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
};

#[derive(Debug, PartialEq, Eq)]
struct TransferEvent {
    operation: VmHeapOperation,
    plan: ValueTransferPlanKind,
    kind: ValueKind,
    handle: Option<VmHandle>,
}

struct FakeHeap {
    domain: u8,
    live: HashSet<(ValueKind, VmHandle)>,
    dropped: HashSet<(ValueKind, VmHandle)>,
    events: Vec<TransferEvent>,
    release_count: usize,
}

impl FakeHeap {
    fn new(domain: u8) -> Self {
        Self {
            domain,
            live: HashSet::new(),
            dropped: HashSet::new(),
            events: Vec::new(),
            release_count: 0,
        }
    }

    fn register(&mut self, value: &ValueSlot) {
        if let (Some(kind), Some(handle)) = (value.kind(), value.as_handle()) {
            self.live.insert((kind, handle));
        }
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

    fn record(
        &mut self,
        operation: VmHeapOperation,
        plan: ValueTransferPlanKind,
        value: &ValueSlot,
    ) -> Result<(), VmHeapError> {
        let (kind, handle) = Self::parts(value)?;
        self.events.push(TransferEvent {
            operation,
            plan,
            kind,
            handle,
        });
        Ok(())
    }
}

impl VmHeap for FakeHeap {
    fn validate_live(&self, value: &ValueSlot) -> Result<(), VmHeapError> {
        let (kind, Some(handle)) = Self::parts(value)? else {
            return Ok(());
        };
        if Self::domain_of(handle) != self.domain {
            return Err(VmHeapError::InvalidHandle {
                kind,
                handle,
                reason: VmHandleInvalidReason::WrongDomain,
            });
        }
        if !self.live.contains(&(kind, handle)) {
            return Err(VmHeapError::InvalidHandle {
                kind,
                handle,
                reason: VmHandleInvalidReason::StaleGenerationOrEpoch,
            });
        }
        Ok(())
    }

    fn snapshot(
        &mut self,
        value: &ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(value)?;
        let (kind, _) = Self::parts(value)?;
        match plan {
            ValueTransferPlanKind::SnapshotShare | ValueTransferPlanKind::ExplicitCloneLease => {
                self.record(VmHeapOperation::Snapshot, plan, value)?;
                Ok(*value)
            }
            ValueTransferPlanKind::MoveOnly | ValueTransferPlanKind::AffineResource => {
                Err(VmHeapError::TransferPlanViolation {
                    operation: VmHeapOperation::Snapshot,
                    plan,
                    kind,
                })
            }
        }
    }

    fn transfer(
        &mut self,
        value: ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<ValueSlot, VmHeapError> {
        self.validate_live(&value)?;
        self.record(VmHeapOperation::Transfer, plan, &value)?;
        Ok(value)
    }

    fn drop_value(
        &mut self,
        value: ValueSlot,
        plan: ValueTransferPlanKind,
    ) -> Result<(), VmHeapError> {
        let (kind, handle) = Self::parts(&value)?;
        let Some(handle) = handle else {
            self.record(VmHeapOperation::Drop, plan, &value)?;
            return Ok(());
        };
        if self.dropped.contains(&(kind, handle)) {
            return Ok(());
        }
        self.validate_live(&value)?;
        self.live.remove(&(kind, handle));
        self.dropped.insert((kind, handle));
        self.release_count += 1;
        self.record(VmHeapOperation::Drop, plan, &value)
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
fn snapshot_and_transfer_follow_the_linked_plan() {
    let value = request_ref(7, 9);
    let mut heap = FakeHeap::new(7);
    heap.register(&value);

    let snapshot = heap.snapshot(&value, ValueTransferPlanKind::SnapshotShare);
    assert!(matches!(snapshot, Ok(slot) if slot == value));
    let clone_lease = heap.snapshot(&value, ValueTransferPlanKind::ExplicitCloneLease);
    assert!(matches!(clone_lease, Ok(slot) if slot == value));
    for plan in [
        ValueTransferPlanKind::MoveOnly,
        ValueTransferPlanKind::AffineResource,
    ] {
        let rejected = heap.snapshot(&value, plan);
        assert!(matches!(
            rejected,
            Err(VmHeapError::TransferPlanViolation {
                operation: VmHeapOperation::Snapshot,
                plan: rejected_plan,
                kind: ValueKind::RequestHeapRef,
            }) if rejected_plan == plan
        ));
    }

    let moved = heap.transfer(value, ValueTransferPlanKind::AffineResource);
    assert!(matches!(moved, Ok(slot) if slot == value));
    assert_eq!(heap.events.len(), 3);
    assert_eq!(heap.events[0].operation, VmHeapOperation::Snapshot);
    assert_eq!(
        heap.events[1].plan,
        ValueTransferPlanKind::ExplicitCloneLease
    );
    assert_eq!(heap.events[2].operation, VmHeapOperation::Transfer);
}

#[test]
fn explicit_drop_is_exact_and_idempotent() {
    let value = ValueSlot::resource_ref(handle(3, 11), CompactTypeTag::new(21), ValueFlags::new(1));
    let mut heap = FakeHeap::new(3);
    heap.register(&value);

    assert_eq!(
        heap.drop_value(value, ValueTransferPlanKind::AffineResource),
        Ok(())
    );
    assert_eq!(
        heap.drop_value(value, ValueTransferPlanKind::AffineResource),
        Ok(())
    );
    assert_eq!(heap.release_count, 1);
    assert_eq!(heap.events.len(), 1);
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
fn root_sources_are_object_safe_and_propagate_validation_errors() {
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
    assert!(visitor.visited == roots.0);

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

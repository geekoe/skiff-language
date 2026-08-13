//! The single linked-plan lifecycle executor for the synchronous VM core.
//!
//! Every slot and operand-stack transition that moves, shares, drops, or
//! overwrites a value must pass through [`LifecycleExecutor`]. It is the only
//! consumer of the heap primitives `snapshot_share`, `transfer_owner`,
//! `release_snapshot`, and `release_resource`; nothing in `fiber.rs` calls
//! them directly and no post-hoc frame reconciliation remains. Each transition
//! selects its physical primitive from the exact
//! [`LinkedValueTransferPlan`] carried by the linked frame or program point.
//!
//! Unsupported plans fail closed instead of falling back to a type guess:
//! a plan that requires a native clone/drop adapter or a recursive resource
//! shape outside the declared support surface is [`LifecycleError::PlanUnavailable`].

use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedResourceDropPlan, LinkedValueDropPlan,
    LinkedValueTransferPlan,
};
use skiff_runtime_model::{
    vm_heap::VmHeap,
    vm_value::ValueSlot,
};

use crate::VmError;
use skiff_artifact_model::Opcode;

/// Executor-local failure that the fiber maps onto its closed [`crate::VmError`].
pub(crate) enum LifecycleError {
    /// The heap rejected the physical primitive without changing logical
    /// ownership/share state; the caller retains its owner.
    Heap(skiff_runtime_model::vm_heap::VmHeapError),
    /// The linked plan has no supported physical primitive.
    PlanUnavailable,
}

impl LifecycleError {
    /// Projects the executor failure onto the closed VM error surface with the
    /// exact dispatch site. Heap errors are passed through unchanged; an
    /// unsupported plan is the typed `FullValueLifecyclePlanUnavailable` gate.
    pub(crate) fn into_vm_error(
        self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> VmError {
        match self {
            Self::Heap(error) => VmError::Heap(error),
            Self::PlanUnavailable => VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            },
        }
    }
}

/// The unique lifecycle executor for one dispatch step.
pub(crate) struct LifecycleExecutor<'heap> {
    heap: &'heap mut dyn VmHeap,
}

impl<'heap> LifecycleExecutor<'heap> {
    pub(crate) fn new(heap: &'heap mut dyn VmHeap) -> Self {
        Self { heap }
    }

    /// Direct heap access for non-lifecycle operations such as allocation and
    /// aggregate reads. Slot transitions must not bypass this executor through
    /// the returned port.
    pub(crate) fn heap(&mut self) -> &mut dyn VmHeap {
        self.heap
    }

    /// Creates a second snapshot owner. Only `SnapshotShare` plans are
    /// shareable; move-only and resource plans fail closed.
    pub(crate) fn share(
        &mut self,
        source: &ValueSlot,
        plan: &LinkedValueTransferPlan,
    ) -> Result<ValueSlot, LifecycleError> {
        match plan {
            LinkedValueTransferPlan::SnapshotShare { .. } => self
                .heap
                .snapshot_share(source)
                .map_err(LifecycleError::Heap),
            _ => Err(LifecycleError::PlanUnavailable),
        }
    }

    /// Moves one logical owner. Snapshot, move-only, and affine-resource plans
    /// all relocate through `transfer_owner`; an explicit clone lease has no
    /// supported physical move without its adapter.
    pub(crate) fn transfer(
        &mut self,
        source: &ValueSlot,
        plan: &LinkedValueTransferPlan,
    ) -> Result<ValueSlot, LifecycleError> {
        match plan {
            LinkedValueTransferPlan::SnapshotShare { .. }
            | LinkedValueTransferPlan::MoveOnly { .. }
            | LinkedValueTransferPlan::AffineResource { .. } => self
                .heap
                .transfer_owner(source)
                .map_err(LifecycleError::Heap),
            LinkedValueTransferPlan::ExplicitCloneLease { .. } => {
                Err(LifecycleError::PlanUnavailable)
            }
        }
    }

    /// Releases exactly one logical owner according to the plan's drop role.
    pub(crate) fn release(
        &mut self,
        owner: &ValueSlot,
        plan: &LinkedValueTransferPlan,
    ) -> Result<(), LifecycleError> {
        match plan {
            LinkedValueTransferPlan::SnapshotShare { drop }
            | LinkedValueTransferPlan::MoveOnly { drop } => self.release_value(owner, drop),
            LinkedValueTransferPlan::AffineResource { drop } => {
                self.release_resource(owner, drop)
            }
            LinkedValueTransferPlan::ExplicitCloneLease { drop, .. } => {
                self.release_resource(owner, drop)
            }
        }
    }

    fn release_value(
        &mut self,
        owner: &ValueSlot,
        drop: &LinkedValueDropPlan,
    ) -> Result<(), LifecycleError> {
        match drop {
            LinkedValueDropPlan::Trivial
            | LinkedValueDropPlan::SnapshotRelease
            | LinkedValueDropPlan::RecursiveShape { .. } => self
                .heap
                .release_snapshot(owner)
                .map_err(LifecycleError::Heap),
            LinkedValueDropPlan::NativeAdapter { .. } => Err(LifecycleError::PlanUnavailable),
        }
    }

    fn release_resource(
        &mut self,
        owner: &ValueSlot,
        drop: &LinkedResourceDropPlan,
    ) -> Result<(), LifecycleError> {
        match drop {
            LinkedResourceDropPlan::ResourceTableRelease => self
                .heap
                .release_resource(owner)
                .map_err(LifecycleError::Heap),
            LinkedResourceDropPlan::RecursiveShape { .. }
            | LinkedResourceDropPlan::NativeAdapter { .. } => Err(LifecycleError::PlanUnavailable),
        }
    }
}

#[cfg(test)]
mod tests {
    use skiff_runtime_model::{
        vm_heap::{VmHeap, VmHeapError, VmHeapOperation},
        vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot, VmHandle},
    };

    use super::{LifecycleError, LifecycleExecutor};
    use skiff_runtime_linked_bytecode::{
        LinkedResourceDropPlan, LinkedValueDropPlan, LinkedValueTransferPlan,
    };

    #[derive(Default)]
    struct RecordingHeap {
        shares: usize,
        transfers: usize,
        snapshot_releases: usize,
        resource_releases: usize,
    }

    impl VmHeap for RecordingHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            self.shares += 1;
            Ok(*source)
        }

        fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            self.transfers += 1;
            Ok(*source)
        }

        fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            self.snapshot_releases += 1;
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            self.resource_releases += 1;
            Ok(())
        }
    }

    fn record() -> ValueSlot {
        ValueSlot::request_heap_ref(
            VmHandle::new(1),
            CompactTypeTag::new(7),
            ValueFlags::new(0),
        )
    }

    fn resource() -> ValueSlot {
        ValueSlot::resource_ref(VmHandle::new(2), CompactTypeTag::new(8), ValueFlags::new(0))
    }

    #[test]
    fn lifecycle_executor_selects_the_physical_primitive_per_plan() {
        let mut heap = RecordingHeap::default();
        let mut executor = LifecycleExecutor::new(&mut heap);
        let record = record();

        let snapshot = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        };
        let moved = LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::SnapshotRelease,
        };
        let affine = LinkedValueTransferPlan::AffineResource {
            drop: LinkedResourceDropPlan::ResourceTableRelease,
        };

        assert!(executor.share(&record, &snapshot).is_ok());
        assert!(executor.transfer(&record, &moved).is_ok());
        assert!(executor.release(&record, &snapshot).is_ok());
        assert!(executor.release(&record, &moved).is_ok());
        assert!(executor.release(&resource(), &affine).is_ok());

        assert_eq!(heap.shares, 1);
        assert_eq!(heap.transfers, 1);
        assert_eq!(heap.snapshot_releases, 2);
        assert_eq!(heap.resource_releases, 1);
    }

    #[test]
    fn lifecycle_executor_fails_closed_for_unsupported_plan_roles() {
        let mut heap = RecordingHeap::default();
        let mut executor = LifecycleExecutor::new(&mut heap);
        let record = record();

        let native_value_drop = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::NativeAdapter {
                adapter: skiff_artifact_model::NativeValueLifecycleAdapter {
                    binding_key: "native.drop".to_string(),
                    role: skiff_artifact_model::NativeValueAdapterRole::ValueDrop,
                    abi_version: 0,
                },
            },
        };
        let clone_lease = LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter: skiff_artifact_model::NativeValueLifecycleAdapter {
                binding_key: "native.clone".to_string(),
                role: skiff_artifact_model::NativeValueAdapterRole::CloneLease,
                abi_version: 0,
            },
            drop: LinkedResourceDropPlan::NativeAdapter {
                adapter: skiff_artifact_model::NativeValueLifecycleAdapter {
                    binding_key: "native.resource.drop".to_string(),
                    role: skiff_artifact_model::NativeValueAdapterRole::ResourceDrop,
                    abi_version: 0,
                },
            },
        };
        let recursive_resource = LinkedValueTransferPlan::AffineResource {
            drop: LinkedResourceDropPlan::RecursiveShape {
                shape: skiff_runtime_linked_bytecode::ShapeIndex::new(1),
            },
        };

        assert!(matches!(
            executor.share(&record, &LinkedValueTransferPlan::MoveOnly {
                drop: LinkedValueDropPlan::Trivial,
            }),
            Err(LifecycleError::PlanUnavailable)
        ));
        assert!(matches!(
            executor.release(&record, &native_value_drop),
            Err(LifecycleError::PlanUnavailable)
        ));
        assert!(matches!(
            executor.release(&record, &clone_lease),
            Err(LifecycleError::PlanUnavailable)
        ));
        assert!(matches!(
            executor.release(&record, &recursive_resource),
            Err(LifecycleError::PlanUnavailable)
        ));
        assert_eq!(heap.shares, 0);
        assert_eq!(heap.transfers, 0);
        assert_eq!(heap.snapshot_releases, 0);
        assert_eq!(heap.resource_releases, 0);
    }

    #[test]
    fn lifecycle_executor_propagates_heap_errors_without_observation() {
        struct FailingHeap;

        impl VmHeap for FailingHeap {
            fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
                Ok(())
            }

            fn snapshot_share(
                &mut self,
                _source: &ValueSlot,
            ) -> Result<ValueSlot, VmHeapError> {
                Err(VmHeapError::OperationKindMismatch {
                    operation: VmHeapOperation::SnapshotShare,
                    kind: ValueKind::RequestHeapRef,
                })
            }

            fn transfer_owner(
                &mut self,
                _source: &ValueSlot,
            ) -> Result<ValueSlot, VmHeapError> {
                Ok(*_source)
            }

            fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
                Ok(())
            }

            fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
                Ok(())
            }
        }

        let mut heap = FailingHeap;
        let mut executor = LifecycleExecutor::new(&mut heap);
        let snapshot = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        };
        assert!(matches!(
            executor.share(&record(), &snapshot),
            Err(LifecycleError::Heap(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::SnapshotShare,
                ..
            }))
        ));
    }
}

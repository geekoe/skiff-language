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
use skiff_runtime_model::{vm_heap::VmHeap, vm_value::ValueSlot};

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
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            } => Ok(*source),
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
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            }
            | LinkedValueTransferPlan::MoveOnly {
                drop: LinkedValueDropPlan::Trivial,
            } => Ok(*source),
            LinkedValueTransferPlan::SnapshotShare { .. }
            | LinkedValueTransferPlan::MoveOnly { .. } => self
                .heap
                .transfer_owner(source)
                .map_err(LifecycleError::Heap),
            LinkedValueTransferPlan::AffineResource {
                drop: LinkedResourceDropPlan::ResourceTableRelease,
            } => self
                .heap
                .transfer_owner(source)
                .map_err(LifecycleError::Heap),
            LinkedValueTransferPlan::AffineResource { .. } => Err(LifecycleError::PlanUnavailable),
            LinkedValueTransferPlan::ExplicitCloneLease { .. } => {
                Err(LifecycleError::PlanUnavailable)
            }
        }
    }

    /// Whether one linked plan has a synchronous physical owner-move in this
    /// executor. Multi-value adopters preflight the complete batch before the
    /// first transfer so a later unsupported plan cannot strand earlier
    /// owners between storage and the adopting container.
    pub(crate) const fn supports_transfer(plan: &LinkedValueTransferPlan) -> bool {
        match plan {
            LinkedValueTransferPlan::SnapshotShare { .. }
            | LinkedValueTransferPlan::MoveOnly { .. } => true,
            LinkedValueTransferPlan::AffineResource { drop } => {
                matches!(drop, LinkedResourceDropPlan::ResourceTableRelease)
            }
            LinkedValueTransferPlan::ExplicitCloneLease { .. } => false,
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
            LinkedValueTransferPlan::AffineResource { drop } => self.release_resource(owner, drop),
            LinkedValueTransferPlan::ExplicitCloneLease { drop, .. } => {
                self.release_resource(owner, drop)
            }
        }
    }

    /// Whether the synchronous core has a physical release primitive for the
    /// complete linked plan. Callers that release a batch use this preflight
    /// before the first mutation, so an unsupported later plan cannot leave a
    /// partially consumed batch.
    pub(crate) const fn supports_release(plan: &LinkedValueTransferPlan) -> bool {
        match plan {
            LinkedValueTransferPlan::SnapshotShare { drop }
            | LinkedValueTransferPlan::MoveOnly { drop } => matches!(
                drop,
                LinkedValueDropPlan::Trivial
                    | LinkedValueDropPlan::SnapshotRelease
                    | LinkedValueDropPlan::RecursiveShape { .. }
            ),
            LinkedValueTransferPlan::AffineResource { drop }
            | LinkedValueTransferPlan::ExplicitCloneLease { drop, .. } => {
                matches!(drop, LinkedResourceDropPlan::ResourceTableRelease)
            }
        }
    }

    pub(crate) fn release_batch(
        &mut self,
        owners: &[ValueSlot],
        plans: &[LinkedValueTransferPlan],
    ) -> Result<(), LifecycleError> {
        if owners.len() != plans.len() || !plans.iter().all(Self::supports_release) {
            return Err(LifecycleError::PlanUnavailable);
        }
        for (owner, plan) in owners.iter().zip(plans) {
            self.release(owner, plan)?;
        }
        Ok(())
    }

    fn release_value(
        &mut self,
        owner: &ValueSlot,
        drop: &LinkedValueDropPlan,
    ) -> Result<(), LifecycleError> {
        match drop {
            LinkedValueDropPlan::Trivial => Ok(()),
            LinkedValueDropPlan::SnapshotRelease | LinkedValueDropPlan::RecursiveShape { .. } => {
                self.heap
                    .release_snapshot(owner)
                    .map_err(LifecycleError::Heap)
            }
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

    fn tag(type_index: u32) -> CompactTypeTag {
        CompactTypeTag::try_from_type_index(type_index)
            .expect("test type index must fit compact tag")
    }

    fn record() -> ValueSlot {
        ValueSlot::request_heap_ref(VmHandle::new(1), tag(7), ValueFlags::new(0))
    }

    fn resource() -> ValueSlot {
        ValueSlot::resource_ref(VmHandle::new(2), tag(8), ValueFlags::new(0))
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
            executor.share(
                &record,
                &LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::Trivial,
                }
            ),
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

            fn snapshot_share(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
                Err(VmHeapError::OperationKindMismatch {
                    operation: VmHeapOperation::SnapshotShare,
                    kind: ValueKind::RequestHeapRef,
                })
            }

            fn transfer_owner(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
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
            drop: LinkedValueDropPlan::SnapshotRelease,
        };
        assert!(matches!(
            executor.share(&record(), &snapshot),
            Err(LifecycleError::Heap(VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::SnapshotShare,
                ..
            }))
        ));
    }

    #[test]
    fn trivial_plans_take_the_sidecar_free_fast_path() {
        let mut heap = RecordingHeap::default();
        let mut executor = LifecycleExecutor::new(&mut heap);
        let scalar = ValueSlot::number(1.0);
        let trivial_share = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        };
        let trivial_move = LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::Trivial,
        };

        assert!(matches!(
            executor.share(&scalar, &trivial_share),
            Ok(value) if value == scalar
        ));
        assert!(matches!(
            executor.transfer(&scalar, &trivial_move),
            Ok(value) if value == scalar
        ));
        assert!(executor.release(&scalar, &trivial_share).is_ok());
        assert!(executor.release(&scalar, &trivial_move).is_ok());

        // Immediate scalars keep the Phase 1 sidecar-free invariant: no heap
        // primitive is ever invoked for a trivial plan.
        assert_eq!(heap.shares, 0);
        assert_eq!(heap.transfers, 0);
        assert_eq!(heap.snapshot_releases, 0);
        assert_eq!(heap.resource_releases, 0);
    }

    #[test]
    fn phase_5_first_poll_http_arguments_preflight_all_plans_before_release() {
        let mut heap = RecordingHeap::default();
        let mut executor = LifecycleExecutor::new(&mut heap);
        let owners = [ValueSlot::integer(1), ValueSlot::integer(2)];
        let plans = [
            LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            },
            LinkedValueTransferPlan::AffineResource {
                drop: LinkedResourceDropPlan::RecursiveShape {
                    shape: skiff_runtime_linked_bytecode::ShapeIndex::new(1),
                },
            },
        ];

        assert!(matches!(
            executor.release_batch(&owners, &plans),
            Err(LifecycleError::PlanUnavailable)
        ));
        assert_eq!(heap.snapshot_releases, 0);
        assert_eq!(heap.resource_releases, 0);
    }

    #[test]
    fn phase_5_first_poll_http_arguments_middle_failure_is_not_retried() {
        #[derive(Default)]
        struct MiddleFailingHeap {
            attempts: Vec<i64>,
        }

        impl VmHeap for MiddleFailingHeap {
            fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
                Ok(())
            }

            fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
                Ok(*source)
            }

            fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
                Ok(*source)
            }

            fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
                let value = owner
                    .as_integer()
                    .ok_or(VmHeapError::InvalidValueMetadata)?;
                self.attempts.push(value);
                if value == 2 {
                    return Err(VmHeapError::OperationKindMismatch {
                        operation: VmHeapOperation::ReleaseSnapshot,
                        kind: ValueKind::Integer,
                    });
                }
                Ok(())
            }

            fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
                Ok(())
            }
        }

        let mut heap = MiddleFailingHeap::default();
        let mut executor = LifecycleExecutor::new(&mut heap);
        let owners = [
            ValueSlot::integer(1),
            ValueSlot::integer(2),
            ValueSlot::integer(3),
        ];
        let plan = LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        };
        let plans = [plan.clone(), plan.clone(), plan];

        assert!(matches!(
            executor.release_batch(&owners, &plans),
            Err(LifecycleError::Heap(_))
        ));
        assert_eq!(heap.attempts, [1, 2]);
    }
}

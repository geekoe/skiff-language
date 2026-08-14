use skiff_artifact_model::{NativeValueLifecycleAdapter, ValueTransferPlanKind};

use crate::ShapeIndex;

/// Concrete drop behavior for a snapshot or move-only value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedValueDropPlan {
    Trivial,
    SnapshotRelease,
    RecursiveShape {
        shape: ShapeIndex,
    },
    NativeAdapter {
        adapter: NativeValueLifecycleAdapter,
    },
}

/// Concrete drop behavior for an affine resource or cloneable lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedResourceDropPlan {
    ResourceTableRelease,
    RecursiveShape {
        shape: ShapeIndex,
    },
    NativeAdapter {
        adapter: NativeValueLifecycleAdapter,
    },
}

/// Complete concrete lifecycle plan carried by linked candidate facts.
///
/// Artifact `ValueTransferPlan::FromType` deliberately has no representation
/// here. A linker must resolve it before constructing a candidate; there is no
/// conversion or unchecked constructor that can retain the expression.
/// Native adapters remain the exact artifact-model DTO; the candidate does
/// not create a parallel registry identity or role vocabulary.
///
/// ```compile_fail
/// use skiff_artifact_model::{TypeRefIr, ValueTransferPlan};
/// use skiff_runtime_linked_bytecode::LinkedValueTransferPlan;
///
/// let unresolved = ValueTransferPlan::FromType {
///     ty: TypeRefIr::Builtin {
///         name: "string".to_string(),
///         args: Vec::new(),
///     },
/// };
/// let _: LinkedValueTransferPlan = unresolved;
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkedValueTransferPlan {
    SnapshotShare {
        drop: LinkedValueDropPlan,
    },
    MoveOnly {
        drop: LinkedValueDropPlan,
    },
    AffineResource {
        drop: LinkedResourceDropPlan,
    },
    ExplicitCloneLease {
        clone_adapter: NativeValueLifecycleAdapter,
        drop: LinkedResourceDropPlan,
    },
}

impl LinkedValueTransferPlan {
    /// Coarse diagnostic category only. Execution must use the complete plan,
    /// including the exact drop/clone adapter role and ABI.
    pub const fn kind(&self) -> ValueTransferPlanKind {
        match self {
            Self::SnapshotShare { .. } => ValueTransferPlanKind::SnapshotShare,
            Self::MoveOnly { .. } => ValueTransferPlanKind::MoveOnly,
            Self::AffineResource { .. } => ValueTransferPlanKind::AffineResource,
            Self::ExplicitCloneLease { .. } => ValueTransferPlanKind::ExplicitCloneLease,
        }
    }
}

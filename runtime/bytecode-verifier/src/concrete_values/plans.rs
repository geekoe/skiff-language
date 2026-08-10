mod data;
mod functions;
mod signatures;

use skiff_artifact_model::{
    NativeResourceDropPlan, NativeValueDropPlan, NativeValueLifecycleConcrete,
};
use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedResourceDropPlan, LinkedValueDropPlan,
    LinkedValueTransferPlan, TypeIndex,
};

use super::ConcreteValueFacts;
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Checks every candidate-declared lifecycle plan against concrete type facts.
pub(super) fn prove_declared_plans(
    candidate: &LinkedBytecodeCandidate,
    facts: &ConcreteValueFacts,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    functions::prove_function_plans(candidate, facts)?;
    signatures::prove_signature_plans(candidate, facts)?;
    data::prove_data_plans(candidate, facts)
}

pub(super) fn prove_position(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    declared: &LinkedValueTransferPlan,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let position = position.as_ref();
    let fact = usize::try_from(ty.get())
        .ok()
        .and_then(|index| facts.types.get(index))
        .ok_or_else(|| {
            semantic_violation(
                location,
                format!(
                    "{position} references type {} without a dense concrete value fact",
                    ty.get()
                ),
            )
        })?;

    if let Some(shape) = recursive_shape(declared) {
        return Err(semantic_violation(
            location,
            format!(
                "{position} for type {} declares recursive-shape drop {} instead of an independently classified native lifecycle plan",
                ty.get(),
                shape
            ),
        ));
    }

    let expected = bridge_lifecycle(&fact.lifecycle.lifecycle);
    if declared != &expected {
        return Err(semantic_violation(
            location,
            format!(
                "{position} for type {} has lifecycle plan {declared:?}, but independent classification requires {expected:?}",
                ty.get()
            ),
        ));
    }
    Ok(())
}

pub(super) fn matches_declared_plan(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    declared: &LinkedValueTransferPlan,
) -> bool {
    facts
        .types
        .get(ty.get() as usize)
        .filter(|fact| fact.coordinate == ty)
        .is_some_and(|fact| {
            recursive_shape(declared).is_none()
                && bridge_lifecycle(&fact.lifecycle.lifecycle).eq(declared)
        })
}

pub(super) fn table_location(
    table: CandidateTable,
    row: usize,
) -> Result<VerificationLocation, VerificationError> {
    let row = u32::try_from(row).map_err(|_| {
        semantic_violation(
            VerificationLocation::Image,
            format!("{} table row ordinal {row} does not fit u32", table.name()),
        )
    })?;
    Ok(VerificationLocation::Table { table, row })
}

fn bridge_lifecycle(lifecycle: &NativeValueLifecycleConcrete) -> LinkedValueTransferPlan {
    match lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            LinkedValueTransferPlan::SnapshotShare {
                drop: bridge_value_drop(drop),
            }
        }
        NativeValueLifecycleConcrete::MoveOnly { drop } => LinkedValueTransferPlan::MoveOnly {
            drop: bridge_value_drop(drop),
        },
        NativeValueLifecycleConcrete::AffineResource { drop } => {
            LinkedValueTransferPlan::AffineResource {
                drop: bridge_resource_drop(drop),
            }
        }
        NativeValueLifecycleConcrete::ExplicitCloneLease {
            clone_adapter,
            drop,
        } => LinkedValueTransferPlan::ExplicitCloneLease {
            clone_adapter: clone_adapter.clone(),
            drop: bridge_resource_drop(drop),
        },
    }
}

fn bridge_value_drop(drop: &NativeValueDropPlan) -> LinkedValueDropPlan {
    match drop {
        NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
        NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
        NativeValueDropPlan::NativeAdapter { adapter } => LinkedValueDropPlan::NativeAdapter {
            adapter: adapter.clone(),
        },
    }
}

fn bridge_resource_drop(drop: &NativeResourceDropPlan) -> LinkedResourceDropPlan {
    match drop {
        NativeResourceDropPlan::ResourceTableRelease => {
            LinkedResourceDropPlan::ResourceTableRelease
        }
        NativeResourceDropPlan::NativeAdapter { adapter } => {
            LinkedResourceDropPlan::NativeAdapter {
                adapter: adapter.clone(),
            }
        }
    }
}

fn recursive_shape(plan: &LinkedValueTransferPlan) -> Option<u32> {
    match plan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        }
        | LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        }
        | LinkedValueTransferPlan::AffineResource {
            drop: LinkedResourceDropPlan::RecursiveShape { shape },
        }
        | LinkedValueTransferPlan::ExplicitCloneLease {
            drop: LinkedResourceDropPlan::RecursiveShape { shape },
            ..
        } => Some(shape.get()),
        _ => None,
    }
}

fn semantic_violation(
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ValueTransferAndDrop,
        location,
        detail: detail.into(),
    }
}

mod data;
mod functions;
mod signatures;

use skiff_artifact_model::{
    HostEffectExecutorIdentity, NativeResourceDropPlan, NativeValueDropPlan, NativeValueEmbedding,
    NativeValueLifecycleConcrete, PrivilegedAffineCompositeIdentity, TypeRefIr,
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

    if !plan_matches(facts, ty, &fact.lifecycle.lifecycle, declared) {
        let expected = lifecycle_expectation(facts, ty, &fact.lifecycle.lifecycle);
        let recursive_detail = recursive_shape(declared)
            .map(|shape| format!(" (recursive-shape {shape})"))
            .unwrap_or_default();
        return Err(semantic_violation(
            location,
            format!(
                "{position} for type {} has lifecycle plan {declared:?}{recursive_detail}, but independent classification requires {expected}",
                ty.get()
            ),
        ));
    }
    Ok(())
}

pub(super) fn prove_ordinary_position(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    declared: &LinkedValueTransferPlan,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let position = position.as_ref();
    prove_position(facts, ty, declared, location, position)?;
    let fact = facts.type_fact(ty).ok_or_else(|| {
        semantic_violation(
            location,
            format!("{position} has no exact concrete value fact"),
        )
    })?;
    if fact.lifecycle.embedding != NativeValueEmbedding::Ordinary {
        return Err(semantic_violation(
            location,
            format!("{position} is not an Ordinary value placement"),
        ));
    }
    Ok(())
}

pub(super) fn prove_request_local_position(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    declared: &LinkedValueTransferPlan,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let position = position.as_ref();
    prove_position(facts, ty, declared, location, position)?;
    let fact = facts.type_fact(ty).ok_or_else(|| {
        semantic_violation(
            location,
            format!("{position} has no exact concrete value fact"),
        )
    })?;
    let admitted = match fact.lifecycle.embedding {
        NativeValueEmbedding::Ordinary => true,
        NativeValueEmbedding::Privileged => facts
            .privileged_affine_shapes
            .iter()
            .any(|shape| shape.nominal_type == ty),
        NativeValueEmbedding::Forbidden => is_exact_http_body_stream(&fact.normalized_type),
    };
    if !admitted {
        return Err(semantic_violation(
            location,
            format!("{position} is neither Ordinary nor an exact request-local HTTP stream value"),
        ));
    }
    Ok(())
}

pub(super) fn prove_host_result_position(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    declared: &LinkedValueTransferPlan,
    identity: HostEffectExecutorIdentity,
    ordinal: usize,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let position = position.as_ref();
    if identity != HostEffectExecutorIdentity::HttpClientStream {
        return prove_ordinary_position(facts, ty, declared, location, position);
    }
    prove_position(facts, ty, declared, location, position)?;
    let exact = ordinal == 0
        && facts.privileged_affine_shapes.iter().any(|shape| {
            shape.identity == PrivilegedAffineCompositeIdentity::HttpClientStreamHandle
                && shape.nominal_type == ty
        });
    if !exact {
        return Err(semantic_violation(
            location,
            format!("{position} is not the exact privileged HttpClientStreamHandle result"),
        ));
    }
    Ok(())
}

pub(super) fn prove_server_stream_type(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let position = position.as_ref();
    let fact = facts.type_fact(ty).ok_or_else(|| {
        semantic_violation(
            location,
            format!("{position} has no exact concrete value fact"),
        )
    })?;
    if fact.lifecycle.embedding != NativeValueEmbedding::Forbidden
        || !is_exact_server_stream(&fact.normalized_type)
    {
        return Err(semantic_violation(
            location,
            format!("{position} is not the exact server-stream result authority"),
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
        .is_some_and(|fact| plan_matches(facts, ty, &fact.lifecycle.lifecycle, declared))
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

fn plan_matches(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    lifecycle: &NativeValueLifecycleConcrete,
    declared: &LinkedValueTransferPlan,
) -> bool {
    if matches!(
        lifecycle,
        NativeValueLifecycleConcrete::MoveOnly {
            drop: NativeValueDropPlan::PrivilegedRecursiveShape,
        }
    ) {
        let LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        } = declared
        else {
            return false;
        };
        return facts
            .privileged_shape(*shape)
            .is_some_and(|fact| fact.nominal_type == ty);
    }
    recursive_shape(declared).is_none()
        && bridge_lifecycle(lifecycle).is_some_and(|expected| expected == *declared)
}

fn lifecycle_expectation(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    lifecycle: &NativeValueLifecycleConcrete,
) -> String {
    if matches!(
        lifecycle,
        NativeValueLifecycleConcrete::MoveOnly {
            drop: NativeValueDropPlan::PrivilegedRecursiveShape,
        }
    ) {
        let shapes = facts
            .privileged_affine_shapes
            .iter()
            .filter(|fact| fact.nominal_type == ty)
            .map(|fact| fact.shape.get().to_string())
            .collect::<Vec<_>>();
        return format!("an exact MoveOnly recursive shape binding in {shapes:?}");
    }
    format!("{:?}", bridge_lifecycle(lifecycle))
}

fn bridge_lifecycle(lifecycle: &NativeValueLifecycleConcrete) -> Option<LinkedValueTransferPlan> {
    Some(match lifecycle {
        NativeValueLifecycleConcrete::SnapshotShare { drop } => {
            LinkedValueTransferPlan::SnapshotShare {
                drop: bridge_value_drop(drop)?,
            }
        }
        NativeValueLifecycleConcrete::MoveOnly { drop } => LinkedValueTransferPlan::MoveOnly {
            drop: bridge_value_drop(drop)?,
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
    })
}

fn bridge_value_drop(drop: &NativeValueDropPlan) -> Option<LinkedValueDropPlan> {
    Some(match drop {
        NativeValueDropPlan::Trivial => LinkedValueDropPlan::Trivial,
        NativeValueDropPlan::SnapshotRelease => LinkedValueDropPlan::SnapshotRelease,
        NativeValueDropPlan::PrivilegedRecursiveShape => return None,
        NativeValueDropPlan::NativeAdapter { adapter } => LinkedValueDropPlan::NativeAdapter {
            adapter: adapter.clone(),
        },
    })
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

fn is_exact_http_body_stream(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args }
            if name == "Stream"
                && matches!(
                    args.as_slice(),
                    [TypeRefIr::Builtin { name, args }] if name == "bytes" && args.is_empty()
                )
    )
}

fn is_exact_server_stream(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args }
            if name == "Stream" && args.len() == 1
    )
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

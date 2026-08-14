mod placements;

use skiff_artifact_model::{
    classify_value_lifecycle, normalize_value_lifecycle_type, NativeValueDropPlan,
    NativeValueEmbedding, NativeValueLifecycleConcrete, NativeValueLifecycleResolution,
    PositionalTypeEnvironment, ValueLifecyclePolicyError, ValueLifecycleResolverError,
};
use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate, LinkedTypeEntry};

use super::{
    classes::{build_type_classes, ClassifiedType},
    normalization::{
        lifecycle_budget_after_owner_normalization, normalize_owner_type,
        preflight_candidate_types, OwnerNormalizationBudget,
    },
    resolver::HydratedValueLifecycleResolver,
    ConcreteValueFacts,
};
use crate::{
    VerificationError, VerificationLimit, VerificationLimits, VerificationLocation,
    VerificationObligation,
};

/// Independently normalizes and classifies every candidate type.
pub(super) fn prove_concrete_types(
    candidate: &LinkedBytecodeCandidate,
    resolver: &mut HydratedValueLifecycleResolver<'_>,
    limits: &VerificationLimits,
) -> Result<ConcreteValueFacts, VerificationError> {
    if candidate.types().is_empty() {
        return build_type_classes(Vec::new(), 0, limits.max_value_lifecycle_canonical_bytes);
    }

    prove_nonzero_budget(limits)?;
    preflight_candidate_types(candidate, limits)?;
    let environment = PositionalTypeEnvironment::empty();
    let mut owner_budget = OwnerNormalizationBudget::new(limits);
    let mut owner_normalized = Vec::with_capacity(candidate.types().len());

    for row in candidate.types() {
        let location = type_location(row);
        resolver
            .begin_row(row.origin().package_build_id())
            .map_err(|error| {
                resolver_error(
                    error,
                    VerificationObligation::ConcreteTypeAndShape,
                    location,
                    "establishing exact type-row origin",
                )
            })?;
        if resolver.current_package_build_id() != Some(row.origin().package_build_id()) {
            return Err(semantic_violation(
                VerificationObligation::ConcreteTypeAndShape,
                location,
                "resolver did not retain the exact type-row package build scope",
            ));
        }
        prove_nongeneric_origin(row, resolver, location)?;

        let raw_type = resolver
            .source_type(*row.origin().artifact_index())
            .map_err(|error| {
                resolver_error(
                    error,
                    VerificationObligation::ConcreteTypeAndShape,
                    location,
                    "reading the admitted raw type row",
                )
            })?
            .clone();
        let normalized = normalize_owner_type(&raw_type, resolver, &mut owner_budget, location)?;
        owner_normalized.push(normalized);
    }

    let first_location = candidate
        .types()
        .first()
        .map(type_location)
        .unwrap_or(VerificationLocation::Image);
    let mut budget =
        lifecycle_budget_after_owner_normalization(limits, &owner_budget, first_location)?;
    let mut types = Vec::with_capacity(candidate.types().len());
    for (row, owner_normalized_type) in candidate.types().iter().zip(owner_normalized) {
        let location = type_location(row);
        resolver
            .begin_row(row.origin().package_build_id())
            .map_err(|error| {
                resolver_error(
                    error,
                    VerificationObligation::ConcreteTypeAndShape,
                    location,
                    "re-establishing exact type-row origin for lifecycle classification",
                )
            })?;
        let (owner_normalized_type, private_type_authority) = owner_normalized_type.into_parts();
        let normalized_type =
            normalize_value_lifecycle_type(&owner_normalized_type, &environment, &mut budget)
                .map_err(|error| policy_error(error, location, &owner_budget, limits))?;
        if row.type_ref() != &normalized_type {
            return Err(semantic_violation(
                VerificationObligation::ConcreteTypeAndShape,
                location,
                "candidate type differs from the normalized admitted raw type",
            ));
        }
        resolver
            .establish_row_private_type_authority(private_type_authority)
            .map_err(|error| {
                resolver_error(
                    error,
                    VerificationObligation::ConcreteTypeAndShape,
                    location,
                    "establishing row-scoped private type authority",
                )
            })?;
        let lifecycle = if is_void_type(&normalized_type) {
            NativeValueLifecycleResolution {
                lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                    drop: NativeValueDropPlan::Trivial,
                },
                embedding: NativeValueEmbedding::Ordinary,
            }
        } else if is_exception_or_catch_result(&normalized_type) {
            NativeValueLifecycleResolution {
                lifecycle: NativeValueLifecycleConcrete::SnapshotShare {
                    drop: NativeValueDropPlan::SnapshotRelease,
                },
                embedding: NativeValueEmbedding::Ordinary,
            }
        } else {
            classify_value_lifecycle(&normalized_type, &environment, resolver, &mut budget)
                .map_err(|error| policy_error(error, location, &owner_budget, limits))?
        };
        types.push(ClassifiedType::new(row.index(), normalized_type, lifecycle));
    }

    let lifecycle_bytes = owner_budget
        .used_bytes()
        .checked_add(budget.used_bytes())
        .unwrap_or(u64::MAX);
    let mut facts = build_type_classes(types, lifecycle_bytes, owner_budget.max_bytes())?;
    facts.privileged_affine_shapes =
        placements::prove_type_placements(candidate, &facts, resolver)?;
    Ok(facts)
}

fn is_void_type(ty: &skiff_artifact_model::TypeRefIr) -> bool {
    matches!(
        ty,
        skiff_artifact_model::TypeRefIr::Builtin { name, args }
            if name == "void" && args.is_empty()
    )
}

fn is_exception_or_catch_result(ty: &skiff_artifact_model::TypeRefIr) -> bool {
    matches!(
        ty,
        skiff_artifact_model::TypeRefIr::Builtin { name, args }
            if (name == "Exception" && args.len() == 1)
                || (name == "CatchResult" && args.len() == 2)
    )
}

fn prove_nonzero_budget(limits: &VerificationLimits) -> Result<(), VerificationError> {
    for (limit, max) in [
        (
            VerificationLimit::ValueLifecycleNodes,
            limits.max_value_lifecycle_nodes,
        ),
        (
            VerificationLimit::ValueLifecycleCanonicalBytes,
            limits.max_value_lifecycle_canonical_bytes,
        ),
        (
            VerificationLimit::TypeNestingDepth,
            limits.max_type_nesting_depth,
        ),
    ] {
        if max == 0 {
            return Err(VerificationError::LimitExceeded {
                limit,
                actual: 1,
                max,
                location: VerificationLocation::Image,
            });
        }
    }
    Ok(())
}

fn prove_nongeneric_origin(
    row: &LinkedTypeEntry,
    resolver: &HydratedValueLifecycleResolver<'_>,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let Some(specialization) = row.origin().specialization() else {
        return Ok(());
    };
    if !specialization.concrete_type_arguments().is_empty()
        || specialization.concrete_receiver().is_some()
    {
        return Err(semantic_violation(
            VerificationObligation::ConcreteSpecialization,
            location,
            "type-row specialization retains generic arguments or a concrete receiver",
        ));
    }
    let source = resolver.source_function(specialization).map_err(|error| {
        resolver_error(
            error,
            VerificationObligation::ConcreteSpecialization,
            location,
            "reading the admitted specialization template",
        )
    })?;
    if !source.type_parameters.is_empty() || source.self_type_ref.is_some() {
        return Err(semantic_violation(
            VerificationObligation::ConcreteSpecialization,
            location,
            "type-row specialization names a generic or receiver-bearing admitted template",
        ));
    }
    Ok(())
}

fn policy_error(
    error: ValueLifecyclePolicyError,
    location: VerificationLocation,
    owner_budget: &OwnerNormalizationBudget,
    limits: &VerificationLimits,
) -> VerificationError {
    if let ValueLifecyclePolicyError::BudgetExceeded {
        dimension,
        limit,
        attempted,
    } = &error
    {
        let mapped = match *dimension {
            "nodes" => Some((
                VerificationLimit::ValueLifecycleNodes,
                owner_budget.used_nodes(),
                owner_budget.max_nodes(),
            )),
            "bytes" => Some((
                VerificationLimit::ValueLifecycleCanonicalBytes,
                owner_budget.used_bytes(),
                owner_budget.max_bytes(),
            )),
            "depth" => Some((
                VerificationLimit::TypeNestingDepth,
                0,
                limits
                    .max_type_nesting_depth
                    .min(skiff_artifact_model::bytecode::limits::MAX_NESTING_DEPTH),
            )),
            _ => None,
        };
        if let Some((limit_kind, offset, configured_max)) = mapped {
            return VerificationError::LimitExceeded {
                limit: limit_kind,
                actual: offset.checked_add(*attempted).unwrap_or(u64::MAX),
                max: configured_max.max(*limit),
                location,
            };
        }
    }
    semantic_violation(
        VerificationObligation::ConcreteTypeAndShape,
        location,
        format!("value lifecycle policy rejected the admitted raw type: {error}"),
    )
}

fn resolver_error(
    error: ValueLifecycleResolverError,
    obligation: VerificationObligation,
    location: VerificationLocation,
    action: &'static str,
) -> VerificationError {
    semantic_violation(
        obligation,
        location,
        format!(
            "{action} failed at authority {}: {}",
            error.authority, error.message
        ),
    )
}

fn semantic_violation(
    obligation: VerificationObligation,
    location: VerificationLocation,
    detail: impl Into<String>,
) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation,
        location,
        detail: detail.into(),
    }
}

fn type_location(row: &LinkedTypeEntry) -> VerificationLocation {
    VerificationLocation::Table {
        table: CandidateTable::Types,
        row: row.index().get(),
    }
}

mod placements;

use skiff_artifact_model::{
    classify_value_lifecycle, normalize_value_lifecycle_type, PositionalTypeEnvironment,
    ValueLifecyclePolicyBudget, ValueLifecyclePolicyError, ValueLifecycleResolverError,
};
use skiff_runtime_linked_bytecode::{CandidateTable, LinkedBytecodeCandidate, LinkedTypeEntry};

use super::{resolver::HydratedValueLifecycleResolver, ConcreteTypeFact, ConcreteValueFacts};
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
        return Ok(ConcreteValueFacts {
            types: Box::new([]),
        });
    }

    prove_nonzero_budget(limits)?;
    let max_depth = u32::try_from(limits.max_type_nesting_depth).unwrap_or(u32::MAX);
    let mut budget = ValueLifecyclePolicyBudget::new(
        limits.max_value_lifecycle_nodes,
        limits.max_value_lifecycle_canonical_bytes,
        max_depth,
    )
    .map_err(|error| policy_error(error, VerificationLocation::Image))?;
    let environment = PositionalTypeEnvironment::empty();
    let mut types = Vec::with_capacity(candidate.types().len());

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
        let normalized_type = normalize_value_lifecycle_type(&raw_type, &environment, &mut budget)
            .map_err(|error| policy_error(error, location))?;
        if row.type_ref() != &normalized_type {
            return Err(semantic_violation(
                VerificationObligation::ConcreteTypeAndShape,
                location,
                "candidate type differs from the normalized admitted raw type",
            ));
        }
        let lifecycle =
            classify_value_lifecycle(&normalized_type, &environment, resolver, &mut budget)
                .map_err(|error| policy_error(error, location))?;
        types.push(ConcreteTypeFact {
            normalized_type,
            lifecycle,
        });
    }

    let facts = ConcreteValueFacts {
        types: types.into_boxed_slice(),
    };
    placements::prove_type_placements(candidate, &facts)?;
    Ok(facts)
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
) -> VerificationError {
    if let ValueLifecyclePolicyError::BudgetExceeded {
        dimension,
        limit,
        attempted,
    } = &error
    {
        let mapped = match *dimension {
            "nodes" => Some(VerificationLimit::ValueLifecycleNodes),
            "bytes" => Some(VerificationLimit::ValueLifecycleCanonicalBytes),
            "depth" => Some(VerificationLimit::TypeNestingDepth),
            _ => None,
        };
        if let Some(limit_kind) = mapped {
            return VerificationError::LimitExceeded {
                limit: limit_kind,
                actual: *attempted,
                max: *limit,
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

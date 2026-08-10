mod owner;
mod preflight;
mod recursive;
mod schema;

use std::collections::BTreeSet;

use skiff_artifact_model::{PackageBuildId, TypeRefIr, ValueLifecycleResolverError};

use super::resolver::HydratedValueLifecycleResolver;
use crate::{VerificationError, VerificationLimits, VerificationLocation};

pub(super) use preflight::{preflight_candidate_types, OwnerNormalizationBudget};

/// Reconstructs the exact owner-complete form of one admitted raw type from
/// the row's already-established package-build scope.
pub(super) fn normalize_owner_type(
    raw: &TypeRefIr,
    resolver: &HydratedValueLifecycleResolver<'_>,
    budget: &mut OwnerNormalizationBudget,
    location: VerificationLocation,
) -> Result<OwnerNormalizedType, VerificationError> {
    budget.inspect(raw, location)?;
    let mut normalizer = TypeOwnerNormalizer {
        resolver,
        location,
        private_type_authority: BTreeSet::new(),
    };
    let type_ref = normalizer.normalize(raw)?;
    Ok(OwnerNormalizedType {
        type_ref,
        private_type_authority: normalizer.private_type_authority,
    })
}

pub(super) struct OwnerNormalizedType {
    type_ref: TypeRefIr,
    private_type_authority: BTreeSet<PackageBuildId>,
}

impl OwnerNormalizedType {
    pub(super) fn into_parts(self) -> (TypeRefIr, BTreeSet<PackageBuildId>) {
        (self.type_ref, self.private_type_authority)
    }

    #[cfg(test)]
    pub(super) fn into_type_ref(self) -> TypeRefIr {
        self.type_ref
    }
}

struct TypeOwnerNormalizer<'a, 'r> {
    resolver: &'a HydratedValueLifecycleResolver<'r>,
    location: VerificationLocation,
    private_type_authority: BTreeSet<PackageBuildId>,
}

impl TypeOwnerNormalizer<'_, '_> {
    fn violation(&self, detail: impl Into<String>) -> VerificationError {
        VerificationError::SemanticViolation {
            obligation: crate::VerificationObligation::ConcreteTypeAndShape,
            location: self.location,
            detail: detail.into(),
        }
    }

    fn authority_violation(
        &self,
        action: &'static str,
        error: ValueLifecycleResolverError,
    ) -> VerificationError {
        self.violation(format!(
            "{action} failed at authority {}: {}",
            error.authority, error.message
        ))
    }
}

pub(super) fn lifecycle_budget_after_owner_normalization(
    limits: &VerificationLimits,
    owner: &OwnerNormalizationBudget,
    location: VerificationLocation,
) -> Result<skiff_artifact_model::ValueLifecyclePolicyBudget, VerificationError> {
    let remaining_nodes = owner.max_nodes().saturating_sub(owner.used_nodes());
    let remaining_bytes = owner.max_bytes().saturating_sub(owner.used_bytes());
    if remaining_nodes == 0 {
        return Err(owner.limit_exceeded(
            crate::VerificationLimit::ValueLifecycleNodes,
            1,
            location,
        ));
    }
    if remaining_bytes == 0 {
        return Err(owner.limit_exceeded(
            crate::VerificationLimit::ValueLifecycleCanonicalBytes,
            1,
            location,
        ));
    }
    let effective_depth = limits
        .max_type_nesting_depth
        .min(skiff_artifact_model::bytecode::limits::MAX_NESTING_DEPTH);
    let max_depth = u32::try_from(effective_depth).unwrap_or(u32::MAX);
    skiff_artifact_model::ValueLifecyclePolicyBudget::new(
        remaining_nodes,
        remaining_bytes,
        max_depth,
    )
    .map_err(|_| VerificationError::SemanticViolation {
        obligation: crate::VerificationObligation::ConcreteTypeAndShape,
        location,
        detail: "owner normalization left an invalid lifecycle budget".to_string(),
    })
}

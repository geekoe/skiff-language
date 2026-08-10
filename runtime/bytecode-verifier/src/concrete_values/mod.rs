mod plans;
mod resolver;
mod types;

use skiff_artifact_model::{NativeValueLifecycleResolution, TypeRefIr};
use skiff_runtime_linked_bytecode::{LinkedBytecodeCandidate, TypeIndex};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{VerificationError, VerificationLimits};

/// Independently rederived concrete value facts in dense `TypeIndex` order.
///
/// Candidate-declared types and plans remain untrusted. Construction stays
/// inside the P2 proof modules; later proof families receive only this
/// read-only result.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConcreteValueFacts {
    types: Box<[ConcreteTypeFact]>,
}

/// One normalized concrete type and its independently classified lifecycle.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConcreteTypeFact {
    normalized_type: TypeRefIr,
    lifecycle: NativeValueLifecycleResolution,
}

impl ConcreteValueFacts {
    #[cfg(test)]
    pub(crate) fn empty_for_test() -> Self {
        Self {
            types: Box::new([]),
        }
    }

    /// Returns the independently derived fact for one dense linked type.
    #[allow(dead_code)]
    pub(crate) fn type_fact(&self, index: TypeIndex) -> Option<&ConcreteTypeFact> {
        usize::try_from(index.get())
            .ok()
            .and_then(|index| self.types.get(index))
    }
}

impl ConcreteTypeFact {
    #[allow(dead_code)]
    pub(crate) const fn normalized_type(&self) -> &TypeRefIr {
        &self.normalized_type
    }

    #[allow(dead_code)]
    pub(crate) const fn lifecycle(&self) -> &NativeValueLifecycleResolution {
        &self.lifecycle
    }
}

/// P2 orchestration seam: resolve exact hydrated facts, independently
/// classify every linked type, then check every candidate-declared lifecycle
/// plan against those facts.
pub(crate) fn prove_types_and_plans(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<ConcreteValueFacts, VerificationError> {
    let mut resolver = resolver::HydratedValueLifecycleResolver::new(hydrated, candidate);
    let facts = types::prove_concrete_types(candidate, &mut resolver, limits)?;
    plans::prove_declared_plans(candidate, &facts, limits)?;
    Ok(facts)
}

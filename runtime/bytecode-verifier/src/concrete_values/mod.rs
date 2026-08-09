mod plans;
mod resolver;
mod types;

use skiff_artifact_model::{NativeValueLifecycleResolution, TypeRefIr};
use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
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

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::{resolver::HydratedValueLifecycleResolver, ConcreteValueFacts};
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Independently normalizes and classifies every candidate type.
///
/// The proof body is intentionally left to the P2 type owner.
pub(super) fn prove_concrete_types(
    _candidate: &LinkedBytecodeCandidate,
    _resolver: &mut HydratedValueLifecycleResolver<'_>,
    _limits: &VerificationLimits,
) -> Result<ConcreteValueFacts, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ConcreteTypeAndShape,
        location: VerificationLocation::Image,
    })
}

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::ConcreteValueFacts;
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Checks every candidate-declared lifecycle plan against concrete type facts.
///
/// The proof body is intentionally left to the P2 plan owner.
pub(super) fn prove_declared_plans(
    _candidate: &LinkedBytecodeCandidate,
    _facts: &ConcreteValueFacts,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ValueTransferAndDrop,
        location: VerificationLocation::Image,
    })
}

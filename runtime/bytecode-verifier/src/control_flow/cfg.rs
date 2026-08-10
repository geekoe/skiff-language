use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::ControlFlowFacts;
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// Independently derives bounded CFG successors for every linked function.
///
/// This is the first P3 gate and deliberately rejects every candidate,
/// including candidates with no functions or functions with no instructions,
/// until the canonical opcode-driven proof is implemented.
pub(super) fn prove_control_flow(
    _candidate: &LinkedBytecodeCandidate,
    _limits: &VerificationLimits,
) -> Result<ControlFlowFacts, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ControlFlow,
        location: VerificationLocation::Image,
    })
}

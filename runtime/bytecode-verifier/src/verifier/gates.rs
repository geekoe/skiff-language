use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use crate::{
    control_flow::ControlFlowFacts, VerificationError, VerificationLocation,
    VerificationObligation, VerifiedStatementSchedule,
};

/// The effect/NoPending proof remains an explicit incomplete semantic gate.
/// It is deliberately after the complete statement schedule proof and before
/// entry distillation or seal construction. Empty images receive no bypass.
pub(super) fn prove_effect_and_no_pending(
    _candidate: &LinkedBytecodeCandidate,
    _control_flow: &ControlFlowFacts,
    _statement_schedule: &VerifiedStatementSchedule,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::EffectAndNoPending,
        location: VerificationLocation::Image,
    })
}

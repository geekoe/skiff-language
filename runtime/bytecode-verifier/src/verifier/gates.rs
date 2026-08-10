use crate::{
    admission::ExactCanonicalEffectBinding, control_flow::ControlFlowAndCallFacts,
    VerificationError, VerificationLocation, VerificationObligation, VerifiedStatementSchedule,
};

/// The effect/NoPending proof remains an explicit incomplete semantic gate.
/// It is deliberately after the complete statement schedule proof and before
/// entry distillation or seal construction. Empty images receive no bypass.
pub(super) fn prove_effect_and_no_pending(
    effect_binding: &ExactCanonicalEffectBinding,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    statement_schedule: &VerifiedStatementSchedule,
) -> Result<(), VerificationError> {
    let frontier = effect_binding.frontier_summary().map_err(|violation| {
        let (function, detail) = violation.into_parts();
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: function.map_or(VerificationLocation::Image, |function| {
                VerificationLocation::Function { function }
            }),
            detail,
        }
    })?;
    let (control_flow_function_count, exact_call_function_count) =
        control_flow_and_calls.function_counts();
    if let Some(detail) = frontier.cross_proof_mismatch_detail(
        control_flow_function_count,
        exact_call_function_count,
        statement_schedule.function_count(),
    ) {
        return Err(VerificationError::SemanticViolation {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Image,
            detail,
        });
    }

    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::EffectAndNoPending,
        location: VerificationLocation::Image,
    })
}

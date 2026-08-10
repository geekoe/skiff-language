use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::{ControlFlowFacts, ExactTargetAndCallFacts};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// Proves exact instruction targets, callable signatures and call plans.
pub(super) fn prove_exact_targets_and_call_plans(
    _candidate: &LinkedBytecodeCandidate,
    _concrete_values: &ConcreteValueFacts,
    _control_flow: &ControlFlowFacts,
    _limits: &VerificationLimits,
) -> Result<ExactTargetAndCallFacts, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location: VerificationLocation::Image,
    })
}

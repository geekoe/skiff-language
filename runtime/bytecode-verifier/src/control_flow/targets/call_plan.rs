use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};

use super::{
    facts::{ExactCallPlan, ExactTargetCoordinate},
    ControlFlowFacts,
};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// P3 call-shape, P2-position and pending-plan seam.
#[allow(dead_code)]
pub(super) fn prove_call_plan(
    _candidate: &LinkedBytecodeCandidate,
    _concrete_values: &ConcreteValueFacts,
    _control_flow: &ControlFlowFacts,
    _caller: FunctionIndex,
    _site: InstructionIndex,
    _target: ExactTargetCoordinate,
    _limits: &VerificationLimits,
) -> Result<ExactCallPlan, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location: VerificationLocation::Image,
    })
}

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::{ControlFlowFacts, ExactTargetAndCallFacts};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// Computes and merges abstract operand-stack and frame-slot state.
pub(super) fn prove_stack_and_slot_state(
    _candidate: &LinkedBytecodeCandidate,
    _concrete_values: &ConcreteValueFacts,
    _targets: &ExactTargetAndCallFacts,
    _control_flow: &mut ControlFlowFacts,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::StackAndSlotState,
        location: VerificationLocation::Image,
    })
}

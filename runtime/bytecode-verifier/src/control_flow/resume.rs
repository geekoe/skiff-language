use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;

use super::{ControlFlowFacts, ExactTargetAndCallFacts};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// Checks every pending-capable site against its unique resume descriptor.
pub(super) fn prove_resume_sites(
    _candidate: &LinkedBytecodeCandidate,
    _concrete_values: &ConcreteValueFacts,
    _targets: &ExactTargetAndCallFacts,
    _control_flow: &ControlFlowFacts,
    _limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ResumeSite,
        location: VerificationLocation::Image,
    })
}

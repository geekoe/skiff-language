use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::facts::ExactTargetCoordinate;
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// P3 exact-local specialization and function-coordinate seam.
#[allow(dead_code)]
pub(super) fn prove_exact_local_target(
    _hydrated: &HydratedDeploymentBytecode,
    _candidate: &LinkedBytecodeCandidate,
    _caller: FunctionIndex,
    _site: InstructionIndex,
    _target: FunctionIndex,
    _limits: &VerificationLimits,
) -> Result<ExactTargetCoordinate, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location: VerificationLocation::Image,
    })
}

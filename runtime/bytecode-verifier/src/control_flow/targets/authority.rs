use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::facts::ExactTargetCoordinate;
use crate::{VerificationError, VerificationLimits, VerificationLocation, VerificationObligation};

/// P3 authority seam for a typed, image-local call target.
#[allow(dead_code)]
pub(super) fn prove_target_authority(
    _hydrated: &HydratedDeploymentBytecode,
    _candidate: &LinkedBytecodeCandidate,
    _caller: FunctionIndex,
    _site: InstructionIndex,
    _limits: &VerificationLimits,
) -> Result<ExactTargetCoordinate, VerificationError> {
    Err(VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location: VerificationLocation::Image,
    })
}

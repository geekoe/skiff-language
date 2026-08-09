//! Private, fail-closed admission proofs that precede semantic verification.

mod binding;
mod bounds;

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use crate::{VerificationError, VerificationLimits};

pub(super) fn prove_admission(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    bounds::prove_checked_budgets(candidate, limits)?;
    binding::prove_exact_binding(hydrated, candidate)
}

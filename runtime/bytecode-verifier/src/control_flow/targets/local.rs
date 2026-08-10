use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedBytecodeCandidate};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{
    authority::prove_target_authority,
    facts::{ExactEffectFacts, ExactTargetCoordinate},
};
use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Proves that one CFG-derived target is the exact monomorphic ordinary
/// function represented by the candidate's exact-local target table.
pub(super) fn prove_exact_local_target(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    target: FunctionIndex,
) -> Result<(ExactTargetCoordinate, ExactEffectFacts), VerificationError> {
    let location = VerificationLocation::Instruction {
        function: caller,
        instruction: site,
    };
    let function = candidate
        .functions()
        .get(target.get() as usize)
        .filter(|function| function.index() == target)
        .ok_or_else(|| violation(location, "exact-local function target is out of bounds"))?;

    let mut declarations = candidate
        .exact_local_targets()
        .iter()
        .filter(|declaration| {
            declaration.function() == target && declaration.key() == function.key()
        });
    if declarations.next().is_none() {
        return Err(violation(
            location,
            "exact-local invocation has no matching target-table authority",
        ));
    }
    if declarations.next().is_some() {
        return Err(violation(
            location,
            "exact-local invocation has duplicate target-table authority",
        ));
    }

    if !function.key().concrete_type_arguments().is_empty()
        || function.key().concrete_receiver().is_some()
    {
        return Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
            location,
        });
    }

    let effect = prove_target_authority(hydrated, function, caller, site)?;
    Ok((ExactTargetCoordinate::LocalFunction(target), effect))
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}

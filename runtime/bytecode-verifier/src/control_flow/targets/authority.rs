use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedFunction};
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::facts::ExactEffectFacts;
use crate::{VerificationError, VerificationLocation, VerificationObligation};

/// Returns effect facts only after binding the concrete target back to its
/// exact hydrated package, admitted function and canonical effect owner.
pub(super) fn prove_target_authority(
    hydrated: &HydratedDeploymentBytecode,
    target: &LinkedFunction,
    caller: FunctionIndex,
    site: InstructionIndex,
) -> Result<ExactEffectFacts, VerificationError> {
    let location = VerificationLocation::Instruction {
        function: caller,
        instruction: site,
    };
    let package = hydrated
        .packages()
        .get(target.key().package_build_id())
        .ok_or_else(|| violation(location, "local target owner package is not hydrated"))?;

    let function_key = target.key().artifact_function_key().as_str();
    let mut sources = package
        .bytecode()
        .view()
        .functions()
        .iter()
        .filter(|source| source.function_key == function_key);
    let source = sources.next().ok_or_else(|| {
        violation(
            location,
            "local target is absent from its exact admitted bytecode artifact",
        )
    })?;
    if sources.next().is_some() {
        return Err(violation(
            location,
            "local target function key is not unique in its admitted bytecode artifact",
        ));
    }
    if source.origin.ordinary_executable().is_none() {
        return Err(VerificationError::ProofUnavailable {
            obligation: VerificationObligation::ExactTargetAndCallPlan,
            location,
        });
    }

    let canonical_callable = package
        .canonical_effect_callable_for_function_key(function_key)
        .ok_or_else(|| {
            violation(
                location,
                "local target has no canonical hydrated effect callable",
            )
        })?;
    let semantic_facts = package
        .artifact()
        .callable_semantic_facts
        .get(canonical_callable)
        .ok_or_else(|| {
            violation(
                location,
                "local target canonical callable has no semantic effect facts",
            )
        })?;

    if &source.effect_summary_ref != canonical_callable
        || target.effect_summary_ref() != canonical_callable
        || target.declarative_effect_summary() != &semantic_facts.effects
    {
        return Err(violation(
            location,
            "local target effect owner or summary differs from canonical hydrated authority",
        ));
    }

    Ok(ExactEffectFacts::new(
        canonical_callable.clone(),
        semantic_facts.effects.clone(),
    ))
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}

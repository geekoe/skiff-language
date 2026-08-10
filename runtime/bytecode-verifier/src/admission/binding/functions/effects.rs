use std::collections::BTreeSet;

use skiff_artifact_model::{CallableEffectSummary, PackageLocalAbiSymbol, PendingEffectCategory};
use skiff_runtime_linked_bytecode::LinkedFunction;
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::admission::facts::{ExactFunctionEffectBinding, ExactLocalAbiEffectDeclaration};
use crate::{VerificationError, VerificationLocation};

use super::{semantic_violation, ValidatedFunction};

pub(super) fn prove_exact_effect_binding(
    package: &HydratedBytecodePackage,
    function: &LinkedFunction,
    source: &ValidatedFunction,
) -> Result<ExactFunctionEffectBinding, VerificationError> {
    let location = VerificationLocation::Function {
        function: function.index(),
    };
    let canonical_callable = package
        .canonical_implementation_callable_for_function_key(&source.function_key)
        .ok_or_else(|| {
            semantic_violation(
                location,
                "ordinary function has no canonical implementation effect authority",
            )
        })?;
    if package.function_key_for_canonical_implementation_callable(canonical_callable)
        != Some(source.function_key.as_str())
    {
        return Err(semantic_violation(
            location,
            "canonical implementation effect authority does not map back to the function",
        ));
    }
    let summary = &package
        .artifact()
        .callable_semantic_facts
        .get(canonical_callable)
        .ok_or_else(|| {
            semantic_violation(location, "canonical callable has no semantic effect facts")
        })?
        .effects;
    if &source.effect_summary_ref != canonical_callable
        || function.effect_summary_ref() != canonical_callable
        || function.declarative_effect_summary() != summary
    {
        return Err(semantic_violation(
            location,
            "function effect owner or summary differ from the admitted artifact",
        ));
    }

    prove_summary_is_canonical(summary, location)?;
    let declarations = collect_local_abi_declarations(package, source, location)?;
    prove_alias_effect_summaries(package, summary, &declarations, location)?;
    prove_aliases_do_not_drift(summary, &declarations, location)?;

    Ok(ExactFunctionEffectBinding::new(
        function.index(),
        canonical_callable.clone(),
        summary.clone(),
        declarations.into_boxed_slice(),
    ))
}

fn prove_alias_effect_summaries(
    package: &HydratedBytecodePackage,
    canonical: &CallableEffectSummary,
    declarations: &[ExactLocalAbiEffectDeclaration],
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    for declaration in declarations {
        let alias = package
            .artifact()
            .callable_semantic_facts
            .get(declaration.callable())
            .ok_or_else(|| {
                semantic_violation(
                    location,
                    "Package Local ABI alias has no semantic effect facts",
                )
            })?;
        if &alias.effects != canonical {
            return Err(semantic_violation(
                location,
                "Package Local ABI alias effect summary drifts from canonical authority",
            ));
        }
    }
    Ok(())
}

fn collect_local_abi_declarations(
    package: &HydratedBytecodePackage,
    source: &ValidatedFunction,
    location: VerificationLocation,
) -> Result<Vec<ExactLocalAbiEffectDeclaration>, VerificationError> {
    let mut declarations = package
        .artifact()
        .package_local_abi
        .public_symbols
        .values()
        .chain(
            package
                .artifact()
                .package_local_abi
                .implementation_symbols
                .values(),
        )
        .filter_map(|symbol| match symbol {
            PackageLocalAbiSymbol::Callable {
                callable_id,
                signature,
            } if package.function_key_for_callable(callable_id)
                == Some(source.function_key.as_str()) =>
            {
                Some(ExactLocalAbiEffectDeclaration::new(
                    callable_id.clone(),
                    signature.may_suspend,
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    declarations.sort_by(|left, right| left.callable().as_str().cmp(right.callable().as_str()));
    if declarations.is_empty() {
        return Err(semantic_violation(
            location,
            "ordinary function has no Package Local ABI effect declaration",
        ));
    }
    for adjacent in declarations.windows(2) {
        if adjacent[0].callable() == adjacent[1].callable() {
            return Err(semantic_violation(
                location,
                "Package Local ABI repeats a callable effect declaration",
            ));
        }
    }
    Ok(declarations)
}

fn prove_summary_is_canonical(
    summary: &CallableEffectSummary,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let CallableEffectSummary::Analyzed { effects } = summary else {
        return Ok(());
    };
    if effects.may_pending != effects.may_pending() {
        return Err(semantic_violation(
            location,
            "canonical analyzed mayPending disagrees with pending effect categories",
        ));
    }
    let mut categories = BTreeSet::<PendingEffectCategory>::new();
    if effects
        .pending_effect_categories
        .iter()
        .copied()
        .any(|category| !categories.insert(category))
    {
        return Err(semantic_violation(
            location,
            "canonical analyzed pending effect categories contain a duplicate",
        ));
    }
    Ok(())
}

fn prove_aliases_do_not_drift(
    summary: &CallableEffectSummary,
    declarations: &[ExactLocalAbiEffectDeclaration],
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let declared = declarations[0].may_suspend();
    if declarations
        .iter()
        .any(|declaration| declaration.may_suspend() != declared)
    {
        return Err(semantic_violation(
            location,
            "Package Local ABI aliases disagree on maySuspend",
        ));
    }
    if let CallableEffectSummary::Analyzed { effects } = summary {
        if declared != effects.may_pending {
            return Err(semantic_violation(
                location,
                "Package Local ABI maySuspend disagrees with canonical analyzed mayPending",
            ));
        }
    }
    Ok(())
}

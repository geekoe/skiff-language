use skiff_artifact_model::{
    contract_for_opcode, Arity, ControlContract, OperandRole, ParamModeIr, PendingContract,
    SlotContract, ValueSource,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFrameLayout, TypeIndex,
};

use super::{
    facts::{
        CallSiteCoordinate, ExactCallPlan, ExactEffectFacts, ExactParameterPosition,
        ExactResultPosition, ExactTargetCoordinate, PendingPlan, ReceiverProjection,
    },
    ControlFlowFacts,
};
use crate::{
    concrete_values::{ConcreteTypeFact, ConcreteValueFacts},
    VerificationError, VerificationLocation, VerificationObligation,
};

/// Reconstructs one call plan from the canonical opcode contract and target
/// frame. Raw linked type indices are never treated as semantic identities.
pub(super) fn prove_call_plan(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    _control_flow: &ControlFlowFacts,
    caller: FunctionIndex,
    site: InstructionIndex,
    target: ExactTargetCoordinate,
    effect: ExactEffectFacts,
) -> Result<ExactCallPlan, VerificationError> {
    let location = VerificationLocation::Instruction {
        function: caller,
        instruction: site,
    };
    let caller_function = function(candidate, caller, location)?;
    let instruction = caller_function
        .instructions()
        .get(site.get() as usize)
        .ok_or_else(|| violation(location, "exact-local call site is out of bounds"))?;
    let ExactTargetCoordinate::LocalFunction(target_function) = target else {
        return Err(unavailable(location));
    };
    let target_function = function(candidate, target_function, location)?;
    let contract = contract_for_opcode(instruction.opcode());

    let pending = match contract.pending {
        PendingContract::TransitiveTarget {
            target: OperandRole::LocalTarget,
        } => PendingPlan::TransitiveTarget,
        PendingContract::NoPendingTarget { .. }
        | PendingContract::ActualWithResume { .. }
        | PendingContract::Never
        | PendingContract::TransitiveTarget { .. } => return Err(unavailable(location)),
    };

    if target_function
        .frame()
        .parameters()
        .iter()
        .any(|parameter| parameter.mode() != ParamModeIr::Value)
    {
        return Err(unavailable(location));
    }
    let parameters = exact_parameters(target_function.frame(), concrete_values, location)?;
    let results = exact_results(target_function.frame(), concrete_values, location)?;

    match contract.control {
        ControlContract::Fallthrough => {
            prove_fallthrough_contract(contract.typed, location)?;
            prove_declared_count(
                contract.operand_word(OperandRole::ArgCount, instruction.operands()),
                parameters.len(),
                "ArgCount",
                location,
            )?;
            prove_declared_count(
                contract.operand_word(OperandRole::ResultCount, instruction.operands()),
                results.len(),
                "ResultCount",
                location,
            )?;
        }
        ControlContract::TailCall => {
            prove_tail_contract(contract.typed, location)?;
            prove_declared_count(
                contract.operand_word(OperandRole::ArgCount, instruction.operands()),
                parameters.len(),
                "ArgCount",
                location,
            )?;
            prove_tail_results(
                caller_function.frame().result_types(),
                target_function.frame().result_types(),
                concrete_values,
                location,
            )?;
        }
        _ => return Err(unavailable(location)),
    }

    Ok(ExactCallPlan::new(
        CallSiteCoordinate::new(caller, site),
        target,
        effect,
        ReceiverProjection::None,
        parameters.into_boxed_slice(),
        results.into_boxed_slice(),
        pending,
        None,
        None,
    ))
}

fn exact_parameters(
    frame: &LinkedFrameLayout,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<Vec<ExactParameterPosition>, VerificationError> {
    frame
        .parameters()
        .iter()
        .enumerate()
        .map(|(ordinal, parameter)| {
            let slot = parameter.slot().get() as usize;
            let ty = frame.slot_types().get(slot).copied().ok_or_else(|| {
                violation(
                    location,
                    format!("target parameter {ordinal} selects an absent frame slot"),
                )
            })?;
            let slot_plan = frame.slot_plans().get(slot).ok_or_else(|| {
                violation(
                    location,
                    format!("target parameter {ordinal} has no slot lifecycle plan"),
                )
            })?;
            if slot_plan != parameter.plan() {
                return Err(violation(
                    location,
                    format!("target parameter {ordinal} plan does not alias its slot plan"),
                ));
            }
            require_fact(facts, ty, location, format!("target parameter {ordinal}"))?;
            Ok(ExactParameterPosition::new(ty, parameter.mode()))
        })
        .collect()
}

fn exact_results(
    frame: &LinkedFrameLayout,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<Vec<ExactResultPosition>, VerificationError> {
    frame
        .result_types()
        .iter()
        .copied()
        .enumerate()
        .map(|(ordinal, ty)| {
            require_fact(facts, ty, location, format!("target result {ordinal}"))?;
            Ok(ExactResultPosition::new(ty))
        })
        .collect()
}

fn prove_fallthrough_contract(
    typed: skiff_artifact_model::TypedTransition,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let exact_input = matches!(
        typed.stack_in,
        [group]
            if group.arity == Arity::Declared(OperandRole::ArgCount)
                && group.value
                    == ValueSource::TargetParameters {
                        target: OperandRole::LocalTarget,
                    }
    );
    let exact_output = matches!(
        typed.stack_out,
        [group]
            if group.arity == Arity::Declared(OperandRole::ResultCount)
                && group.value
                    == ValueSource::TargetResults {
                        target: OperandRole::LocalTarget,
                    }
    );
    if !exact_input || !exact_output || typed.slots != SlotContract::None {
        return Err(unavailable(location));
    }
    Ok(())
}

fn prove_tail_contract(
    typed: skiff_artifact_model::TypedTransition,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let exact_input = matches!(
        typed.stack_in,
        [group]
            if group.arity == Arity::Declared(OperandRole::ArgCount)
                && group.value
                    == ValueSource::TargetParameters {
                        target: OperandRole::LocalTarget,
                    }
    );
    if !exact_input || !typed.stack_out.is_empty() || typed.slots != SlotContract::None {
        return Err(unavailable(location));
    }
    Ok(())
}

fn prove_declared_count(
    declared: Option<u32>,
    expected: usize,
    name: &'static str,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let declared = declared.ok_or_else(|| {
        violation(
            location,
            format!("canonical local-call contract has no {name} immediate"),
        )
    })?;
    let expected = u32::try_from(expected)
        .map_err(|_| violation(location, format!("target {name} does not fit u32")))?;
    if declared != expected {
        return Err(violation(
            location,
            format!("local call {name} {declared} does not match exact target arity {expected}"),
        ));
    }
    Ok(())
}

fn prove_tail_results(
    caller: &[TypeIndex],
    target: &[TypeIndex],
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if caller.len() != target.len() {
        return Err(violation(
            location,
            "tail-call target result arity differs from the caller result arity",
        ));
    }
    for (ordinal, (caller, target)) in caller.iter().zip(target).enumerate() {
        let caller = require_fact(facts, *caller, location, format!("caller result {ordinal}"))?;
        let target = require_fact(facts, *target, location, format!("target result {ordinal}"))?;
        if !same_concrete_value(caller, target) {
            return Err(violation(
                location,
                format!("tail-call result {ordinal} differs from the caller result type"),
            ));
        }
    }
    Ok(())
}

fn require_fact(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    location: VerificationLocation,
    position: impl AsRef<str>,
) -> Result<&ConcreteTypeFact, VerificationError> {
    facts.type_fact(ty).ok_or_else(|| {
        violation(
            location,
            format!(
                "{} references type {} without a concrete value fact",
                position.as_ref(),
                ty.get()
            ),
        )
    })
}

fn same_concrete_value(left: &ConcreteTypeFact, right: &ConcreteTypeFact) -> bool {
    left.normalized_type() == right.normalized_type() && left.lifecycle() == right.lifecycle()
}

fn function(
    candidate: &LinkedBytecodeCandidate,
    index: FunctionIndex,
    location: VerificationLocation,
) -> Result<&skiff_runtime_linked_bytecode::LinkedFunction, VerificationError> {
    candidate
        .functions()
        .get(index.get() as usize)
        .filter(|function| function.index() == index)
        .ok_or_else(|| violation(location, "local-call function coordinate is out of bounds"))
}

const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}

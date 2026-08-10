use skiff_artifact_model::ParamModeIr;
use skiff_runtime_linked_bytecode::LinkedFrameLayout;

use super::{consume_inputs, Context};
use crate::{
    control_flow::{
        tail::VerifiedTailCallProof, AbstractSlotState, AbstractValue, ExactCallPlan,
        ExactTargetCoordinate, PendingPlan, ProgramPointState,
    },
    VerificationError, VerificationLocation, VerificationObligation,
};

pub(super) fn prove(
    before: &ProgramPointState,
    context: &Context<'_>,
    plan: &ExactCallPlan,
) -> Result<VerifiedTailCallProof, VerificationError> {
    let ExactTargetCoordinate::LocalFunction(target_index) = plan.target() else {
        return Err(unavailable(context.location));
    };
    if plan.call_site().function() != context.function.index()
        || plan.call_site().instruction() != context.instruction_index
        || plan.pending() != PendingPlan::TransitiveTarget
        || plan.resume().is_some()
        || plan.loan_layout().is_some()
        || plan
            .parameters()
            .iter()
            .any(|parameter| parameter.mode() != ParamModeIr::Value)
    {
        return Err(unavailable(context.location));
    }
    if !before.active_regions.is_empty() {
        return Err(violation(
            context.location,
            "tail replacement has active exception regions",
        ));
    }
    if !before.writable_loans.is_empty() {
        return Err(violation(
            context.location,
            "tail replacement has outstanding writable loans",
        ));
    }

    let (residue, inputs) = consume_inputs(before, context)?;
    if !residue.is_empty() {
        return Err(violation(
            context.location,
            "tail call leaves residual values below its exact arguments",
        ));
    }
    let [arguments] = inputs.as_slice() else {
        return Err(unavailable(context.location));
    };

    let target = context
        .candidate
        .functions()
        .get(target_index.get() as usize)
        .filter(|target| target.index() == target_index)
        .ok_or_else(|| violation(context.location, "tail target function is not dense"))?;
    prove_arguments(arguments, target.frame(), plan, context)?;
    prove_result_plan(context.function.frame(), target.frame(), context)?;
    prove_caller_cleanup(before, context)?;

    Ok(VerifiedTailCallProof::new(
        context.function.index(),
        plan.call_site().instruction(),
        target_index,
    ))
}

fn prove_arguments(
    arguments: &[AbstractValue],
    target: &LinkedFrameLayout,
    plan: &ExactCallPlan,
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    if arguments.len() != plan.parameters().len()
        || target.parameters().len() != plan.parameters().len()
    {
        return Err(violation(
            context.location,
            "tail argument transfer is not exact",
        ));
    }

    for (ordinal, ((argument, parameter), target_parameter)) in arguments
        .iter()
        .zip(plan.parameters())
        .zip(target.parameters())
        .enumerate()
    {
        if target_parameter.mode() != ParamModeIr::Value {
            return Err(unavailable(context.location));
        }
        let slot = target_parameter.slot().get() as usize;
        let target_type = target.slot_types().get(slot).copied().ok_or_else(|| {
            violation(
                context.location,
                format!("tail target parameter {ordinal} has no frame slot"),
            )
        })?;
        let target_plan = target.slot_plans().get(slot).ok_or_else(|| {
            violation(
                context.location,
                format!("tail target parameter {ordinal} has no lifecycle plan"),
            )
        })?;
        let AbstractValue::Concrete(argument_type) = *argument;
        if context
            .facts
            .semantically_equal(argument_type, parameter.ty())
            != Some(true)
            || context
                .facts
                .semantically_equal(parameter.ty(), target_type)
                != Some(true)
            || !context
                .facts
                .matches_declared_plan(argument_type, target_plan)
            || target_parameter.plan() != target_plan
        {
            return Err(violation(
                context.location,
                format!("tail argument {ordinal} has no exact target-slot transfer plan"),
            ));
        }
    }
    Ok(())
}

fn prove_result_plan(
    caller: &LinkedFrameLayout,
    target: &LinkedFrameLayout,
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    if caller.result_types().len() != target.result_types().len()
        || caller.result_plans().len() != target.result_plans().len()
    {
        return Err(violation(
            context.location,
            "tail target return plan is not exact",
        ));
    }
    for (ordinal, (((caller_type, caller_plan), target_type), target_plan)) in caller
        .result_types()
        .iter()
        .zip(caller.result_plans())
        .zip(target.result_types())
        .zip(target.result_plans())
        .enumerate()
    {
        if context.facts.semantically_equal(*caller_type, *target_type) != Some(true)
            || caller_plan != target_plan
            || !context
                .facts
                .matches_declared_plan(*caller_type, caller_plan)
            || !context
                .facts
                .matches_declared_plan(*target_type, target_plan)
        {
            return Err(violation(
                context.location,
                format!("tail result {ordinal} has no exact common return plan"),
            ));
        }
    }
    Ok(())
}

fn prove_caller_cleanup(
    before: &ProgramPointState,
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    let frame = context.function.frame();
    if before.slots.len() != frame.slot_types().len()
        || before.slots.len() != frame.slot_plans().len()
    {
        return Err(violation(
            context.location,
            "tail replacement slot state is not dense with the caller frame",
        ));
    }

    for (ordinal, ((state, declared_type), declared_plan)) in before
        .slots
        .iter()
        .zip(frame.slot_types())
        .zip(frame.slot_plans())
        .enumerate()
    {
        let AbstractSlotState::Live(actual_type) = state else {
            continue;
        };
        if context
            .facts
            .semantically_equal(*actual_type, *declared_type)
            != Some(true)
            || !context
                .facts
                .matches_declared_plan(*actual_type, declared_plan)
        {
            return Err(violation(
                context.location,
                format!("live caller slot {ordinal} has no exact tail cleanup plan"),
            ));
        }
    }
    Ok(())
}

pub(super) const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::TailCall,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::TailCall,
        location,
        detail: detail.into(),
    }
}

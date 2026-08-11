mod authority;
mod call_plan;
mod facts;
mod local;
mod remote;

use skiff_runtime_linked_bytecode::LinkedBytecodeCandidate;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::ControlFlowFacts;
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

pub(super) use facts::ExactTargetAndCallFacts;
pub(crate) use facts::{ExactCallPlan, ExactTargetCoordinate, PendingPlan};

/// Proves exact instruction targets, callable signatures and call plans.
pub(super) fn prove_exact_targets_and_call_plans(
    hydrated: &HydratedDeploymentBytecode,
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    _limits: &VerificationLimits,
) -> Result<ExactTargetAndCallFacts, VerificationError> {
    let mut dense = candidate
        .functions()
        .iter()
        .map(|function| vec![None; function.instructions().len()])
        .collect::<Vec<_>>();

    for (caller_ordinal, invocation) in control_flow.exact_local_invocations() {
        let caller = u32::try_from(caller_ordinal)
            .map(skiff_runtime_linked_bytecode::FunctionIndex::new)
            .map_err(|_| {
                violation(
                    VerificationLocation::Image,
                    "dense caller ordinal does not fit FunctionIndex",
                )
            })?;
        let location = VerificationLocation::Instruction {
            function: caller,
            instruction: invocation.site,
        };
        let function = candidate
            .functions()
            .get(caller_ordinal)
            .filter(|function| function.index() == caller)
            .ok_or_else(|| violation(location, "CFG caller coordinate is not in the candidate"))?;
        if function
            .instructions()
            .get(invocation.site.get() as usize)
            .is_none()
        {
            return Err(violation(
                location,
                "CFG exact-local call site is not in the candidate function",
            ));
        }

        let (target, effect) = local::prove_exact_local_target(
            hydrated,
            candidate,
            caller,
            invocation.site,
            invocation.target,
        )?;
        let plan = call_plan::prove_call_plan(
            candidate,
            concrete_values,
            control_flow,
            caller,
            invocation.site,
            target,
            effect,
        )?;
        let slot = dense
            .get_mut(caller_ordinal)
            .and_then(|row| row.get_mut(invocation.site.get() as usize))
            .ok_or_else(|| violation(location, "dense exact-call coordinate is out of bounds"))?;
        if slot.replace(plan).is_some() {
            return Err(violation(
                location,
                "CFG reported the same exact-local call site more than once",
            ));
        }
    }

    remote::prove_remote_targets_and_call_plans(
        candidate,
        concrete_values,
        control_flow,
        &mut dense,
    )?;
    prove_unsupported_targets_remain_closed(candidate, &dense)?;
    ExactTargetAndCallFacts::try_from_dense(candidate, dense)
}

fn prove_unsupported_targets_remain_closed(
    candidate: &LinkedBytecodeCandidate,
    dense: &[Vec<Option<facts::ExactCallPlan>>],
) -> Result<(), VerificationError> {
    for (function_ordinal, function) in candidate.functions().iter().enumerate() {
        for (instruction_ordinal, instruction) in function.instructions().iter().enumerate() {
            let requires_plan = matches!(
                instruction.opcode(),
                skiff_artifact_model::Opcode::CallLocal
                    | skiff_artifact_model::Opcode::TailCallLocal
                    | skiff_artifact_model::Opcode::CallLocalInOut
                    | skiff_artifact_model::Opcode::CallService
                    | skiff_artifact_model::Opcode::CallActor
                    | skiff_artifact_model::Opcode::CallInterface
                    | skiff_artifact_model::Opcode::InvokeCallback
                    | skiff_artifact_model::Opcode::InvokeHost
                    | skiff_artifact_model::Opcode::InvokeIntrinsic
            );
            if !requires_plan
                || dense
                    .get(function_ordinal)
                    .and_then(|row| row.get(instruction_ordinal))
                    .is_some_and(Option::is_some)
            {
                continue;
            }
            let instruction = u32::try_from(instruction_ordinal)
                .map(skiff_runtime_linked_bytecode::InstructionIndex::new)
                .map_err(|_| {
                    violation(
                        VerificationLocation::Function {
                            function: function.index(),
                        },
                        "dense instruction ordinal does not fit InstructionIndex",
                    )
                })?;
            return Err(VerificationError::ProofUnavailable {
                obligation: VerificationObligation::ExactTargetAndCallPlan,
                location: VerificationLocation::Instruction {
                    function: function.index(),
                    instruction,
                },
            });
        }
    }
    Ok(())
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}

#[cfg(test)]
pub(super) fn prove_exact_local_call_plan_for_test(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    caller: skiff_runtime_linked_bytecode::FunctionIndex,
    site: skiff_runtime_linked_bytecode::InstructionIndex,
    target: skiff_runtime_linked_bytecode::FunctionIndex,
) -> Result<(), VerificationError> {
    let location = VerificationLocation::Instruction {
        function: caller,
        instruction: site,
    };
    let target_function = candidate
        .functions()
        .get(target.get() as usize)
        .filter(|function| function.index() == target)
        .ok_or_else(|| violation(location, "test target is outside the candidate"))?;
    let effect = facts::ExactEffectFacts::new(
        target_function.effect_summary_ref().clone(),
        target_function.declarative_effect_summary().clone(),
    );
    call_plan::prove_call_plan(
        candidate,
        concrete_values,
        control_flow,
        caller,
        site,
        facts::ExactTargetCoordinate::LocalFunction(target),
        effect,
    )
    .map(drop)
}

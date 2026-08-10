use std::collections::BTreeSet;

use skiff_artifact_model::{CallableEffectSummary, CallableMayEffects, Opcode};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex};

use super::{VerifiedCallableEffects, VerifiedFunctionEffects};
use crate::{
    admission::{ExactCanonicalEffectBinding, ExactFunctionEffectBinding},
    control_flow::{ControlFlowAndCallFacts, ExactTargetCoordinate, PendingPlan},
    VerificationError, VerificationLocation, VerificationObligation, VerifiedStatementSchedule,
};

/// Builds a whole-image post-fixed effect certificate without recursive graph
/// traversal. P1 owns canonical summaries, P3 owns exact call edges, and the
/// statement schedule supplies a third independent dense instruction shape.
pub(crate) fn prove_effect_and_no_pending(
    effect_binding: &ExactCanonicalEffectBinding,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    statement_schedule: &VerifiedStatementSchedule,
) -> Result<VerifiedCallableEffects, VerificationError> {
    prove_cross_fact_shape(effect_binding, control_flow_and_calls, statement_schedule)?;
    let functions = collect_authoritative_effects(effect_binding)?;
    let verified = VerifiedCallableEffects::new(functions.into_boxed_slice());
    prove_instruction_effects(control_flow_and_calls, statement_schedule, &verified)?;
    Ok(verified)
}

fn prove_cross_fact_shape(
    effect_binding: &ExactCanonicalEffectBinding,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    statement_schedule: &VerifiedStatementSchedule,
) -> Result<(), VerificationError> {
    let frontier = effect_binding.frontier_summary().map_err(|violation| {
        let (function, detail) = violation.into_parts();
        violation_error(
            function.map_or(VerificationLocation::Image, function_location),
            detail,
        )
    })?;
    let (control_flow_function_count, exact_call_function_count) =
        control_flow_and_calls.function_counts();
    if let Some(detail) = frontier.cross_proof_mismatch_detail(
        control_flow_function_count,
        exact_call_function_count,
        statement_schedule.function_count(),
    ) {
        return Err(violation_error(VerificationLocation::Image, detail));
    }
    Ok(())
}

fn collect_authoritative_effects(
    binding: &ExactCanonicalEffectBinding,
) -> Result<Vec<VerifiedFunctionEffects>, VerificationError> {
    binding
        .functions()
        .iter()
        .enumerate()
        .map(|(ordinal, binding)| collect_function_effects(ordinal, binding))
        .collect()
}

fn collect_function_effects(
    ordinal: usize,
    binding: &ExactFunctionEffectBinding,
) -> Result<VerifiedFunctionEffects, VerificationError> {
    let function = u32::try_from(ordinal)
        .map(FunctionIndex::new)
        .map_err(|_| violation_error(VerificationLocation::Image, "effect ordinal exceeds u32"))?;
    let location = function_location(function);
    let effects = binding
        .summary()
        .effects_for_boundary()
        .map_err(|_| unavailable(location))?;
    prove_canonical_effects(effects, location)?;
    if !effects.inout_path_effects.is_empty() {
        return Err(unavailable(location));
    }
    let no_pending = effects.pending_effect_categories.is_empty();
    if binding
        .local_abi_declarations()
        .iter()
        .any(|declaration| declaration.may_suspend() == no_pending)
    {
        return Err(violation_error(
            location,
            "effect certificate disagrees with Local ABI maySuspend",
        ));
    }
    Ok(VerifiedFunctionEffects::new(
        function,
        binding.canonical_callable().clone(),
        effects.clone(),
    ))
}

fn prove_canonical_effects(
    effects: &CallableMayEffects,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected_may_pending = !effects.pending_effect_categories.is_empty();
    if effects.may_pending != expected_may_pending {
        return Err(violation_error(
            location,
            "analyzed mayPending disagrees with pending categories",
        ));
    }
    let mut categories = BTreeSet::new();
    if effects
        .pending_effect_categories
        .iter()
        .copied()
        .any(|category| !categories.insert(category))
    {
        return Err(violation_error(
            location,
            "analyzed pending categories contain a duplicate",
        ));
    }
    Ok(())
}

fn prove_instruction_effects(
    control_flow_and_calls: &ControlFlowAndCallFacts,
    statement_schedule: &VerifiedStatementSchedule,
    verified: &VerifiedCallableEffects,
) -> Result<(), VerificationError> {
    for function in control_flow_and_calls.instruction_rows() {
        prove_function_shape(function, control_flow_and_calls, statement_schedule)?;
        let caller = verified.function(function.function()).ok_or_else(|| {
            violation_error(
                function_location(function.function()),
                "missing caller effect certificate",
            )
        })?;
        for (ordinal, opcode) in function.opcodes().iter().copied().enumerate() {
            let instruction_index =
                u32::try_from(ordinal)
                    .map(InstructionIndex::new)
                    .map_err(|_| {
                        violation_error(
                            function_location(function.function()),
                            "effect instruction ordinal exceeds u32",
                        )
                    })?;
            let location = instruction_location(function.function(), instruction_index);
            if !control_flow_and_calls
                .control_flow()
                .proves_reachable_instruction(function.function(), instruction_index)
            {
                return Err(violation_error(
                    location,
                    "effect instruction is not CFG-reachable",
                ));
            }
            prove_instruction(
                opcode,
                function.function(),
                instruction_index,
                caller,
                control_flow_and_calls,
                verified,
            )?;
        }
    }
    Ok(())
}

fn prove_function_shape(
    function: &crate::control_flow::VerifiedFunctionInstructions,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    statement_schedule: &VerifiedStatementSchedule,
) -> Result<(), VerificationError> {
    let instruction_count = function.opcodes().len();
    if !control_flow_and_calls
        .control_flow()
        .proves_function_shape(function.function(), instruction_count)
        || statement_schedule.instruction_count(function.function()) != Some(instruction_count)
    {
        return Err(violation_error(
            function_location(function.function()),
            "effect proof instruction shape disagrees across proof tokens",
        ));
    }
    Ok(())
}

fn prove_instruction(
    opcode: Opcode,
    caller_index: FunctionIndex,
    instruction: InstructionIndex,
    caller: &VerifiedFunctionEffects,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    verified: &VerifiedCallableEffects,
) -> Result<(), VerificationError> {
    let location = instruction_location(caller_index, instruction);
    match opcode {
        Opcode::Const
        | Opcode::CopySlot
        | Opcode::MoveSlot
        | Opcode::StoreSlot
        | Opcode::Drop
        | Opcode::Dup
        | Opcode::LoadSlot
        | Opcode::TakeSlot
        | Opcode::Pop
        | Opcode::Jump
        | Opcode::JumpIfTrue
        | Opcode::JumpIfFalse
        | Opcode::BudgetCheckpoint
        | Opcode::Return
        | Opcode::Not => {
            if control_flow_and_calls
                .exact_call_plan(caller_index, instruction)
                .is_some()
            {
                return Err(violation_error(
                    location,
                    "bottom-effect opcode unexpectedly carries an exact call plan",
                ));
            }
            Ok(())
        }
        Opcode::CallLocal => prove_local_call(
            caller_index,
            instruction,
            caller,
            control_flow_and_calls,
            verified,
        ),
        Opcode::SwitchTag
        | Opcode::Trap
        | Opcode::TailCallLocal
        | Opcode::CallService
        | Opcode::CallActor
        | Opcode::CallInterface
        | Opcode::CallLocalInOut
        | Opcode::InterfaceBoxLocal
        | Opcode::InterfaceBoxRemote
        | Opcode::MakeCallback
        | Opcode::InvokeCallback
        | Opcode::NewRecord
        | Opcode::GetDenseField
        | Opcode::SetWritablePath
        | Opcode::RepresentationWrap
        | Opcode::NewArrayBuilder
        | Opcode::ArrayBuilderPush
        | Opcode::FreezeArray
        | Opcode::ArrayGet
        | Opcode::ArrayPushOwned
        | Opcode::NewMapBuilder
        | Opcode::MapBuilderPut
        | Opcode::FreezeMap
        | Opcode::MapGet
        | Opcode::MapPutOwned
        | Opcode::ArrayLen
        | Opcode::MapLen
        | Opcode::MapEntryAt
        | Opcode::StreamNext
        | Opcode::EmitStream
        | Opcode::Throw
        | Opcode::Rethrow
        | Opcode::EnterRegion
        | Opcode::LeaveRegion
        | Opcode::InvokeHost
        | Opcode::InvokeIntrinsic
        | Opcode::Negate
        | Opcode::Add
        | Opcode::Subtract
        | Opcode::Multiply
        | Opcode::Divide
        | Opcode::Equal
        | Opcode::NotEqual
        | Opcode::LessThan
        | Opcode::LessOrEqual
        | Opcode::GreaterThan
        | Opcode::GreaterOrEqual => Err(unavailable(location)),
    }
}

fn prove_local_call(
    caller_index: FunctionIndex,
    instruction: InstructionIndex,
    caller: &VerifiedFunctionEffects,
    control_flow_and_calls: &ControlFlowAndCallFacts,
    verified: &VerifiedCallableEffects,
) -> Result<(), VerificationError> {
    let location = instruction_location(caller_index, instruction);
    let plan = control_flow_and_calls
        .exact_call_plan(caller_index, instruction)
        .ok_or_else(|| unavailable(location))?;
    if plan.call_site().function() != caller_index || plan.call_site().instruction() != instruction
    {
        return Err(violation_error(
            location,
            "effect call plan coordinate mismatch",
        ));
    }
    let ExactTargetCoordinate::LocalFunction(target_index) = plan.target() else {
        return Err(unavailable(location));
    };
    let target = verified
        .function(target_index)
        .ok_or_else(|| violation_error(location, "effect call target has no dense certificate"))?;
    prove_plan_effect_matches_target(plan, target, location)?;
    match plan.pending() {
        PendingPlan::TransitiveTarget => {
            prove_effect_subset(target.effects(), caller.effects(), location)
        }
        PendingPlan::RequiresNoPending => {
            if !target.no_pending() {
                return Err(violation_error(
                    location,
                    "call plan requires a NoPending target",
                ));
            }
            Err(unavailable(location))
        }
        PendingPlan::Never | PendingPlan::ActualWithResume(_) => Err(unavailable(location)),
    }
}

fn prove_plan_effect_matches_target(
    plan: &crate::control_flow::ExactCallPlan,
    target: &VerifiedFunctionEffects,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if plan.effect().canonical_callable() != target.canonical_callable() {
        return Err(violation_error(
            location,
            "call-plan effect owner disagrees with dense canonical binding",
        ));
    }
    let CallableEffectSummary::Analyzed { effects } = plan.effect().summary() else {
        return Err(unavailable(location));
    };
    if effects != target.effects() {
        return Err(violation_error(
            location,
            "call-plan effect summary disagrees with dense canonical binding",
        ));
    }
    Ok(())
}

fn prove_effect_subset(
    callee: &CallableMayEffects,
    caller: &CallableMayEffects,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if callee.escapes_caller_value && !caller.escapes_caller_value {
        return Err(underclaim(location, "escapesCallerValue"));
    }
    if callee.requires_same_heap_identity && !caller.requires_same_heap_identity {
        return Err(underclaim(location, "requiresSameHeapIdentity"));
    }
    if callee.invokes_unknown_target && !caller.invokes_unknown_target {
        return Err(underclaim(location, "invokesUnknownTarget"));
    }
    if let Some(category) = callee
        .pending_effect_categories
        .iter()
        .find(|category| !caller.pending_effect_categories.contains(category))
    {
        return Err(underclaim(
            location,
            format!("pending category {category:?}"),
        ));
    }
    if !callee.inout_path_effects.is_empty() || !caller.inout_path_effects.is_empty() {
        return Err(unavailable(location));
    }
    Ok(())
}

fn underclaim(location: VerificationLocation, field: impl AsRef<str>) -> VerificationError {
    violation_error(
        location,
        format!(
            "caller effect summary underclaims callee {}",
            field.as_ref()
        ),
    )
}

const fn function_location(function: FunctionIndex) -> VerificationLocation {
    VerificationLocation::Function { function }
}

const fn instruction_location(
    function: FunctionIndex,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function,
        instruction,
    }
}

const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::EffectAndNoPending,
        location,
    }
}

fn violation_error(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::EffectAndNoPending,
        location,
        detail: detail.into(),
    }
}

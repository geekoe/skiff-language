mod instruction;
mod state;

use std::collections::VecDeque;

use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedFunction,
};

use super::{
    tail::{VerifiedTailCallFacts, VerifiedTailCallProof},
    ControlFlowEdgeKind, ControlFlowFacts, ExactTargetAndCallFacts, FunctionFlowFacts,
    ProgramPointState,
};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLimit, VerificationLimits,
    VerificationLocation, VerificationObligation,
};

/// Computes and merges abstract operand-stack and frame-slot state without
/// consulting the candidate stack-map sidecar. The sidecar is compared only
/// after the independent fixpoint and actual maximum depth are complete.
pub(super) fn prove_stack_and_slot_state(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    targets: &ExactTargetAndCallFacts,
    control_flow: &mut ControlFlowFacts,
    limits: &VerificationLimits,
) -> Result<VerifiedTailCallFacts, VerificationError> {
    if candidate.functions().len() != control_flow.functions.len() {
        return Err(violation(
            VerificationLocation::Image,
            "CFG function facts are not dense with the candidate",
        ));
    }
    let mut tail_rows = Vec::with_capacity(candidate.functions().len());
    for (ordinal, (function, flow)) in candidate
        .functions()
        .iter()
        .zip(control_flow.functions.iter_mut())
        .enumerate()
    {
        let expected = u32::try_from(ordinal)
            .map(FunctionIndex::new)
            .map_err(|_| violation(VerificationLocation::Image, "function ordinal exceeds u32"))?;
        if function.index() != expected {
            return Err(violation(
                VerificationLocation::Function {
                    function: function.index(),
                },
                "candidate functions are not dense",
            ));
        }
        tail_rows.push(prove_function(
            function,
            candidate,
            concrete_values,
            targets,
            flow,
            limits,
        )?);
    }
    VerifiedTailCallFacts::try_from_dense(candidate, targets, tail_rows)
}

fn prove_function(
    function: &LinkedFunction,
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    targets: &ExactTargetAndCallFacts,
    flow: &mut FunctionFlowFacts,
    limits: &VerificationLimits,
) -> Result<Vec<Option<VerifiedTailCallProof>>, VerificationError> {
    let instruction_count = function.instructions().len();
    if flow.successors.len() != instruction_count {
        return Err(violation(
            VerificationLocation::Function {
                function: function.index(),
            },
            "CFG successor facts are not dense with the function",
        ));
    }
    let mut states = vec![None; instruction_count];
    let entry = state::seed(function, concrete_values)?;
    states[0] = Some(entry);
    let mut worklist = VecDeque::from([0_usize]);
    let mut computed_max = 0_u32;
    let mut tail_proofs = vec![None; instruction_count];

    while let Some(ordinal) = worklist.pop_front() {
        let instruction = instruction_index(function, ordinal)?;
        let location = instruction_location(function, instruction);
        let before = states
            .get(ordinal)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or_else(|| violation(location, "worklist instruction has no input state"))?;
        computed_max = record_depth(computed_max, &before, limits, location)?;
        let transfer = instruction::apply(
            candidate,
            function,
            instruction,
            &before,
            concrete_values,
            targets,
        )?;

        match transfer {
            instruction::InstructionTransfer::Tail(proof) => {
                if !flow.successors[ordinal].is_empty() {
                    return Err(tail_violation(
                        location,
                        "tail-call terminal unexpectedly has a CFG successor",
                    ));
                }
                if let Some(previous) = tail_proofs[ordinal].replace(proof) {
                    if previous != proof {
                        return Err(tail_violation(
                            location,
                            "tail-call proof changed while merging input state",
                        ));
                    }
                }
            }
            instruction::InstructionTransfer::Continue(after) => {
                computed_max = record_depth(computed_max, &after, limits, location)?;
                for edge in &flow.successors[ordinal] {
                    let propagated = match edge.kind {
                        ControlFlowEdgeKind::Ordinary => after.clone(),
                        ControlFlowEdgeKind::Exceptional { region } => {
                            exception_state(function, &before, region, concrete_values, location)?
                        }
                    };
                    let target = edge.target.get() as usize;
                    let target_location = instruction_location(function, edge.target);
                    let Some(target_state) = states.get_mut(target) else {
                        return Err(violation(target_location, "CFG successor is out of bounds"));
                    };
                    match target_state {
                        None => {
                            *target_state = Some(propagated);
                            worklist.push_back(target);
                        }
                        Some(current) => {
                            if state::merge(current, &propagated, concrete_values, target_location)?
                            {
                                worklist.push_back(target);
                            }
                        }
                    }
                }
            }
        }
    }

    let mut completed_states = Vec::with_capacity(states.len());
    for (ordinal, state) in states.into_iter().enumerate() {
        let instruction = instruction_index(function, ordinal)?;
        completed_states.push(state.ok_or_else(|| {
            violation(
                instruction_location(function, instruction),
                "instruction has no independently derived reachable state",
            )
        })?);
    }
    if computed_max > function.max_operand_depth() {
        return Err(violation(
            VerificationLocation::Function {
                function: function.index(),
            },
            format!(
                "computed operand depth {computed_max} exceeds declared maximum {}",
                function.max_operand_depth()
            ),
        ));
    }

    state::compare_hint(function, &completed_states, concrete_values)?;
    flow.states_before = completed_states.into_boxed_slice();
    flow.computed_max_operand_depth = computed_max;
    Ok(tail_proofs)
}

fn exception_state(
    function: &LinkedFunction,
    before: &ProgramPointState,
    region_index: usize,
    concrete_values: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<ProgramPointState, VerificationError> {
    let region = function
        .exception_regions()
        .get(region_index)
        .ok_or_else(|| violation(location, "exception successor region is out of bounds"))?;
    let height = region.handler_stack_height() as usize;
    if height > before.stack.len() {
        return Err(violation(
            location,
            format!(
                "exception handler stack height {height} exceeds source stack {}",
                before.stack.len()
            ),
        ));
    }
    let mut slots = before.slots.to_vec();
    let catch_slot = region.catch_slot().get() as usize;
    if catch_slot >= slots.len() {
        return Err(violation(location, "exception catch slot is out of bounds"));
    }
    if concrete_values
        .type_fact(region.catch_slot_type())
        .is_none()
    {
        return Err(violation(
            location,
            "exception catch slot type has no concrete fact",
        ));
    }
    slots[catch_slot] = super::AbstractSlotState::Live(region.catch_slot_type());
    Ok(ProgramPointState {
        stack: before.stack[..height].to_vec().into_boxed_slice(),
        slots: slots.into_boxed_slice(),
        active_regions: active_regions_at(function, region.handler()).into_boxed_slice(),
        writable_loans: Box::new([]),
    })
}

fn active_regions_at(
    function: &LinkedFunction,
    instruction: InstructionIndex,
) -> Vec<skiff_runtime_linked_bytecode::ActiveRegionIndex> {
    function
        .active_regions()
        .iter()
        .enumerate()
        .filter(|(_, region)| {
            region.start().get() <= instruction.get() && instruction.get() < region.end().get()
        })
        .map(|(ordinal, _)| skiff_runtime_linked_bytecode::ActiveRegionIndex::new(ordinal as u32))
        .collect()
}

fn record_depth(
    current: u32,
    state: &ProgramPointState,
    limits: &VerificationLimits,
    location: VerificationLocation,
) -> Result<u32, VerificationError> {
    let actual =
        u64::try_from(state.stack.len()).map_err(|_| VerificationError::LimitExceeded {
            limit: VerificationLimit::OperandDepth,
            actual: u64::MAX,
            max: limits.max_operand_depth,
            location,
        })?;
    if actual > limits.max_operand_depth {
        return Err(VerificationError::LimitExceeded {
            limit: VerificationLimit::OperandDepth,
            actual,
            max: limits.max_operand_depth,
            location,
        });
    }
    let actual = u32::try_from(actual).map_err(|_| VerificationError::LimitExceeded {
        limit: VerificationLimit::OperandDepth,
        actual,
        max: limits.max_operand_depth,
        location,
    })?;
    Ok(current.max(actual))
}

fn instruction_index(
    function: &LinkedFunction,
    ordinal: usize,
) -> Result<InstructionIndex, VerificationError> {
    u32::try_from(ordinal)
        .map(InstructionIndex::new)
        .map_err(|_| {
            violation(
                VerificationLocation::Function {
                    function: function.index(),
                },
                "dense instruction ordinal does not fit u32",
            )
        })
}

const fn instruction_location(
    function: &LinkedFunction,
    instruction: InstructionIndex,
) -> VerificationLocation {
    VerificationLocation::Instruction {
        function: function.index(),
        instruction,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::StackAndSlotState,
        location,
        detail: detail.into(),
    }
}

fn tail_violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::TailCall,
        location,
        detail: detail.into(),
    }
}

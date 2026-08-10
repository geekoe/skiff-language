use skiff_artifact_model::ParamModeIr;
use skiff_runtime_linked_bytecode::{LinkedFunction, LinkedSlotState, LinkedStackValue, TypeIndex};

use super::super::{AbstractSlotState, AbstractValue, AbstractWritableLoan, ProgramPointState};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLocation,
    VerificationObligation,
};

pub(super) fn seed(
    function: &LinkedFunction,
    facts: &ConcreteValueFacts,
) -> Result<ProgramPointState, VerificationError> {
    let location = VerificationLocation::Function {
        function: function.index(),
    };
    let mut slots = vec![AbstractSlotState::Uninitialized; function.frame().slot_types().len()];
    for (ordinal, parameter) in function.frame().parameters().iter().enumerate() {
        if parameter.mode() != ParamModeIr::Value {
            return Err(VerificationError::ProofUnavailable {
                obligation: VerificationObligation::StackAndSlotState,
                location,
            });
        }
        let slot = parameter.slot().get() as usize;
        let ty = function
            .frame()
            .slot_types()
            .get(slot)
            .copied()
            .ok_or_else(|| violation(location, format!("parameter {ordinal} has no frame slot")))?;
        require_type(facts, ty, location, format!("parameter {ordinal}"))?;
        let state = slots
            .get_mut(slot)
            .ok_or_else(|| violation(location, format!("parameter {ordinal} is out of bounds")))?;
        if !matches!(state, AbstractSlotState::Uninitialized) {
            return Err(violation(
                location,
                format!("parameter {ordinal} aliases another parameter slot"),
            ));
        }
        *state = AbstractSlotState::Live(ty);
    }
    Ok(ProgramPointState {
        stack: Box::new([]),
        slots: slots.into_boxed_slice(),
        active_regions: Box::new([]),
        writable_loans: Box::new([]),
    })
}

pub(super) fn merge(
    current: &mut ProgramPointState,
    incoming: &ProgramPointState,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<bool, VerificationError> {
    if current.stack.len() != incoming.stack.len() {
        return Err(violation(
            location,
            format!(
                "operand-stack heights differ at merge: {} and {}",
                current.stack.len(),
                incoming.stack.len()
            ),
        ));
    }
    if current.slots.len() != incoming.slots.len() {
        return Err(violation(location, "frame-slot widths differ at merge"));
    }
    if current.active_regions != incoming.active_regions {
        return Err(violation(location, "active-region states differ at merge"));
    }
    if current.writable_loans != incoming.writable_loans {
        return Err(violation(location, "writable-loan states differ at merge"));
    }

    let mut changed = false;
    for (ordinal, (left, right)) in current
        .stack
        .iter_mut()
        .zip(incoming.stack.iter())
        .enumerate()
    {
        changed |= merge_value(left, *right, facts, location, "stack", ordinal)?;
    }
    for (ordinal, (left, right)) in current
        .slots
        .iter_mut()
        .zip(incoming.slots.iter())
        .enumerate()
    {
        match (*left, *right) {
            (AbstractSlotState::Uninitialized, AbstractSlotState::Uninitialized)
            | (AbstractSlotState::Moved, AbstractSlotState::Moved) => {}
            (AbstractSlotState::Live(left_ty), AbstractSlotState::Live(right_ty)) => {
                let mut value = AbstractValue::Concrete(left_ty);
                changed |= merge_value(
                    &mut value,
                    AbstractValue::Concrete(right_ty),
                    facts,
                    location,
                    "slot",
                    ordinal,
                )?;
                let AbstractValue::Concrete(merged) = value;
                *left = AbstractSlotState::Live(merged);
            }
            _ => {
                return Err(violation(
                    location,
                    format!("slot {ordinal} liveness differs at merge"),
                ));
            }
        }
    }
    Ok(changed)
}

pub(super) fn compare_hint(
    function: &LinkedFunction,
    states: &[ProgramPointState],
    facts: &ConcreteValueFacts,
) -> Result<(), VerificationError> {
    if states.len() != function.stack_map().entries().len() {
        return Err(violation(
            VerificationLocation::Function {
                function: function.index(),
            },
            format!(
                "computed state count {} differs from candidate hint count {}",
                states.len(),
                function.stack_map().entries().len()
            ),
        ));
    }
    for (ordinal, (actual, hint)) in states
        .iter()
        .zip(function.stack_map().entries())
        .enumerate()
    {
        let instruction = u32::try_from(ordinal)
            .map(skiff_runtime_linked_bytecode::InstructionIndex::new)
            .map_err(|_| {
                violation(
                    VerificationLocation::Function {
                        function: function.index(),
                    },
                    "dense instruction ordinal does not fit u32",
                )
            })?;
        let location = VerificationLocation::Instruction {
            function: function.index(),
            instruction,
        };
        compare_stack(&actual.stack, hint.stack_before(), facts, location)?;
        compare_slots(&actual.slots, hint.slots_before(), facts, location)?;
        if actual.active_regions.as_ref() != hint.active_regions() {
            return Err(violation(
                location,
                "candidate active-region hint is incorrect",
            ));
        }
        compare_loans(&actual.writable_loans, hint.writable_loans(), location)?;
    }
    Ok(())
}

fn merge_value(
    left: &mut AbstractValue,
    right: AbstractValue,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
    owner: &'static str,
    ordinal: usize,
) -> Result<bool, VerificationError> {
    let (AbstractValue::Concrete(left_ty), AbstractValue::Concrete(right_ty)) = (*left, right);
    if facts.semantically_equal(left_ty, right_ty) != Some(true) {
        return Err(violation(
            location,
            format!("{owner} {ordinal} concrete types differ at merge"),
        ));
    }
    let merged = facts.merge_coordinate(left_ty, right_ty).map_err(|_| {
        violation(
            location,
            format!("{owner} {ordinal} has no merge coordinate"),
        )
    })?;
    let changed = merged != left_ty;
    *left = AbstractValue::Concrete(merged);
    Ok(changed)
}

fn compare_stack(
    actual: &[AbstractValue],
    hint: &[LinkedStackValue],
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if actual.len() != hint.len() {
        return Err(violation(
            location,
            "candidate operand-stack height hint is incorrect",
        ));
    }
    for (ordinal, (actual, hint)) in actual.iter().zip(hint).enumerate() {
        compare_live_value(*actual, hint, facts, location, "stack", ordinal)?;
    }
    Ok(())
}

fn compare_slots(
    actual: &[AbstractSlotState],
    hint: &[LinkedSlotState],
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    if actual.len() != hint.len() {
        return Err(violation(
            location,
            "candidate frame-slot width hint is incorrect",
        ));
    }
    for (ordinal, (actual, hint)) in actual.iter().zip(hint).enumerate() {
        match (actual, hint) {
            (AbstractSlotState::Uninitialized, LinkedSlotState::Uninitialized)
            | (AbstractSlotState::Moved, LinkedSlotState::Moved) => {}
            (AbstractSlotState::Live(actual), LinkedSlotState::Live(hint)) => {
                compare_live_value(
                    AbstractValue::Concrete(*actual),
                    hint,
                    facts,
                    location,
                    "slot",
                    ordinal,
                )?;
            }
            _ => {
                return Err(violation(
                    location,
                    format!("candidate slot {ordinal} liveness hint is incorrect"),
                ));
            }
        }
    }
    Ok(())
}

fn compare_live_value(
    actual: AbstractValue,
    hint: &LinkedStackValue,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
    owner: &'static str,
    ordinal: usize,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(actual_ty) = actual;
    if facts.semantically_equal(actual_ty, hint.ty()) != Some(true) {
        return Err(violation(
            location,
            format!("candidate {owner} {ordinal} type hint is incorrect"),
        ));
    }
    if !facts.matches_declared_plan(actual_ty, hint.plan()) {
        return Err(violation(
            location,
            format!("candidate {owner} {ordinal} lifecycle plan hint is incorrect"),
        ));
    }
    Ok(())
}

fn compare_loans(
    actual: &[AbstractWritableLoan],
    hint: &[skiff_runtime_linked_bytecode::LinkedWritableLoanState],
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let equal = actual.len() == hint.len()
        && actual.iter().zip(hint).all(|(actual, hint)| {
            actual.root_slot == hint.root_slot() && actual.path == hint.path()
        });
    if !equal {
        return Err(violation(
            location,
            "candidate writable-loan hint is incorrect",
        ));
    }
    Ok(())
}

fn require_type(
    facts: &ConcreteValueFacts,
    ty: TypeIndex,
    location: VerificationLocation,
    owner: impl AsRef<str>,
) -> Result<(), VerificationError> {
    facts.type_fact(ty).map(drop).ok_or_else(|| {
        violation(
            location,
            format!("{} type {} has no concrete fact", owner.as_ref(), ty.get()),
        )
    })
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::StackAndSlotState,
        location,
        detail: detail.into(),
    }
}

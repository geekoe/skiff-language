use skiff_artifact_model::{
    Arity, SlotAction, SlotContract, TypedStackGroup, TypedTransition, ValueSource,
};
use skiff_runtime_linked_bytecode::{
    LinkedFrameLayout, LinkedInstruction, LinkedSlotState, LinkedStackValue,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::{obligation_error, values, MachineState, StackMapContext};

pub(super) fn apply_instruction(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    transition: TypedTransition,
    state: MachineState,
    location: BytecodeLinkLocation,
) -> Result<MachineState, BytecodeLinkError> {
    let (next, inputs) = apply_stack_inputs(
        context,
        instruction,
        transition.stack_in,
        state,
        location.clone(),
    )?;
    let mut next = apply_slot_effects(
        context.frame,
        instruction,
        transition.slots,
        &inputs,
        next,
        location.clone(),
    )?;
    apply_stack_outputs(
        context,
        instruction,
        transition.stack_out,
        &inputs,
        &mut next,
        location,
    )?;
    Ok(next)
}

fn apply_stack_inputs(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    groups: &[TypedStackGroup],
    mut state: MachineState,
    location: BytecodeLinkLocation,
) -> Result<(MachineState, Vec<Vec<LinkedStackValue>>), BytecodeLinkError> {
    let arities = groups
        .iter()
        .map(|group| resolve_arity(group.arity, instruction, context.frame, location.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    let total = arities.iter().try_fold(0usize, |total, arity| {
        total.checked_add(*arity).ok_or_else(|| {
            obligation_error(
                location.clone(),
                "arithmetic overflow while summing stack inputs".to_string(),
            )
        })
    })?;
    if state.stack.len() < total {
        return Err(obligation_error(
            location,
            format!(
                "instruction requires {total} stack values but only {} are available",
                state.stack.len()
            ),
        ));
    }
    let consumed = state.stack.split_off(state.stack.len() - total);
    let mut offset = 0usize;
    let mut inputs = Vec::with_capacity(groups.len());
    for (group, arity) in groups.iter().zip(arities) {
        let end = offset.checked_add(arity).ok_or_else(|| {
            obligation_error(
                location.clone(),
                "arithmetic overflow while partitioning stack inputs".to_string(),
            )
        })?;
        let values = consumed
            .get(offset..end)
            .map(<[LinkedStackValue]>::to_vec)
            .ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    "typed stack input groups exceed the consumed stack segment".to_string(),
                )
            })?;
        validate_input_group(context, instruction, group.value, &values, location.clone())?;
        offset = end;
        inputs.push(values);
    }
    Ok((state, inputs))
}

fn validate_input_group(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    source: ValueSource,
    actual: &[LinkedStackValue],
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match source {
        ValueSource::AnyStackValue | ValueSource::TaggedValue => return Ok(()),
        ValueSource::ComparablePair => {
            if matches!(actual, [left, right] if left == right) {
                return Ok(());
            }
            return Err(obligation_error(
                location,
                "comparison inputs do not have one exact concrete type and plan".to_string(),
            ));
        }
        _ => {}
    }
    let expected = values::source_values(context, instruction, source, &[], location.clone())?;
    if expected != actual {
        return Err(obligation_error(
            location,
            format!(
                "typed stack input {} differs from its exact concrete source",
                source.name()
            ),
        ));
    }
    Ok(())
}

fn apply_stack_outputs(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    groups: &[TypedStackGroup],
    inputs: &[Vec<LinkedStackValue>],
    state: &mut MachineState,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    for group in groups {
        let arity = resolve_arity(group.arity, instruction, context.frame, location.clone())?;
        let values =
            values::source_values(context, instruction, group.value, inputs, location.clone())?;
        match values.as_slice() {
            [value] if arity > 1 => state.stack.extend((0..arity).map(|_| value.clone())),
            _ if values.len() == arity => state.stack.extend(values),
            _ => {
                return Err(obligation_error(
                    location,
                    format!(
                        "typed stack output {} produces {} values, expected {arity}",
                        group.value.name(),
                        values.len()
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn apply_slot_effects(
    frame: &LinkedFrameLayout,
    instruction: &LinkedInstruction,
    contract: SlotContract,
    inputs: &[Vec<LinkedStackValue>],
    mut state: MachineState,
    location: BytecodeLinkLocation,
) -> Result<MachineState, BytecodeLinkError> {
    let effects = match contract {
        SlotContract::None => return Ok(state),
        SlotContract::InOutCallLoans { .. } => {
            return Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
                location,
            });
        }
        SlotContract::Effects(effects) => effects,
    };
    let before = state.slots.clone();
    for effect in effects {
        let slot = values::operand_word(instruction, effect.operand, location.clone())? as usize;
        let current = before.get(slot).ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("frame slot {slot} is out of bounds"),
            )
        })?;
        match effect.action {
            SlotAction::Read | SlotAction::ReadShare | SlotAction::Mutate => {
                require_live(current, slot, effect.action, location.clone())?;
            }
            SlotAction::Take | SlotAction::Drop => {
                require_live(current, slot, effect.action, location.clone())?;
                let next_slot = state.slots.get_mut(slot).ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("frame state slot {slot} is out of bounds"),
                    )
                })?;
                *next_slot = LinkedSlotState::Moved;
            }
            SlotAction::Write => {
                let value =
                    slot_write_value(instruction, effect.value, inputs, &before, location.clone())?;
                validate_slot_write(frame, slot, &value, location.clone())?;
                let next_slot = state.slots.get_mut(slot).ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("frame state slot {slot} is out of bounds"),
                    )
                })?;
                *next_slot = LinkedSlotState::Live(value);
            }
        }
    }
    Ok(state)
}

fn require_live(
    current: &LinkedSlotState,
    slot: usize,
    action: SlotAction,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if matches!(current, LinkedSlotState::Live(_)) {
        Ok(())
    } else {
        Err(obligation_error(
            location,
            format!("slot {slot} is not live for {}", action.name()),
        ))
    }
}

fn slot_write_value(
    instruction: &LinkedInstruction,
    source: ValueSource,
    inputs: &[Vec<LinkedStackValue>],
    before: &[LinkedSlotState],
    location: BytecodeLinkLocation,
) -> Result<LinkedStackValue, BytecodeLinkError> {
    let values = match source {
        ValueSource::Slot { operand } => {
            let source = values::operand_word(instruction, operand, location.clone())? as usize;
            match before.get(source) {
                Some(LinkedSlotState::Live(value)) => vec![value.clone()],
                _ => {
                    return Err(obligation_error(
                        location,
                        format!("source slot {source} is not live"),
                    ));
                }
            }
        }
        ValueSource::StackInput { group } => {
            inputs.get(group as usize).cloned().ok_or_else(|| {
                obligation_error(
                    location.clone(),
                    format!("slot write references missing stack input group {group}"),
                )
            })?
        }
        _ => {
            return Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
                location,
            });
        }
    };
    let [value] = values.as_slice() else {
        return Err(obligation_error(
            location,
            "slot write source does not contain exactly one value".to_string(),
        ));
    };
    Ok(value.clone())
}

fn validate_slot_write(
    frame: &LinkedFrameLayout,
    slot: usize,
    value: &LinkedStackValue,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let expected_type = frame.slot_types().get(slot).copied().ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("frame slot type {slot} is out of bounds"),
        )
    })?;
    let expected_plan = frame.slot_plans().get(slot).cloned().ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("frame slot plan {slot} is out of bounds"),
        )
    })?;
    if value != &LinkedStackValue::new(expected_type, expected_plan) {
        return Err(obligation_error(
            location,
            format!("slot write type or lifecycle plan differs at slot {slot}"),
        ));
    }
    Ok(())
}

fn resolve_arity(
    arity: Arity,
    instruction: &LinkedInstruction,
    frame: &LinkedFrameLayout,
    location: BytecodeLinkLocation,
) -> Result<usize, BytecodeLinkError> {
    match arity {
        Arity::Fixed(value) => Ok(value as usize),
        Arity::Declared(role) => {
            let value = values::operand_word(instruction, role, location.clone())?;
            usize::try_from(value).map_err(|_| {
                obligation_error(
                    location,
                    "declared stack arity does not fit usize".to_string(),
                )
            })
        }
        Arity::FunctionResultCount => Ok(frame.result_types().len()),
    }
}

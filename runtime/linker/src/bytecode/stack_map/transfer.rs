use skiff_artifact_model::{
    Arity, SlotAction, SlotContract, TypedStackGroup, TypedTransition, ValueSource,
};
use skiff_runtime_linked_bytecode::{
    LinkedFrameLayout, LinkedInstruction, LinkedSlotState, LinkedStackValue,
    LinkedWritableLoanState,
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
    let (mut next, inputs) = apply_instruction_without_results(
        context,
        instruction,
        transition,
        state,
        location.clone(),
    )?;
    apply_stack_outputs(
        context,
        instruction,
        transition.stack_out,
        &inputs,
        &mut next,
        location.clone(),
    )?;
    Ok(next)
}

pub(super) fn apply_instruction_without_results(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    transition: TypedTransition,
    state: MachineState,
    location: BytecodeLinkLocation,
) -> Result<(MachineState, Vec<Vec<LinkedStackValue>>), BytecodeLinkError> {
    let (next, inputs) = apply_stack_inputs(
        context,
        instruction,
        transition.stack_in,
        state,
        location.clone(),
    )?;
    let mut next = apply_slot_effects(
        context,
        instruction,
        transition.slots,
        &inputs,
        next,
        location.clone(),
    )?;
    apply_region_effects(instruction, &mut next, location)?;
    Ok((next, inputs))
}

fn apply_region_effects(
    instruction: &LinkedInstruction,
    state: &mut MachineState,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    use skiff_artifact_model::Opcode;
    match instruction.opcode() {
        Opcode::EnterRegion => {
            let region = instruction
                .resolved_operands()
                .iter()
                .find_map(|resolved| match resolved.target() {
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::ActiveRegion(index) => {
                        Some(index)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "enter_region has no active region target".to_string(),
                    )
                })?;
            state.active_regions.push(region);
        }
        Opcode::LeaveRegion => {
            let region = instruction
                .resolved_operands()
                .iter()
                .find_map(|resolved| match resolved.target() {
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::ActiveRegion(index) => {
                        Some(index)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "leave_region has no active region target".to_string(),
                    )
                })?;
            if state.active_regions.pop() != Some(region) {
                return Err(obligation_error(
                    location,
                    "leave_region does not match the innermost active region".to_string(),
                ));
            }
        }
        Opcode::SetWritablePath => {
            let root_slot = instruction
                .resolved_operands()
                .iter()
                .find_map(|resolved| match resolved.target() {
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::FrameSlot(slot) => {
                        Some(slot)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "set_writable_path has no root slot target".to_string(),
                    )
                })?;
            let path = instruction
                .resolved_operands()
                .iter()
                .find_map(|resolved| match resolved.target() {
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::WritablePath(path) => {
                        Some(path)
                    }
                    _ => None,
                })
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        "set_writable_path has no writable path target".to_string(),
                    )
                })?;
            let loan = LinkedWritableLoanState::new(root_slot, path);
            if !state.writable_loans.contains(&loan) {
                state.writable_loans.push(loan);
                state.writable_loans.sort_unstable();
                state.writable_loans.dedup();
            }
        }
        _ => {}
    }
    Ok(())
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
        validate_input_group(
            context,
            instruction,
            group.value,
            &values,
            &inputs,
            location.clone(),
        )?;
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
    inputs: &[Vec<LinkedStackValue>],
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match source {
        ValueSource::AnyStackValue | ValueSource::TaggedValue => return Ok(()),
        ValueSource::ArrayValue | ValueSource::MapValue => {
            return Err(BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
                location,
            });
        }
        ValueSource::ComparablePair => {
            if matches!(actual, [left, right] if linked_value_matches(left, right)) {
                return Ok(());
            }
            return Err(obligation_error(
                location,
                "comparison inputs do not have one exact concrete type and plan".to_string(),
            ));
        }
        _ => {}
    }
    let mut expected =
        values::source_values(context, instruction, source, inputs, location.clone())?;
    if expected.len() == 1 && actual.len() > 1 {
        expected = actual.iter().map(|_| expected[0].clone()).collect();
    }
    if !linked_values_match(&expected, actual) {
        return Err(obligation_error(
            location,
            format!(
                "typed stack input {} differs from its exact concrete source: expected {expected:?}, actual {actual:?}",
                source.name(),
            ),
        ));
    }
    Ok(())
}

fn linked_values_match(expected: &[LinkedStackValue], actual: &[LinkedStackValue]) -> bool {
    expected.len() == actual.len()
        && expected
            .iter()
            .zip(actual)
            .all(|(expected, actual)| linked_value_matches(expected, actual))
}

fn linked_value_matches(expected: &LinkedStackValue, actual: &LinkedStackValue) -> bool {
    expected == actual
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
    context: &StackMapContext<'_, '_>,
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
                let value = validate_slot_write(context, slot, &value, location.clone())?;
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
    context: &StackMapContext<'_, '_>,
    slot: usize,
    value: &LinkedStackValue,
    location: BytecodeLinkLocation,
) -> Result<LinkedStackValue, BytecodeLinkError> {
    let expected_type = context
        .frame
        .slot_types()
        .get(slot)
        .copied()
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("frame slot type {slot} is out of bounds"),
            )
        })?;
    let expected_plan = context
        .frame
        .slot_plans()
        .get(slot)
        .cloned()
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("frame slot plan {slot} is out of bounds"),
            )
        })?;
    let expected = LinkedStackValue::new(expected_type, expected_plan);
    if !linked_value_matches(&expected, value) {
        let expected_type = context.type_linker.linked_type_ref(expected.ty());
        let actual_type = context.type_linker.linked_type_ref(value.ty());
        return Err(obligation_error(
            location,
            format!(
                "slot write type or lifecycle plan differs at slot {slot}: expected {expected:?} ({expected_type:?}), actual {value:?} ({actual_type:?})"
            ),
        ));
    }
    Ok(expected)
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

#[cfg(test)]
mod tests {
    use skiff_runtime_linked_bytecode::{
        LinkedStackValue, LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
    };

    use super::linked_value_matches;

    fn snapshot() -> LinkedValueTransferPlan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }
    }

    fn trivial() -> LinkedValueTransferPlan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    }

    #[test]
    fn transfer_requires_exact_type_index_and_lifecycle_plan() {
        let snapshot_value = LinkedStackValue::new(TypeIndex::new(7), snapshot());
        let same_snapshot_value = LinkedStackValue::new(TypeIndex::new(7), snapshot());
        let trivial_value = LinkedStackValue::new(TypeIndex::new(7), trivial());
        let other_type = LinkedStackValue::new(TypeIndex::new(8), snapshot());

        assert!(linked_value_matches(&snapshot_value, &same_snapshot_value));
        assert!(!linked_value_matches(&snapshot_value, &trivial_value));
        assert!(!linked_value_matches(&trivial_value, &snapshot_value));
        assert!(!linked_value_matches(&snapshot_value, &other_type));
    }
}

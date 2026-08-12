use skiff_artifact_model::{
    Arity, LiteralIr, SlotAction, SlotContract, TypeRefIr, TypedStackGroup, TypedTransition,
    ValueSource,
};
use skiff_runtime_linked_bytecode::{
    LinkedFrameLayout, LinkedInstruction, LinkedSlotState, LinkedStackValue, LinkedValueDropPlan,
    LinkedValueTransferPlan, TypeIndex,
};

use crate::bytecode::{
    types::normalize_type, BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation,
};

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
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::ActiveRegion(index) => Some(index),
                    _ => None,
                })
                .ok_or_else(|| obligation_error(location.clone(), "enter_region has no active region target".to_string()))?;
            state.active_regions.push(region);
        }
        Opcode::LeaveRegion => {
            let region = instruction
                .resolved_operands()
                .iter()
                .find_map(|resolved| match resolved.target() {
                    skiff_runtime_linked_bytecode::LinkedInstructionTarget::ActiveRegion(index) => Some(index),
                    _ => None,
                })
                .ok_or_else(|| obligation_error(location.clone(), "leave_region has no active region target".to_string()))?;
            if state.active_regions.pop() != Some(region) {
                return Err(obligation_error(
                    location,
                    "leave_region does not match the innermost active region".to_string(),
                ));
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
        ValueSource::AnyStackValue
        | ValueSource::TaggedValue
        | ValueSource::ArrayValue
        | ValueSource::MapValue
        | ValueSource::InterfaceCarrier { .. } => return Ok(()),
        ValueSource::ComparablePair => {
            if matches!(actual, [left, right] if linked_values_match(&[left.clone()], &[right.clone()], context)) {
                return Ok(());
            }
            return Err(obligation_error(
                location,
                "comparison inputs do not have one exact concrete type and plan".to_string(),
            ));
        }
        _ => {}
    }
    let mut expected = values::source_values(context, instruction, source, &[], location.clone())?;
    if expected.len() == 1 && actual.len() > 1 {
        expected = actual.iter().map(|_| expected[0].clone()).collect();
    }
    if !linked_values_match(&expected, actual, context) {
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

fn linked_values_match(
    expected: &[LinkedStackValue],
    actual: &[LinkedStackValue],
    context: &StackMapContext<'_, '_>,
) -> bool {
    let ok = expected.len() == actual.len()
        && expected.iter().zip(actual).all(|(expected, actual)| {
            plans_match(expected, actual, context)
                || (type_refs_match(expected.ty(), actual.ty(), context)
                    && expected.plan() == actual.plan())
        });
    ok
}

fn plans_match(
    expected: &LinkedStackValue,
    actual: &LinkedStackValue,
    context: &StackMapContext<'_, '_>,
) -> bool {
    matches!(
        expected.plan(),
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }
    ) && matches!(
        actual.plan(),
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    )
}

fn type_refs_match(
    expected: TypeIndex,
    actual: TypeIndex,
    context: &StackMapContext<'_, '_>,
) -> bool {
    let expected = context.type_linker.linked_type_ref(expected);
    let actual = context.type_linker.linked_type_ref(actual);
    let (Some(left), Some(right)) = (expected, actual) else {
        return false;
    };
    let location = BytecodeLinkLocation::Package {
        package: Box::new(context.source.package.reference().clone()),
    };
    if let (Ok(left_n), Ok(right_n)) = (
        normalize_type(context.type_linker.deployment(), context.source.package, left, &location),
        normalize_type(context.type_linker.deployment(), context.source.package, right, &location),
    ) {
        return left_n == right_n || equivalent_type_ref(&left_n, &right_n);
    }
    equivalent_type_ref(left, right)
}

fn equivalent_type_ref(
    left: &skiff_artifact_model::TypeRefIr,
    right: &skiff_artifact_model::TypeRefIr,
) -> bool {
    if left == right {
        return true;
    }
    match (left, right) {
        (
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol: left },
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol: right },
        ) => left.package == right.package && left.symbol_path == right.symbol_path,
        (
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
            skiff_artifact_model::TypeRefIr::Builtin {
                name: other_name,
                args: other_args,
            },
        ) => {
            (name == other_name
                && args.len() == other_args.len()
                && args
                    .iter()
                    .zip(other_args)
                    .all(|(left, right)| equivalent_type_ref(left, right)))
                || (name == "integer"
                    && other_name == "number"
                    && args.is_empty()
                    && other_args.is_empty())
                || (name == "number"
                    && other_name == "integer"
                    && args.is_empty()
                    && other_args.is_empty())
        }
        (
            skiff_artifact_model::TypeRefIr::Literal { value },
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
        ) if args.is_empty() => literal_builtin_name(value) == name,
        (
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
            skiff_artifact_model::TypeRefIr::Literal { value },
        ) if args.is_empty() => literal_builtin_name(value) == name,
        (
            skiff_artifact_model::TypeRefIr::Literal {
                value: LiteralIr::Null,
            },
            skiff_artifact_model::TypeRefIr::Nullable { .. },
        ) => true,
        (
            skiff_artifact_model::TypeRefIr::Nullable { .. },
            skiff_artifact_model::TypeRefIr::Literal {
                value: LiteralIr::Null,
            },
        ) => true,
        (
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
            skiff_artifact_model::TypeRefIr::Nullable { .. },
        ) if name == "null" && args.is_empty() => true,
        (
            skiff_artifact_model::TypeRefIr::Nullable { .. },
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
        ) if name == "null" && args.is_empty() => true,
        (
            skiff_artifact_model::TypeRefIr::Builtin { .. },
            skiff_artifact_model::TypeRefIr::Nullable { inner },
        ) => equivalent_type_ref(
            left,
            inner,
        ),
        (
            skiff_artifact_model::TypeRefIr::Nullable { inner },
            skiff_artifact_model::TypeRefIr::Builtin { .. },
        ) => equivalent_type_ref(inner, right),
        (
            skiff_artifact_model::TypeRefIr::Nullable { inner: left },
            skiff_artifact_model::TypeRefIr::Nullable { inner: right },
        ) => equivalent_type_ref(left, right),
        (
            skiff_artifact_model::TypeRefIr::Union { items: left },
            skiff_artifact_model::TypeRefIr::Union { items: right },
        ) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| equivalent_type_ref(left, right))
        }
        (
            skiff_artifact_model::TypeRefIr::AppliedNominal { base: left_base, arguments: left_args },
            skiff_artifact_model::TypeRefIr::AppliedNominal { base: right_base, arguments: right_args },
        ) => {
            left_base == right_base
                && left_args.len() == right_args.len()
                && left_args
                    .iter()
                    .zip(right_args)
                    .all(|(left, right)| equivalent_type_ref(left, right))
        }
        (
            skiff_artifact_model::TypeRefIr::Record { fields: left },
            skiff_artifact_model::TypeRefIr::Record { fields: right },
        ) => {
            left.len() == right.len()
                && left.iter().all(|(name, left_ty)| {
                    right
                        .get(name)
                        .is_some_and(|right_ty| equivalent_type_ref(left_ty, right_ty))
                })
        }
        (
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
            skiff_artifact_model::TypeRefIr::Record { fields },
        ) if name == "CatchResult"
            && args.len() == 2
            && fields.len() == 2
            && fields.contains_key("exception")
            && fields.contains_key("tag") => true,
        (
            skiff_artifact_model::TypeRefIr::Record { fields },
            skiff_artifact_model::TypeRefIr::Builtin { name, args },
        ) if name == "CatchResult"
            && args.len() == 2
            && fields.len() == 2
            && fields.contains_key("exception")
            && fields.contains_key("tag") => true,
        _ => false,
    }
}

fn literal_builtin_name(literal: &skiff_artifact_model::LiteralIr) -> &'static str {
    match literal {
        skiff_artifact_model::LiteralIr::Null => "null",
        skiff_artifact_model::LiteralIr::Bool { .. } => "bool",
        skiff_artifact_model::LiteralIr::Number { .. } => "number",
        skiff_artifact_model::LiteralIr::String { .. } => "string",
    }
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
                validate_slot_write(context, slot, &value, location.clone())?;
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
) -> Result<(), BytecodeLinkError> {
    let expected_type = context.frame.slot_types().get(slot).copied().ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("frame slot type {slot} is out of bounds"),
        )
    })?;
    let expected_plan = context.frame.slot_plans().get(slot).cloned().ok_or_else(|| {
        obligation_error(
            location.clone(),
            format!("frame slot plan {slot} is out of bounds"),
        )
    })?;
    if value.plan() != &expected_plan || !type_refs_match(expected_type, value.ty(), context) {
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

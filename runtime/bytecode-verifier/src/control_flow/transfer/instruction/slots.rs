use skiff_artifact_model::{SlotAction, SlotContract, ValueSource};
use skiff_runtime_linked_bytecode::FrameSlotIndex;

use super::{unavailable, values, violation, Context};
use crate::{
    control_flow::{AbstractSlotState, AbstractValue, ProgramPointState},
    VerificationError,
};

pub(super) fn apply(
    before: &ProgramPointState,
    inputs: &[Vec<AbstractValue>],
    context: &Context<'_>,
) -> Result<Vec<AbstractSlotState>, VerificationError> {
    let mut slots = before.slots.to_vec();
    let SlotContract::Effects(effects) = context.contract.typed.slots else {
        return match context.contract.typed.slots {
            SlotContract::None => Ok(slots),
            SlotContract::InOutCallLoans { .. } => Err(unavailable(context.location)),
            SlotContract::Effects(_) => unreachable!(),
        };
    };
    for effect in effects {
        let slot = values::resolve_slot(context, effect.operand)?;
        match effect.action {
            SlotAction::Read | SlotAction::ReadShare | SlotAction::Take | SlotAction::Drop => {
                if effect.value
                    != (ValueSource::Slot {
                        operand: effect.operand,
                    })
                {
                    return Err(unavailable(context.location));
                }
                let value = values::live_slot(before, slot, context.location)?;
                if effect.action == SlotAction::ReadShare {
                    values::require_shareable(value, context.facts, context.location)?;
                }
                if matches!(effect.action, SlotAction::Take | SlotAction::Drop) {
                    values::set_slot(&mut slots, slot, AbstractSlotState::Moved, context.location)?;
                }
            }
            SlotAction::Write => {
                let value = resolve_value(effect.value, before, inputs, context)?;
                write_slot(before, &mut slots, slot, value, context)?;
            }
            SlotAction::Mutate => return Err(unavailable(context.location)),
        }
    }
    Ok(slots)
}

fn resolve_value(
    source: ValueSource,
    before: &ProgramPointState,
    inputs: &[Vec<AbstractValue>],
    context: &Context<'_>,
) -> Result<AbstractValue, VerificationError> {
    match source {
        ValueSource::Slot { operand } => values::live_slot(
            before,
            values::resolve_slot(context, operand)?,
            context.location,
        ),
        ValueSource::StackInput { group } => inputs
            .get(group as usize)
            .and_then(|values| (values.len() == 1).then_some(values[0]))
            .ok_or_else(|| {
                violation(
                    context.location,
                    "slot write input is not one concrete value",
                )
            }),
        _ => Err(unavailable(context.location)),
    }
}

fn write_slot(
    before: &ProgramPointState,
    slots: &mut [AbstractSlotState],
    slot: FrameSlotIndex,
    value: AbstractValue,
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    let prior = before
        .slots
        .get(slot.get() as usize)
        .ok_or_else(|| violation(context.location, "slot write destination is out of bounds"))?;
    if !matches!(prior, AbstractSlotState::Uninitialized)
        && context
            .function
            .frame()
            .writable_local_slots()
            .binary_search(&slot)
            .is_err()
    {
        return Err(violation(
            context.location,
            format!(
                "slot {} overwrite is not authorized by writable locals",
                slot.get()
            ),
        ));
    }
    let declared = context
        .function
        .frame()
        .slot_types()
        .get(slot.get() as usize)
        .copied()
        .ok_or_else(|| violation(context.location, "slot write destination is out of bounds"))?;
    values::require_same_type(
        value,
        declared,
        context.facts,
        context.location,
        "slot write",
    )?;
    let AbstractValue::Concrete(value_ty) = value;
    let merged = context
        .facts
        .merge_coordinate(value_ty, declared)
        .map_err(|_| violation(context.location, "slot write has no exact class coordinate"))?;
    values::set_slot(
        slots,
        slot,
        AbstractSlotState::Live(merged),
        context.location,
    )
}

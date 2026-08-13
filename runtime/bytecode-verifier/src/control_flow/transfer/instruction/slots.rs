use skiff_artifact_model::{Opcode, SlotAction, SlotContract, ValueSource};
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
                let value = match effect.value {
                    ValueSource::Slot { operand } if operand == effect.operand => {
                        values::live_slot(before, slot, context.location)?
                    }
                    // `Rethrow` reads its exception slot as the opaque
                    // `Exception<E>` envelope; the same provenance check as
                    // the stack-input path, without touching the slot.
                    ValueSource::ExceptionEnvelope { source_slot }
                        if matches!(effect.action, SlotAction::Read) =>
                    {
                        let source = values::resolve_slot(context, source_slot)?;
                        let value = values::live_slot(before, source, context.location)?;
                        values::require_exception_envelope(value, context, context.location)?;
                        value
                    }
                    _ => return Err(unavailable(context.location)),
                };
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
            SlotAction::Mutate => {
                if effect.value
                    != (ValueSource::Slot {
                        operand: effect.operand,
                    })
                {
                    return Err(unavailable(context.location));
                }
                let AbstractValue::Concrete(owner) =
                    values::live_slot(before, slot, context.location)?;
                match context.instruction.opcode() {
                    Opcode::StreamNext => {
                        context
                            .facts
                            .stream_item_type(owner, context.location)
                            .map_err(|_| {
                                violation(
                                    context.location,
                                    "mutated endpoint is not an affine Stream<T> slot",
                                )
                            })?;
                    }
                    Opcode::ArrayPushOwned | Opcode::MapPutOwned => {
                        let _ = owner;
                    }
                    Opcode::SetWritablePath => {
                        let path = values::resolve_path(
                            context,
                            skiff_artifact_model::OperandRole::WritablePathRef,
                        )?;
                        let row = context
                            .candidate
                            .writable_paths()
                            .get(path.get() as usize)
                            .filter(|row| row.index() == path)
                            .ok_or_else(|| {
                                violation(context.location, "writable path is out of bounds")
                            })?;
                        values::require_same_type(
                            AbstractValue::Concrete(owner),
                            row.root_type(),
                            context.facts,
                            context.location,
                            "writable path root",
                        )?;
                    }
                    _ => return Err(unavailable(context.location)),
                }
            }
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
    values::require_assignable(
        value,
        declared,
        context.facts,
        context.location,
        "slot write",
    )?;
    let AbstractValue::Concrete(value_ty) = value;
    context
        .facts
        .merge_coordinate(value_ty, declared)
        .map_err(|_| violation(context.location, "slot write has no exact class coordinate"))?;
    values::set_slot(
        slots,
        slot,
        AbstractSlotState::Live(declared),
        context.location,
    )
}

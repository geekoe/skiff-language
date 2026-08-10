use skiff_artifact_model::{NativeValueEmbedding, NativeValueLifecycleConcrete, OperandRole};
use skiff_runtime_linked_bytecode::{
    ConstantIndex, FrameSlotIndex, LinkedInstructionTarget, TypeIndex,
};

use super::{unavailable, violation, Context};
use crate::{
    concrete_values::{ConcreteValueFacts, ImplicitBuiltin},
    control_flow::{AbstractSlotState, AbstractValue, ProgramPointState},
    VerificationError, VerificationLocation,
};

pub(super) fn require_implicit(
    values: &[AbstractValue],
    facts: &ConcreteValueFacts,
    builtin: ImplicitBuiltin,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected = facts
        .implicit_representative(builtin)
        .ok_or_else(|| violation(location, "implicit builtin has no unique concrete class"))?;
    values.iter().try_for_each(|value| {
        require_same_type(*value, expected, facts, location, "implicit builtin input")
    })
}

pub(super) fn singleton_implicit(
    count: usize,
    facts: &ConcreteValueFacts,
    builtin: ImplicitBuiltin,
    location: VerificationLocation,
) -> Result<Vec<AbstractValue>, VerificationError> {
    if count != 1 {
        return Err(unavailable(location));
    }
    let ty = facts
        .implicit_representative(builtin)
        .ok_or_else(|| violation(location, "implicit builtin has no unique concrete class"))?;
    Ok(vec![AbstractValue::Concrete(ty)])
}

pub(super) fn require_shareable(
    value: AbstractValue,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(ty) = value;
    let fact = facts
        .type_fact(ty)
        .ok_or_else(|| violation(location, "shareable value has no concrete fact"))?;
    if !matches!(
        &fact.lifecycle().lifecycle,
        NativeValueLifecycleConcrete::SnapshotShare { .. }
    ) {
        return Err(violation(
            location,
            format!("type {} is not independently proven shareable", ty.get()),
        ));
    }
    Ok(())
}

pub(super) fn require_same_type(
    actual: AbstractValue,
    expected: TypeIndex,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
    owner: impl AsRef<str>,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(actual) = actual;
    if facts.semantically_equal(actual, expected) != Some(true) {
        return Err(violation(
            location,
            format!(
                "{} type {} differs from expected type {}",
                owner.as_ref(),
                actual.get(),
                expected.get()
            ),
        ));
    }
    Ok(())
}

pub(super) fn require_concrete_fact(
    value: AbstractValue,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(ty) = value;
    require_type_fact(ty, facts, location)
}

pub(super) fn require_type_fact(
    ty: TypeIndex,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    facts
        .type_fact(ty)
        .map(drop)
        .ok_or_else(|| violation(location, format!("type {} has no concrete fact", ty.get())))
}

pub(super) fn require_constant_materializable(
    ty: TypeIndex,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let fact = facts
        .type_fact(ty)
        .ok_or_else(|| violation(location, "constant type has no concrete fact"))?;
    if fact.lifecycle().embedding != NativeValueEmbedding::Ordinary
        || !matches!(
            &fact.lifecycle().lifecycle,
            NativeValueLifecycleConcrete::SnapshotShare { .. }
        )
    {
        return Err(violation(
            location,
            format!(
                "constant type {} is not an Ordinary SnapshotShare value",
                ty.get()
            ),
        ));
    }
    Ok(())
}

pub(super) fn live_slot(
    state: &ProgramPointState,
    slot: FrameSlotIndex,
    location: VerificationLocation,
) -> Result<AbstractValue, VerificationError> {
    match state.slots.get(slot.get() as usize) {
        Some(AbstractSlotState::Live(ty)) => Ok(AbstractValue::Concrete(*ty)),
        Some(AbstractSlotState::Moved) => Err(violation(
            location,
            format!("slot {} was already moved", slot.get()),
        )),
        Some(AbstractSlotState::Uninitialized) => Err(violation(
            location,
            format!("slot {} is uninitialized", slot.get()),
        )),
        None => Err(violation(location, "slot operand is out of bounds")),
    }
}

pub(super) fn set_slot(
    slots: &mut [AbstractSlotState],
    slot: FrameSlotIndex,
    state: AbstractSlotState,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let destination = slots
        .get_mut(slot.get() as usize)
        .ok_or_else(|| violation(location, "slot operand is out of bounds"))?;
    *destination = state;
    Ok(())
}

pub(super) fn resolve_slot(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<FrameSlotIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::FrameSlot(slot) => Ok(slot),
        _ => Err(violation(
            context.location,
            "slot role has a non-slot typed target",
        )),
    }
}

pub(super) fn resolve_constant(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<ConstantIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::Constant(constant) => Ok(constant),
        _ => Err(violation(
            context.location,
            "constant role has a non-constant typed target",
        )),
    }
}

fn resolved_target(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<LinkedInstructionTarget, VerificationError> {
    let ordinal = context
        .contract
        .operand_position(role)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| violation(context.location, "canonical operand role is absent"))?;
    context
        .instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| violation(context.location, "typed operand target is absent"))
}

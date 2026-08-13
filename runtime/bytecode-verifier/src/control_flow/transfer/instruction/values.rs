use skiff_artifact_model::{
    NativeValueEmbedding, NativeValueLifecycleConcrete, OperandRole, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    CallbackCaptureLayoutIndex, ConstantIndex, FrameSlotIndex, InterfaceTableIndex,
    LinkedContainerLayoutKind, LinkedInstructionTarget, LinkedInterfaceTableKind,
    LinkedWritablePathSegment, ShapeIndex, SyntheticCallbackIndex, TypeIndex, WritablePathIndex,
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
                "{} type {} differs from expected type {}: actual {:?}, expected {:?}",
                owner.as_ref(),
                actual.get(),
                expected.get(),
                facts.type_fact(actual).map(|fact| (fact.normalized_type(), fact.lifecycle())),
                facts.type_fact(expected).map(|fact| (fact.normalized_type(), fact.lifecycle())),
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

pub(super) fn require_integer_or_number(
    value: AbstractValue,
    facts: &ConcreteValueFacts,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(ty) = value;
    let fact = facts
        .type_fact(ty)
        .ok_or_else(|| violation(location, "collection index has no concrete fact"))?;
    let valid = matches!(
        fact.normalized_type(),
        TypeRefIr::Builtin { name, args }
            if (name == "integer" || name == "number") && args.is_empty()
    );
    if !valid {
        return Err(violation(
            location,
            format!("type {} is not a collection index", ty.get()),
        ));
    }
    Ok(())
}

pub(super) fn require_container(
    value: AbstractValue,
    context: &Context<'_>,
    kind: LinkedContainerLayoutKind,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let AbstractValue::Concrete(ty) = value;
    let row = context
        .candidate
        .types()
        .get(ty.get() as usize)
        .filter(|row| row.index() == ty)
        .ok_or_else(|| {
            violation(
                location,
                format!("container type {} has no linked row", ty.get()),
            )
        })?;
    let layout = row
        .container_layout()
        .filter(|layout| layout.kind() == kind)
        .ok_or_else(|| violation(location, "value is not the exact container layout"))?;
    let child = match kind {
        LinkedContainerLayoutKind::Array => layout.element(),
        LinkedContainerLayoutKind::Map => layout.value(),
        LinkedContainerLayoutKind::Json | LinkedContainerLayoutKind::JsonObject => {
            return Err(unavailable(location));
        }
    }
    .ok_or_else(|| violation(location, "container layout is missing its child position"))?;
    require_type_fact(child.ty(), context.facts, location)?;
    Ok(child.ty())
}

pub(super) fn require_map_key(
    value: AbstractValue,
    context: &Context<'_>,
    map: AbstractValue,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let AbstractValue::Concrete(map_ty) = map;
    let row = context
        .candidate
        .types()
        .get(map_ty.get() as usize)
        .filter(|row| row.index() == map_ty)
        .ok_or_else(|| violation(location, "map value has no linked row"))?;
    let layout = row
        .container_layout()
        .filter(|layout| layout.kind() == LinkedContainerLayoutKind::Map)
        .ok_or_else(|| violation(location, "map value has the wrong container layout"))?;
    let key = layout
        .key()
        .ok_or_else(|| violation(location, "map layout has no key position"))?;
    require_same_type(value, key.ty(), context.facts, location, "map key")?;
    Ok(key.ty())
}

pub(super) fn map_key_type(
    context: &Context<'_>,
    map: AbstractValue,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let AbstractValue::Concrete(map_ty) = map;
    let row = context
        .candidate
        .types()
        .get(map_ty.get() as usize)
        .filter(|row| row.index() == map_ty)
        .ok_or_else(|| violation(location, "map value has no linked row"))?;
    let layout = row
        .container_layout()
        .filter(|layout| layout.kind() == LinkedContainerLayoutKind::Map)
        .ok_or_else(|| violation(location, "map value has the wrong container layout"))?;
    let key = layout
        .key()
        .ok_or_else(|| violation(location, "map layout has no key position"))?;
    require_type_fact(key.ty(), context.facts, location)?;
    Ok(key.ty())
}

pub(super) fn map_value_type(
    context: &Context<'_>,
    map: AbstractValue,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let AbstractValue::Concrete(map_ty) = map;
    let row = context
        .candidate
        .types()
        .get(map_ty.get() as usize)
        .filter(|row| row.index() == map_ty)
        .ok_or_else(|| violation(location, "map value has no linked row"))?;
    let layout = row
        .container_layout()
        .filter(|layout| layout.kind() == LinkedContainerLayoutKind::Map)
        .ok_or_else(|| violation(location, "map value has the wrong container layout"))?;
    let value = layout
        .value()
        .ok_or_else(|| violation(location, "map layout has no value position"))?;
    require_type_fact(value.ty(), context.facts, location)?;
    Ok(value.ty())
}
pub(super) fn require_map_element(
    value: AbstractValue,
    context: &Context<'_>,
    map: AbstractValue,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let AbstractValue::Concrete(map_ty) = map;
    let row = context
        .candidate
        .types()
        .get(map_ty.get() as usize)
        .filter(|row| row.index() == map_ty)
        .ok_or_else(|| violation(location, "map value has no linked row"))?;
    let layout = row
        .container_layout()
        .filter(|layout| layout.kind() == LinkedContainerLayoutKind::Map)
        .ok_or_else(|| violation(location, "map value has the wrong container layout"))?;
    let value_position = layout
        .value()
        .ok_or_else(|| violation(location, "map layout has no value position"))?;
    require_same_type(
        value,
        value_position.ty(),
        context.facts,
        location,
        "map value",
    )?;
    Ok(value_position.ty())
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

pub(super) fn shape_value_type(
    context: &Context<'_>,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let shape = resolve_shape(context, role)?;
    let row = context
        .candidate
        .shapes()
        .get(shape.get() as usize)
        .filter(|row| row.index() == shape)
        .ok_or_else(|| violation(location, "shape target is out of bounds"))?;
    require_type_fact(row.nominal_type(), context.facts, location)?;
    Ok(row.nominal_type())
}

pub(super) fn shape_field_type(
    context: &Context<'_>,
    shape_role: OperandRole,
    ordinal_role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let shape = resolve_shape(context, shape_role)?;
    let ordinal = context
        .contract
        .operand_word(ordinal_role, context.instruction.operands())
        .ok_or_else(|| violation(location, "shape field ordinal is absent"))?;
    let row = context
        .candidate
        .shapes()
        .get(shape.get() as usize)
        .filter(|row| row.index() == shape)
        .ok_or_else(|| violation(location, "shape target is out of bounds"))?;
    let field = row
        .fields()
        .get(ordinal as usize)
        .ok_or_else(|| violation(location, "shape field ordinal is out of bounds"))?;
    require_type_fact(field.ty(), context.facts, location)?;
    Ok(field.ty())
}

pub(super) fn interface_carrier_type(
    context: &Context<'_>,
    interface_role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let table_index = resolve_interface(context, interface_role)?;
    let table = context
        .candidate
        .interface_tables()
        .get(table_index.get() as usize)
        .filter(|table| table.index() == table_index)
        .ok_or_else(|| violation(location, "interface table is out of bounds"))?;
    if let LinkedInterfaceTableKind::Local(local) = table.kind() {
        require_type_fact(local.concrete_type(), context.facts, location)?;
        return Ok(local.concrete_type());
    }
    let target = table.interface().artifact();
    context
        .candidate
        .types()
        .iter()
        .find(|row| {
            matches!(
                row.type_ref(),
                TypeRefIr::AnyInterface { interface } if interface == target
            )
        })
        .map(|row| row.index())
        .ok_or_else(|| unavailable(location))
}

pub(super) fn writable_path_selectors(
    context: &Context<'_>,
    path_role: OperandRole,
    location: VerificationLocation,
) -> Result<Vec<TypeIndex>, VerificationError> {
    let path = resolve_path(context, path_role)?;
    let row = context
        .candidate
        .writable_paths()
        .get(path.get() as usize)
        .filter(|row| row.index() == path)
        .ok_or_else(|| violation(location, "writable path is out of bounds"))?;
    let mut selectors = Vec::new();
    for segment in row.segments() {
        match segment {
            LinkedWritablePathSegment::DenseField { .. } => {}
            LinkedWritablePathSegment::ArrayIndex {
                selector_ordinal, ..
            } => {
                require_selector_ordinal(*selector_ordinal, selectors.len(), location)?;
                // Array index selectors follow the same authority as the
                // canonical `CollectionIndex` input: integer-or-number. Using
                // the number representative accepts an integer selector as
                // well because semantic equality admits integer/number
                // interchangeably.
                let number = context
                    .facts
                    .implicit_representative(ImplicitBuiltin::Number)
                    .ok_or_else(|| violation(location, "number selector has no concrete class"))?;
                selectors.push(number);
            }
            LinkedWritablePathSegment::MapKey {
                selector_ordinal,
                key_type,
                ..
            } => {
                require_selector_ordinal(*selector_ordinal, selectors.len(), location)?;
                require_type_fact(*key_type, context.facts, location)?;
                selectors.push(*key_type);
            }
        }
    }
    Ok(selectors)
}

pub(super) fn writable_path_leaf_type(
    context: &Context<'_>,
    path_role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let path = resolve_path(context, path_role)?;
    let row = context
        .candidate
        .writable_paths()
        .get(path.get() as usize)
        .filter(|row| row.index() == path)
        .ok_or_else(|| violation(location, "writable path is out of bounds"))?;
    require_type_fact(row.leaf_type(), context.facts, location)?;
    Ok(row.leaf_type())
}

pub(super) fn exception_payload_type(
    context: &Context<'_>,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let ty = resolve_type(context, role)?;
    require_type_fact(ty, context.facts, location)?;
    Ok(ty)
}

pub(super) fn require_exception_envelope(
    value: AbstractValue,
    context: &Context<'_>,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let AbstractValue::Concrete(ty) = value;
    let fact = context
        .facts
        .type_fact(ty)
        .ok_or_else(|| violation(location, "exception envelope has no concrete fact"))?;
    let is_envelope = matches!(
        fact.normalized_type(),
        TypeRefIr::Builtin { name, args }
            if name == "Exception" && args.len() == 1
    );
    if !is_envelope {
        return Err(violation(
            location,
            "rethrow source is not an Exception envelope",
        ));
    }
    Ok(())
}

pub(super) fn resolve_type(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<TypeIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::Type(ty) => Ok(ty),
        _ => Err(violation(
            context.location,
            "type role has a non-type typed target",
        )),
    }
}

pub(super) fn resolve_shape(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<ShapeIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::Shape(shape) => Ok(shape),
        _ => Err(violation(
            context.location,
            "shape role has a non-shape typed target",
        )),
    }
}

pub(super) fn resolve_path(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<WritablePathIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::WritablePath(path) => Ok(path),
        _ => Err(violation(
            context.location,
            "writable-path role has a non-path typed target",
        )),
    }
}

fn resolve_interface(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<InterfaceTableIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::InterfaceTable(table) => Ok(table),
        _ => Err(violation(
            context.location,
            "interface role has a non-interface typed target",
        )),
    }
}

fn require_selector_ordinal(
    actual: u32,
    expected: usize,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
    let expected_u32 = u32::try_from(expected)
        .map_err(|_| violation(location, "writable selector ordinal overflows u32"))?;
    if actual != expected_u32 {
        return Err(violation(
            location,
            format!("writable selector ordinal {actual} is not dense {expected_u32}"),
        ));
    }
    Ok(())
}

pub(super) fn array_type_for_element(
    context: &Context<'_>,
    element: TypeIndex,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    context
        .candidate
        .types()
        .iter()
        .find(|row| {
            row.container_layout()
                .and_then(|layout| layout.element())
                .is_some_and(|position| {
                    context.facts.semantically_equal(position.ty(), element) == Some(true)
                })
        })
        .map(|row| row.index())
        .ok_or_else(|| unavailable(location))
}

pub(super) fn map_type_for_key_value(
    context: &Context<'_>,
    key: TypeIndex,
    value: TypeIndex,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    context
        .candidate
        .types()
        .iter()
        .find(|row| {
            row.container_layout()
                .filter(|layout| layout.kind() == LinkedContainerLayoutKind::Map)
                .is_some_and(|layout| {
                    layout
                        .key()
                        .zip(layout.value())
                        .is_some_and(|(left, right)| {
                            context.facts.semantically_equal(left.ty(), key) == Some(true)
                                && context.facts.semantically_equal(right.ty(), value) == Some(true)
                        })
                })
        })
        .map(|row| row.index())
        .ok_or_else(|| unavailable(location))
}

pub(super) fn resolve_capture_layout(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<CallbackCaptureLayoutIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::CallbackCaptureLayout(layout) => Ok(layout),
        _ => Err(violation(
            context.location,
            "capture-layout role has a non-layout typed target",
        )),
    }
}

pub(super) fn callback_closure_type(
    context: &Context<'_>,
    callback_role: OperandRole,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let callback = resolve_callback(context, callback_role)?;
    let target = context
        .candidate
        .synthetic_callbacks()
        .get(callback.get() as usize)
        .filter(|target| target.index() == callback)
        .ok_or_else(|| violation(location, "callback target is out of bounds"))?;
    let interface = target
        .interface_method()
        .ok_or_else(|| unavailable(location))?;
    interface_carrier_type_for_table(context, interface.interface_table(), location)
}

pub(super) fn interface_carrier_type_for_table(
    context: &Context<'_>,
    table_index: InterfaceTableIndex,
    location: VerificationLocation,
) -> Result<TypeIndex, VerificationError> {
    let table = context
        .candidate
        .interface_tables()
        .get(table_index.get() as usize)
        .filter(|table| table.index() == table_index)
        .ok_or_else(|| violation(location, "interface table is out of bounds"))?;
    if let LinkedInterfaceTableKind::Local(local) = table.kind() {
        require_type_fact(local.concrete_type(), context.facts, location)?;
        return Ok(local.concrete_type());
    }
    let target = table.interface().artifact();
    context
        .candidate
        .types()
        .iter()
        .find(|row| {
            matches!(
                row.type_ref(),
                TypeRefIr::AnyInterface { interface } if interface == target
            )
        })
        .map(|row| row.index())
        .ok_or_else(|| unavailable(location))
}

fn resolve_callback(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<SyntheticCallbackIndex, VerificationError> {
    match resolved_target(context, role)? {
        LinkedInstructionTarget::SyntheticCallback(callback) => Ok(callback),
        _ => Err(violation(
            context.location,
            "callback role has a non-callback typed target",
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

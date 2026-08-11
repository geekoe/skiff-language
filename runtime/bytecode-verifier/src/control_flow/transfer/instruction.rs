mod slots;
mod tail;
mod values;

use skiff_artifact_model::{
    contract_for_opcode, Arity, ControlContract, Opcode, OpcodeContract, OperandRole, ParamModeIr,
    TypedStackGroup, ValueSource,
};
use skiff_runtime_linked_bytecode::{
    ActiveRegionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedContainerLayoutKind,
    LinkedFunction, LinkedInstruction, LinkedInstructionTarget, TypeIndex,
};

use super::super::{
    tail::VerifiedTailCallProof, AbstractValue, AbstractWritableLoan, ProgramPointState,
};
use super::ExactTargetAndCallFacts;
use crate::{
    concrete_values::{ConcreteValueFacts, ImplicitBuiltin},
    VerificationError, VerificationLocation, VerificationObligation,
};

struct CallTypes {
    parameters: Box<[TypeIndex]>,
    results: Box<[TypeIndex]>,
}

struct Context<'a> {
    candidate: &'a LinkedBytecodeCandidate,
    function: &'a LinkedFunction,
    instruction: &'a LinkedInstruction,
    instruction_index: InstructionIndex,
    contract: &'static OpcodeContract,
    call: Option<CallTypes>,
    facts: &'a ConcreteValueFacts,
    location: VerificationLocation,
}

pub(super) enum InstructionTransfer {
    Continue(ProgramPointState),
    ContinueDual(ProgramPointState, ProgramPointState),
    Tail(VerifiedTailCallProof),
}

pub(super) fn apply(
    candidate: &LinkedBytecodeCandidate,
    function: &LinkedFunction,
    instruction_index: InstructionIndex,
    before: &ProgramPointState,
    facts: &ConcreteValueFacts,
    targets: &ExactTargetAndCallFacts,
) -> Result<InstructionTransfer, VerificationError> {
    let location = VerificationLocation::Instruction {
        function: function.index(),
        instruction: instruction_index,
    };
    let instruction = function
        .instructions()
        .get(instruction_index.get() as usize)
        .ok_or_else(|| violation(location, "instruction coordinate is out of bounds"))?;
    let exact_call = targets.call_plan(function.index(), instruction_index);
    let call = exact_call
        .map(|plan| {
            if plan
                .parameters()
                .iter()
                .any(|parameter| parameter.mode() != ParamModeIr::Value)
            {
                return Err(unavailable(location));
            }
            Ok(CallTypes {
                parameters: plan
                    .parameters()
                    .iter()
                    .map(|parameter| parameter.ty())
                    .collect(),
                results: plan.results().iter().map(|result| result.ty()).collect(),
            })
        })
        .transpose()?;
    let context = Context {
        candidate,
        function,
        instruction,
        instruction_index,
        contract: contract_for_opcode(instruction.opcode()),
        call,
        facts,
        location,
    };

    if instruction.opcode() == Opcode::TailCallLocal {
        let plan = exact_call.ok_or_else(|| tail::unavailable(location))?;
        return tail::prove(before, &context, plan).map(InstructionTransfer::Tail);
    }
    if instruction.opcode() == Opcode::StreamNext {
        return apply_stream_next(before, &context);
    }
    require_supported(instruction.opcode(), location)?;

    let (mut stack, inputs) = consume_inputs(before, &context)?;
    let next_slots = slots::apply(before, &inputs, &context)?;
    produce_outputs(&mut stack, before, &inputs, &context)?;
    let mut active_regions = before.active_regions.to_vec();
    let mut writable_loans = before.writable_loans.to_vec();
    apply_region_and_loan_effects(&context, &mut active_regions, &mut writable_loans)?;
    Ok(InstructionTransfer::Continue(ProgramPointState {
        stack: stack.into_boxed_slice(),
        slots: next_slots.into_boxed_slice(),
        active_regions: active_regions.into_boxed_slice(),
        writable_loans: writable_loans.into_boxed_slice(),
    }))
}

fn apply_stream_next(
    before: &ProgramPointState,
    context: &Context<'_>,
) -> Result<InstructionTransfer, VerificationError> {
    let (stack, _inputs) = consume_inputs(before, context)?;
    let endpoint_slot = values::resolve_slot(context, OperandRole::Slot)?;
    let endpoint = values::live_slot(before, endpoint_slot, context.location)?;
    let AbstractValue::Concrete(endpoint) = endpoint;
    let item = context
        .facts
        .stream_item_type(endpoint, context.location)?;
    let mut slots = before.slots.to_vec();
    values::set_slot(
        &mut slots,
        endpoint_slot,
        crate::control_flow::AbstractSlotState::Moved,
        context.location,
    )?;
    let mut item_stack = stack.clone();
    item_stack.push(AbstractValue::Concrete(item));
    Ok(InstructionTransfer::ContinueDual(
        ProgramPointState {
            stack: item_stack.into_boxed_slice(),
            slots: slots.clone().into_boxed_slice(),
            active_regions: before.active_regions.clone(),
            writable_loans: before.writable_loans.clone(),
        },
        ProgramPointState {
            stack: stack.into_boxed_slice(),
            slots: slots.into_boxed_slice(),
            active_regions: before.active_regions.clone(),
            writable_loans: before.writable_loans.clone(),
        },
    ))
}

fn apply_region_and_loan_effects(
    context: &Context<'_>,
    active_regions: &mut Vec<ActiveRegionIndex>,
    writable_loans: &mut Vec<AbstractWritableLoan>,
) -> Result<(), VerificationError> {
    match context.instruction.opcode() {
        Opcode::EnterRegion => {
            let region = region_target(context, OperandRole::ActiveRegion)?;
            if active_regions.contains(&region) {
                return Err(violation(
                    context.location,
                    format!("active region {} is already entered", region.get()),
                ));
            }
            active_regions.push(region);
        }
        Opcode::LeaveRegion => {
            let region = region_target(context, OperandRole::ActiveRegion)?;
            if active_regions.last().copied() != Some(region) {
                return Err(violation(
                    context.location,
                    format!(
                        "leave region {} is not the innermost active region",
                        region.get()
                    ),
                ));
            }
            active_regions.pop();
        }
        Opcode::SetWritablePath => {
            let root_slot = values::resolve_slot(context, OperandRole::Slot)?;
            let path = values::resolve_path(context, OperandRole::WritablePathRef)?;
            let loan = AbstractWritableLoan { root_slot, path };
            if !writable_loans.contains(&loan) {
                writable_loans.push(loan);
                writable_loans.sort_unstable();
                writable_loans.dedup();
            }
        }
        _ => {}
    }
    Ok(())
}

fn region_target(
    context: &Context<'_>,
    role: OperandRole,
) -> Result<ActiveRegionIndex, VerificationError> {
    let ordinal = context
        .contract
        .operand_position(role)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| violation(context.location, "canonical active-region role is absent"))?;
    match context
        .instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
    {
        Some(LinkedInstructionTarget::ActiveRegion(region)) => Ok(region),
        _ => Err(violation(
            context.location,
            "active-region role has a non-region typed target",
        )),
    }
}

fn require_supported(
    opcode: Opcode,
    location: VerificationLocation,
) -> Result<(), VerificationError> {
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
        | Opcode::CallLocal
        | Opcode::Return
        | Opcode::Not
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
        | Opcode::GreaterOrEqual
        | Opcode::StreamNext
        | Opcode::Trap
        | Opcode::CallService
        | Opcode::CallActor
        | Opcode::CallInterface
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
        | Opcode::EmitStream
        | Opcode::Throw
        | Opcode::Rethrow
        | Opcode::EnterRegion
        | Opcode::LeaveRegion
        | Opcode::InvokeHost
        | Opcode::InvokeIntrinsic => Ok(()),
        _ => Err(unavailable(location)),
    }
}

fn consume_inputs(
    before: &ProgramPointState,
    context: &Context<'_>,
) -> Result<(Vec<AbstractValue>, Vec<Vec<AbstractValue>>), VerificationError> {
    let counts = context
        .contract
        .typed
        .stack_in
        .iter()
        .map(|group| group_arity(group, context))
        .collect::<Result<Vec<_>, _>>()?;
    let total = counts.iter().try_fold(0_usize, |total, count| {
        total.checked_add(*count).ok_or_else(|| {
            violation(
                context.location,
                "operand-stack input arity overflowed usize",
            )
        })
    })?;
    if before.stack.len() < total {
        return Err(violation(
            context.location,
            format!(
                "operand-stack underflow: instruction needs {total} values but has {}",
                before.stack.len()
            ),
        ));
    }
    let prefix_len = before.stack.len() - total;
    if matches!(context.contract.control, ControlContract::Return) && prefix_len != 0 {
        return Err(violation(
            context.location,
            "return leaves residual values below its exact function results",
        ));
    }

    let consumed = &before.stack[prefix_len..];
    let mut offset = 0_usize;
    let mut inputs = Vec::with_capacity(counts.len());
    for (group, count) in context.contract.typed.stack_in.iter().zip(counts) {
        let end = offset.checked_add(count).ok_or_else(|| {
            violation(
                context.location,
                "operand-stack group offset overflowed usize",
            )
        })?;
        let values = consumed.get(offset..end).ok_or_else(|| {
            violation(
                context.location,
                "operand-stack input group is out of bounds",
            )
        })?;
        validate_input_source(group.value, values, before, &inputs, context)?;
        inputs.push(values.to_vec());
        offset = end;
    }
    Ok((before.stack[..prefix_len].to_vec(), inputs))
}

#[allow(clippy::too_many_arguments)]
fn validate_input_source(
    source: ValueSource,
    input: &[AbstractValue],
    before: &ProgramPointState,
    prior_inputs: &[Vec<AbstractValue>],
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    match source {
        ValueSource::AnyStackValue => input.iter().try_for_each(|value| {
            values::require_concrete_fact(*value, context.facts, context.location)
        }),
        ValueSource::Bool => values::require_implicit(
            input,
            context.facts,
            ImplicitBuiltin::Bool,
            context.location,
        ),
        ValueSource::Number => values::require_implicit(
            input,
            context.facts,
            ImplicitBuiltin::Number,
            context.location,
        ),
        ValueSource::CollectionIndex => {
            require_one(input, context)?;
            values::require_integer_or_number(input[0], context.facts, context.location)
        }
        ValueSource::ComparablePair => {
            if input.len() != 2 {
                return Err(violation(
                    context.location,
                    "comparison pair arity is not exactly two",
                ));
            }
            let [left, right] = input else {
                unreachable!("comparison pair length was checked to be two");
            };
            let AbstractValue::Concrete(left_ty) = *left;
            let AbstractValue::Concrete(right_ty) = *right;
            if context.facts.semantically_equal(left_ty, right_ty) != Some(true) {
                return Err(violation(
                    context.location,
                    "comparison inputs do not have one exact concrete type and plan",
                ));
            }
            Ok(())
        }
        ValueSource::TargetParameters { .. } => {
            let call = context
                .call
                .as_ref()
                .ok_or_else(|| violation(context.location, "local call has no exact call plan"))?;
            if input.len() != call.parameters.len() {
                return Err(violation(
                    context.location,
                    "local-call argument arity is not exact",
                ));
            }
            for (ordinal, (value, expected)) in input.iter().zip(call.parameters.iter()).enumerate()
            {
                values::require_same_type(
                    *value,
                    *expected,
                    context.facts,
                    context.location,
                    format!("local-call argument {ordinal}"),
                )?;
            }
            Ok(())
        }
        ValueSource::FunctionResults => {
            let expected = context.function.frame().result_types();
            if input.len() != expected.len() {
                return Err(violation(
                    context.location,
                    "return result arity is not exact",
                ));
            }
            for (ordinal, (value, expected)) in input.iter().zip(expected).enumerate() {
                values::require_same_type(
                    *value,
                    *expected,
                    context.facts,
                    context.location,
                    format!("return result {ordinal}"),
                )?;
            }
            Ok(())
        }
        ValueSource::InterfaceReceiver { interface }
        | ValueSource::InterfaceCarrier { interface } => {
            require_one(input, context)?;
            let expected = values::interface_carrier_type(context, interface, context.location)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "interface carrier",
            )
        }
        ValueSource::CallbackCaptures { layout } => {
            let layout_index = values::resolve_capture_layout(context, layout)?;
            let row = context
                .candidate
                .callback_capture_layouts()
                .get(layout_index.get() as usize)
                .filter(|row| row.index() == layout_index)
                .ok_or_else(|| {
                    violation(context.location, "callback capture layout is out of bounds")
                })?;
            if input.len() != row.captures().len() {
                return Err(violation(
                    context.location,
                    "callback capture arity is not exact",
                ));
            }
            for (ordinal, (value, capture)) in input.iter().zip(row.captures()).enumerate() {
                values::require_same_type(
                    *value,
                    capture.ty(),
                    context.facts,
                    context.location,
                    format!("callback capture {ordinal}"),
                )?;
            }
            Ok(())
        }
        ValueSource::ShapeFields { shape } => {
            let shape_index = values::resolve_shape(context, shape)?;
            let row = context
                .candidate
                .shapes()
                .get(shape_index.get() as usize)
                .filter(|row| row.index() == shape_index)
                .ok_or_else(|| violation(context.location, "shape target is out of bounds"))?;
            if input.len() != row.fields().len() {
                return Err(violation(
                    context.location,
                    "record field arity is not exact",
                ));
            }
            for (ordinal, (value, field)) in input.iter().zip(row.fields()).enumerate() {
                values::require_same_type(
                    *value,
                    field.ty(),
                    context.facts,
                    context.location,
                    format!("record field {ordinal}"),
                )?;
            }
            Ok(())
        }
        ValueSource::ShapeValue { shape } => {
            require_one(input, context)?;
            let expected = values::shape_value_type(context, shape, context.location)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "shape value",
            )
        }
        ValueSource::ShapeField { shape, ordinal } => {
            require_one(input, context)?;
            let expected = values::shape_field_type(context, shape, ordinal, context.location)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "shape field",
            )
        }
        ValueSource::WritablePathSelectors { path } => {
            let expected = values::writable_path_selectors(context, path, context.location)?;
            if input.len() != expected.len() {
                return Err(violation(
                    context.location,
                    "writable path selector arity is not exact",
                ));
            }
            for (ordinal, (value, expected)) in input.iter().zip(expected).enumerate() {
                values::require_same_type(
                    *value,
                    expected,
                    context.facts,
                    context.location,
                    format!("writable path selector {ordinal}"),
                )?;
            }
            Ok(())
        }
        ValueSource::WritablePathLeaf { path } => {
            require_one(input, context)?;
            let expected = values::writable_path_leaf_type(context, path, context.location)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "writable path leaf",
            )
        }
        ValueSource::RepresentationPayload { ty } => {
            require_one(input, context)?;
            let expected = values::resolve_type(context, ty)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "representation payload",
            )
        }
        ValueSource::ArrayBuilder { .. } | ValueSource::ArrayValue => {
            require_one(input, context)?;
            values::require_container(
                input[0],
                context,
                LinkedContainerLayoutKind::Array,
                context.location,
            )?;
            Ok(())
        }
        ValueSource::ArrayElement { array_input } => {
            let input_group = prior_inputs.get(array_input as usize).ok_or_else(|| {
                violation(
                    context.location,
                    "array element names an absent input group",
                )
            })?;
            let map = input_group
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "array element has no array operand"))?;
            require_one(input, context)?;
            let element = values::require_container(
                map,
                context,
                LinkedContainerLayoutKind::Array,
                context.location,
            )?;
            values::require_same_type(
                input[0],
                element,
                context.facts,
                context.location,
                "array element",
            )
        }
        ValueSource::ArrayElementFromSlot { slot } => {
            require_one(input, context)?;
            let slot_value = values::live_slot(
                before,
                values::resolve_slot(context, slot)?,
                context.location,
            )?;
            let element = values::require_container(
                slot_value,
                context,
                LinkedContainerLayoutKind::Array,
                context.location,
            )?;
            values::require_same_type(
                input[0],
                element,
                context.facts,
                context.location,
                "owned array element",
            )
        }
        ValueSource::MapBuilder { .. } | ValueSource::MapValue => {
            require_one(input, context)?;
            values::require_container(
                input[0],
                context,
                LinkedContainerLayoutKind::Map,
                context.location,
            )?;
            Ok(())
        }
        ValueSource::MapKey { map_input } => {
            let input_group = prior_inputs.get(map_input as usize).ok_or_else(|| {
                violation(context.location, "map key names an absent input group")
            })?;
            let map = input_group
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "map key has no map operand"))?;
            require_one(input, context)?;
            values::require_map_key(input[0], context, map, context.location)?;
            Ok(())
        }
        ValueSource::MapElement { map_input } => {
            let input_group = prior_inputs.get(map_input as usize).ok_or_else(|| {
                violation(context.location, "map element names an absent input group")
            })?;
            let map = input_group
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "map element has no map operand"))?;
            require_one(input, context)?;
            values::require_map_element(input[0], context, map, context.location)?;
            Ok(())
        }
        ValueSource::MapKeyFromSlot { slot } => {
            require_one(input, context)?;
            let slot_value = values::live_slot(
                before,
                values::resolve_slot(context, slot)?,
                context.location,
            )?;
            let key = values::map_key_type(context, slot_value, context.location)?;
            values::require_same_type(
                input[0],
                key,
                context.facts,
                context.location,
                "owned map key",
            )
        }
        ValueSource::MapElementFromSlot { slot } => {
            require_one(input, context)?;
            let slot_value = values::live_slot(
                before,
                values::resolve_slot(context, slot)?,
                context.location,
            )?;
            let value = values::map_value_type(context, slot_value, context.location)?;
            values::require_same_type(
                input[0],
                value,
                context.facts,
                context.location,
                "owned map value",
            )
        }
        ValueSource::ExceptionPayload { type_ref } => {
            require_one(input, context)?;
            let expected = values::exception_payload_type(context, type_ref, context.location)?;
            values::require_same_type(
                input[0],
                expected,
                context.facts,
                context.location,
                "throw payload",
            )
        }
        ValueSource::ExceptionEnvelope { source_slot } => {
            require_one(input, context)?;
            let slot_value = values::live_slot(
                before,
                values::resolve_slot(context, source_slot)?,
                context.location,
            )?;
            values::require_exception_envelope(slot_value, context, context.location)
        }
        ValueSource::FunctionStreamItem => {
            require_one(input, context)?;
            let stream = context
                .function
                .stream_result_type_ref()
                .ok_or_else(|| {
                    violation(
                        context.location,
                        "EmitStream requires the explicit producer stream authority",
                    )
                })?;
            let item = context
                .facts
                .stream_item_type(stream, context.location)?;
            values::require_same_type(
                input[0],
                item,
                context.facts,
                context.location,
                "stream producer item",
            )
        }
        ValueSource::InOutCallInputs { .. }
        | ValueSource::TaggedValue
        | ValueSource::Constant { .. }
        | ValueSource::Slot { .. }
        | ValueSource::StackInput { .. }
        | ValueSource::CallbackClosure { .. } => Err(unavailable(context.location)),
        _ => Err(unavailable(context.location)),
    }
}

fn require_one(input: &[AbstractValue], context: &Context<'_>) -> Result<(), VerificationError> {
    if input.len() != 1 {
        return Err(violation(
            context.location,
            "instruction input group must have exactly one value",
        ));
    }
    Ok(())
}

fn produce_outputs(
    stack: &mut Vec<AbstractValue>,
    before: &ProgramPointState,
    inputs: &[Vec<AbstractValue>],
    context: &Context<'_>,
) -> Result<(), VerificationError> {
    for group in context.contract.typed.stack_out {
        let count = group_arity(group, context)?;
        stack.extend(output_values(group.value, count, before, inputs, context)?);
    }
    Ok(())
}

fn output_values(
    source: ValueSource,
    count: usize,
    before: &ProgramPointState,
    inputs: &[Vec<AbstractValue>],
    context: &Context<'_>,
) -> Result<Vec<AbstractValue>, VerificationError> {
    match source {
        ValueSource::Bool => values::singleton_implicit(
            count,
            context.facts,
            ImplicitBuiltin::Bool,
            context.location,
        ),
        ValueSource::Number => values::singleton_implicit(
            count,
            context.facts,
            ImplicitBuiltin::Number,
            context.location,
        ),
        ValueSource::Constant { operand } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let constant = values::resolve_constant(context, operand)?;
            let ty = context
                .candidate
                .constants()
                .get(constant.get() as usize)
                .filter(|row| row.index() == constant)
                .map(|row| row.ty())
                .ok_or_else(|| violation(context.location, "constant target is out of bounds"))?;
            values::require_constant_materializable(ty, context.facts, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::Slot { operand } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let slot = values::resolve_slot(context, operand)?;
            Ok(vec![values::live_slot(before, slot, context.location)?])
        }
        ValueSource::StackInput { group } => {
            let input = inputs.get(group as usize).ok_or_else(|| {
                violation(context.location, "stack output names an absent input group")
            })?;
            if input.len() == count {
                return Ok(input.clone());
            }
            if input.len() == 1 && count > 1 {
                values::require_shareable(input[0], context.facts, context.location)?;
                return Ok(vec![input[0]; count]);
            }
            Err(unavailable(context.location))
        }
        ValueSource::TargetResults { .. } => {
            let call = context
                .call
                .as_ref()
                .ok_or_else(|| violation(context.location, "local call has no exact call plan"))?;
            if count != call.results.len() {
                return Err(violation(
                    context.location,
                    "local-call result arity is not exact",
                ));
            }
            Ok(call
                .results
                .iter()
                .copied()
                .map(AbstractValue::Concrete)
                .collect())
        }
        ValueSource::StreamItem { endpoint_slot } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let slot = values::resolve_slot(context, endpoint_slot)?;
            let AbstractValue::Concrete(endpoint) =
                values::live_slot(before, slot, context.location)?;
            let item = context
                .facts
                .stream_item_type(endpoint, context.location)
                .map_err(|_| {
                    violation(
                        context.location,
                        "stream item type cannot be derived from the endpoint slot",
                    )
                })?;
            Ok(vec![AbstractValue::Concrete(item)])
        }
        ValueSource::ShapeValue { shape } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let ty = values::shape_value_type(context, shape, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::ShapeField { shape, ordinal } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let ty = values::shape_field_type(context, shape, ordinal, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::RepresentationValue { ty } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let ty = values::resolve_type(context, ty)?;
            values::require_type_fact(ty, context.facts, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::ArrayBuilder { element_type } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let element = values::resolve_type(context, element_type)?;
            let ty = values::array_type_for_element(context, element, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::ArrayFromBuilder { builder_input }
        | ValueSource::MapFromBuilder { builder_input } => {
            let input = inputs.get(builder_input as usize).ok_or_else(|| {
                violation(
                    context.location,
                    "builder output names an absent input group",
                )
            })?;
            if input.len() != 1 {
                return Err(unavailable(context.location));
            }
            Ok(input.clone())
        }
        ValueSource::ArrayElement { array_input } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let input = inputs.get(array_input as usize).ok_or_else(|| {
                violation(
                    context.location,
                    "array element names an absent input group",
                )
            })?;
            let map = input
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "array element has no array operand"))?;
            let ty = values::require_container(
                map,
                context,
                LinkedContainerLayoutKind::Array,
                context.location,
            )?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::MapBuilder {
            key_type,
            value_type,
        } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let key = values::resolve_type(context, key_type)?;
            let value = values::resolve_type(context, value_type)?;
            let ty = values::map_type_for_key_value(context, key, value, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::MapKey { map_input } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let input = inputs.get(map_input as usize).ok_or_else(|| {
                violation(context.location, "map key names an absent input group")
            })?;
            let map = input
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "map key has no map operand"))?;
            let ty = values::map_key_type(context, map, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::MapElement { map_input } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let input = inputs.get(map_input as usize).ok_or_else(|| {
                violation(context.location, "map element names an absent input group")
            })?;
            let map = input
                .first()
                .copied()
                .ok_or_else(|| violation(context.location, "map element has no map operand"))?;
            let ty = values::map_value_type(context, map, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::InterfaceCarrier { interface } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let ty = values::interface_carrier_type(context, interface, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        ValueSource::CallbackClosure { target } => {
            if count != 1 {
                return Err(unavailable(context.location));
            }
            let ty = values::callback_closure_type(context, target, context.location)?;
            Ok(vec![AbstractValue::Concrete(ty)])
        }
        _ => Err(unavailable(context.location)),
    }
}

fn group_arity(group: &TypedStackGroup, context: &Context<'_>) -> Result<usize, VerificationError> {
    match group.arity {
        Arity::Fixed(count) => Ok(usize::from(count)),
        Arity::Declared(role) => context
            .contract
            .operand_word(role, context.instruction.operands())
            .and_then(|count| usize::try_from(count).ok())
            .ok_or_else(|| {
                violation(
                    context.location,
                    "declared stack arity is absent or too large",
                )
            }),
        Arity::FunctionResultCount => Ok(context.function.frame().result_types().len()),
    }
}

const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::StackAndSlotState,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::StackAndSlotState,
        location,
        detail: detail.into(),
    }
}

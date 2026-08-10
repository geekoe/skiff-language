mod slots;
mod tail;
mod values;

use skiff_artifact_model::{
    contract_for_opcode, Arity, ControlContract, Opcode, OpcodeContract, ParamModeIr,
    TypedStackGroup, ValueSource,
};
use skiff_runtime_linked_bytecode::{
    InstructionIndex, LinkedBytecodeCandidate, LinkedFunction, LinkedInstruction, TypeIndex,
};

use super::super::{tail::VerifiedTailCallProof, AbstractValue, ProgramPointState};
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
    require_supported(instruction.opcode(), location)?;

    let (mut stack, inputs) = consume_inputs(before, &context)?;
    let next_slots = slots::apply(before, &inputs, &context)?;
    produce_outputs(&mut stack, before, &inputs, &context)?;
    Ok(InstructionTransfer::Continue(ProgramPointState {
        stack: stack.into_boxed_slice(),
        slots: next_slots.into_boxed_slice(),
        active_regions: before.active_regions.clone(),
        writable_loans: before.writable_loans.clone(),
    }))
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
        | Opcode::StreamNext => Ok(()),
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
        validate_input_source(group.value, values, context)?;
        inputs.push(values.to_vec());
        offset = end;
    }
    Ok((before.stack[..prefix_len].to_vec(), inputs))
}

fn validate_input_source(
    source: ValueSource,
    input: &[AbstractValue],
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
        _ => Err(unavailable(context.location)),
    }
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

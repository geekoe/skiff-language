use skiff_artifact_model::{contract_for_opcode, OperandRole, TypeRefIr, ValueSource};
use skiff_runtime_linked_bytecode::{
    LinkedFrameLayout, LinkedInstruction, LinkedInstructionTarget, LinkedStackValue,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, BytecodeLinkObligation};

use super::{obligation_error, StackMapContext};

pub(super) fn source_values(
    context: &mut StackMapContext<'_, '_>,
    instruction: &LinkedInstruction,
    source: ValueSource,
    inputs: &[Vec<LinkedStackValue>],
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    match source {
        ValueSource::Bool => scalar_value(context, "bool", location),
        ValueSource::Number => scalar_value(context, "number", location),
        ValueSource::Slot { operand } => {
            let slot = operand_word(instruction, operand, location.clone())? as usize;
            let ty = context
                .frame
                .slot_types()
                .get(slot)
                .copied()
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!("frame slot {slot} is out of bounds"),
                    )
                })?;
            let plan = context
                .frame
                .slot_plans()
                .get(slot)
                .cloned()
                .ok_or_else(|| {
                    obligation_error(location, format!("frame slot plan {slot} is out of bounds"))
                })?;
            Ok(vec![LinkedStackValue::new(ty, plan)])
        }
        ValueSource::StackInput { group } => inputs.get(group as usize).cloned().ok_or_else(|| {
            obligation_error(
                location,
                format!("typed transition references missing stack input group {group}"),
            )
        }),
        ValueSource::TargetParameters { target } => {
            target_parameter_values(instruction, target, context.all_frames, location)
        }
        ValueSource::TargetResults { target } => {
            let target = target_frame(instruction, target, context.all_frames, location)?;
            Ok(target
                .result_types()
                .iter()
                .copied()
                .zip(target.result_plans().iter().cloned())
                .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
                .collect())
        }
        ValueSource::FunctionResults => Ok(context
            .frame
            .result_types()
            .iter()
            .copied()
            .zip(context.frame.result_plans().iter().cloned())
            .map(|(ty, plan)| LinkedStackValue::new(ty, plan))
            .collect()),
        ValueSource::AnyStackValue | ValueSource::TaggedValue | ValueSource::ComparablePair => {
            Err(obligation_error(
                location,
                format!(
                    "typed output source {} cannot establish a concrete value",
                    source.name()
                ),
            ))
        }
        _ => Err(BytecodeLinkError::ImplementationUnavailable {
            obligation: BytecodeLinkObligation::ControlFlowAndStackMap,
            location,
        }),
    }
}

pub(super) fn operand_word(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<u32, BytecodeLinkError> {
    contract_for_opcode(instruction.opcode())
        .operand_word(role, instruction.operands())
        .ok_or_else(|| {
            obligation_error(location, format!("operand role {} is absent", role.name()))
        })
}

pub(super) fn target_frame<'a>(
    instruction: &LinkedInstruction,
    role: OperandRole,
    frames: &'a [LinkedFrameLayout],
    location: BytecodeLinkLocation,
) -> Result<&'a LinkedFrameLayout, BytecodeLinkError> {
    let ordinal = contract_for_opcode(instruction.opcode())
        .operand_position(role)
        .ok_or_else(|| {
            obligation_error(
                location.clone(),
                format!("operand role {} is absent", role.name()),
            )
        })?;
    let ordinal = u32::try_from(ordinal).map_err(|_| {
        obligation_error(
            location.clone(),
            "operand ordinal does not fit u32".to_string(),
        )
    })?;
    let Some(LinkedInstructionTarget::Function(function)) = instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal)
        .map(|resolved| resolved.target())
    else {
        return Err(obligation_error(
            location,
            format!(
                "target operand role {} is not a local function",
                role.name()
            ),
        ));
    };
    frames.get(function.get() as usize).ok_or_else(|| {
        obligation_error(
            location,
            format!("local function target {} is out of bounds", function.get()),
        )
    })
}

fn target_parameter_values(
    instruction: &LinkedInstruction,
    role: OperandRole,
    frames: &[LinkedFrameLayout],
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let target = target_frame(instruction, role, frames, location.clone())?;
    target
        .parameters()
        .iter()
        .map(|parameter| {
            target
                .slot_types()
                .get(parameter.slot().get() as usize)
                .copied()
                .map(|ty| LinkedStackValue::new(ty, parameter.plan().clone()))
                .ok_or_else(|| {
                    obligation_error(
                        location.clone(),
                        format!(
                            "target parameter slot {} is out of bounds",
                            parameter.slot().get()
                        ),
                    )
                })
        })
        .collect()
}

fn scalar_value(
    context: &mut StackMapContext<'_, '_>,
    name: &str,
    location: BytecodeLinkLocation,
) -> Result<Vec<LinkedStackValue>, BytecodeLinkError> {
    let ty = context.type_linker.intern_builtin(
        context.source.package,
        context.source.specialization,
        name,
        context.substitutions,
        location.clone(),
    )?;
    let plan = context
        .type_linker
        .plan_for_concrete_type(&TypeRefIr::builtin(name), location)?;
    Ok(vec![LinkedStackValue::new(ty, plan)])
}

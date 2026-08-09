use skiff_artifact_model::{contract_for_opcode, ControlContract, OperandRole};
use skiff_runtime_linked_bytecode::{
    LinkedInstruction, LinkedInstructionTarget, LinkedSwitchTable,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation};

use super::{obligation_error, values::operand_word};

pub(super) fn successors(
    index: usize,
    instruction: &LinkedInstruction,
    control: ControlContract,
    switch_tables: &[LinkedSwitchTable],
    instruction_count: usize,
    location: BytecodeLinkLocation,
) -> Result<Vec<usize>, BytecodeLinkError> {
    match control {
        ControlContract::Fallthrough => Ok(vec![fallthrough(index, instruction_count, location)?]),
        ControlContract::Jump { target } => Ok(vec![branch_target(instruction, target, location)?]),
        ControlContract::Branch { target, .. } => {
            let mut targets = vec![
                fallthrough(index, instruction_count, location.clone())?,
                branch_target(instruction, target, location)?,
            ];
            targets.sort_unstable();
            targets.dedup();
            Ok(targets)
        }
        ControlContract::Switch { table } => {
            switch_successors(instruction, table, switch_tables, location)
        }
        ControlContract::Return
        | ControlContract::TailCall
        | ControlContract::Raise
        | ControlContract::Rethrow => Ok(Vec::new()),
    }
}

fn fallthrough(
    index: usize,
    instruction_count: usize,
    location: BytecodeLinkLocation,
) -> Result<usize, BytecodeLinkError> {
    let next = index.checked_add(1).ok_or_else(|| {
        obligation_error(
            location.clone(),
            "fallthrough instruction index overflowed".to_string(),
        )
    })?;
    (next < instruction_count).then_some(next).ok_or_else(|| {
        obligation_error(
            location,
            "fallthrough instruction is the final function instruction".to_string(),
        )
    })
}

fn switch_successors(
    instruction: &LinkedInstruction,
    role: OperandRole,
    switch_tables: &[LinkedSwitchTable],
    location: BytecodeLinkLocation,
) -> Result<Vec<usize>, BytecodeLinkError> {
    let raw = operand_word(instruction, role, location.clone())? as usize;
    let table = switch_tables.get(raw).ok_or_else(|| {
        obligation_error(location, format!("switch table {raw} is out of bounds"))
    })?;
    let mut targets = table
        .cases()
        .iter()
        .map(|case| case.target().get() as usize)
        .chain(std::iter::once(table.default_target().get() as usize))
        .collect::<Vec<_>>();
    targets.sort_unstable();
    targets.dedup();
    Ok(targets)
}

fn branch_target(
    instruction: &LinkedInstruction,
    role: OperandRole,
    location: BytecodeLinkLocation,
) -> Result<usize, BytecodeLinkError> {
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
    match instruction
        .resolved_operands()
        .iter()
        .find(|resolved| resolved.operand_ordinal() == ordinal)
        .map(|resolved| resolved.target())
    {
        Some(LinkedInstructionTarget::Branch(target)) => Ok(target.get() as usize),
        _ => Err(obligation_error(
            location,
            format!(
                "branch operand role {} has no instruction target",
                role.name()
            ),
        )),
    }
}

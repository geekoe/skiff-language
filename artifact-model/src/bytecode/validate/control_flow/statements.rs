use crate::bytecode::decode::DecodedInstruction;
use crate::bytecode::dto::RelocatableBytecodeFunction;
use crate::bytecode::opcodes::{contract_for_opcode, StatementContract};
use crate::validate_statement_entries_canonical;

use super::super::{table_error, StructuralValidationError};

pub(super) fn validate_statement_entries(
    key: &str,
    function: &RelocatableBytecodeFunction,
    instructions: &[DecodedInstruction],
) -> Result<(), StructuralValidationError> {
    validate_statement_entries_canonical(&function.statement_entries)
        .map_err(|error| table_error(key, error.to_string()))?;

    // Both inputs are canonical by PC, so header membership and required-event
    // checks share one O(I + E) merge cursor.
    let entries = &function.statement_entries;
    let mut entry_cursor = 0_usize;
    for instruction in instructions {
        if entries
            .get(entry_cursor)
            .is_some_and(|entry| entry.pc < instruction.pc)
        {
            return Err(non_header_error(
                key,
                entry_cursor,
                entries[entry_cursor].pc,
            ));
        }

        let pc_entry_start = entry_cursor;
        while entries
            .get(entry_cursor)
            .is_some_and(|entry| entry.pc == instruction.pc)
        {
            entry_cursor = entry_cursor.checked_add(1).ok_or_else(|| {
                table_error(key, "statement entry cursor overflowed usize".to_string())
            })?;
        }
        validate_opcode_requirement(key, instruction, &entries[pc_entry_start..entry_cursor])?;
    }

    if let Some(entry) = entries.get(entry_cursor) {
        return Err(non_header_error(key, entry_cursor, entry.pc));
    }
    Ok(())
}

fn validate_opcode_requirement(
    key: &str,
    instruction: &DecodedInstruction,
    entries: &[crate::StatementEntry],
) -> Result<(), StructuralValidationError> {
    let contract = contract_for_opcode(instruction.descriptor.kind);
    let StatementContract::RequiredEvent {
        charge_kind,
        attribution,
    } = contract.statement
    else {
        return Ok(());
    };
    let matching = entries
        .iter()
        .filter(|entry| entry.attribution_id.class() == attribution)
        .count();
    if matching != 1 {
        return Err(table_error(
            key,
            format!(
                "{} at pc {} derives {} and requires exactly one {} source event at that pc (found {matching})",
                contract.mnemonic,
                instruction.pc,
                charge_kind.name(),
                attribution.name(),
            ),
        ));
    }
    Ok(())
}

fn non_header_error(key: &str, index: usize, pc: u32) -> StructuralValidationError {
    table_error(
        key,
        format!("statementEntries[{index}] pc {pc} is not an instruction header"),
    )
}

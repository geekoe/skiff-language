use crate::bytecode::dto::RelocatableBytecodeFunction;

use super::super::{table_error, StructuralValidationError};

pub(super) fn validate_statement_entries(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
) -> Result<(), StructuralValidationError> {
    let mut previous_pc: Option<u32> = None;
    let mut saw_function_entry = false;
    for (index, entry) in function.statement_entries.iter().enumerate() {
        if let Some(previous_pc) = previous_pc {
            if previous_pc >= entry.pc {
                return Err(table_error(
                    key,
                    format!(
                        "statementEntries[{index}] pc {} is not strictly ascending (previous {previous_pc})",
                        entry.pc
                    ),
                ));
            }
        }
        if header_pcs.binary_search(&entry.pc).is_err() {
            return Err(table_error(
                key,
                format!(
                    "statementEntries[{index}] pc {} is not an instruction header",
                    entry.pc
                ),
            ));
        }
        if entry.statement_id.is_empty() {
            return Err(table_error(
                key,
                format!("statementEntries[{index}].statementId must not be empty"),
            ));
        }
        if entry.charge_kind == crate::bytecode::dto::StatementChargeKind::FunctionEntry {
            if saw_function_entry || entry.pc != 0 {
                return Err(table_error(
                    key,
                    format!("statementEntries[{index}] has invalid duplicate/non-zero FunctionEntry charge"),
                ));
            }
            saw_function_entry = true;
        }
        previous_pc = Some(entry.pc);
    }
    if !function.words.is_empty() && !saw_function_entry {
        return Err(table_error(
            key,
            "non-empty function must declare one FunctionEntry charge".to_string(),
        ));
    }
    Ok(())
}

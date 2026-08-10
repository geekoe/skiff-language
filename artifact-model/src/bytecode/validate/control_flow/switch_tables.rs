use crate::bytecode::dto::{BytecodePools, RelocatableBytecodeFunction};

use super::super::{table_error, validate_type_pool_ref, StructuralValidationError};

pub(super) fn validate_switch_tables(
    key: &str,
    function: &RelocatableBytecodeFunction,
    header_pcs: &[u32],
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    for (index, table) in function.switch_tables.iter().enumerate() {
        if header_pcs.binary_search(&table.default_pc).is_err() {
            return Err(table_error(
                key,
                format!(
                    "switchTables[{index}].defaultPc {} is not an instruction header",
                    table.default_pc
                ),
            ));
        }
        let mut previous_tag = None;
        for (case_index, case) in table.cases.iter().enumerate() {
            validate_type_pool_ref(
                pools,
                case.tag_type_ref,
                &format!("functions[{key}].switchTables[{index}].cases[{case_index}].tagTypeRef"),
            )?;
            if previous_tag.is_some_and(|tag| tag >= case.tag_type_ref) {
                return Err(table_error(
                    key,
                    format!(
                        "switchTables[{index}].cases tagTypeRef values are not strictly ascending"
                    ),
                ));
            }
            if header_pcs.binary_search(&case.target_pc).is_err() {
                return Err(table_error(
                    key,
                    format!(
                        "switchTables[{index}].cases[{case_index}].targetPc {} is not an instruction header",
                        case.target_pc
                    ),
                ));
            }
            previous_tag = Some(case.tag_type_ref);
        }
    }
    Ok(())
}

use std::collections::BTreeSet;

use crate::bytecode::dto::{limits, BytecodePoolEntry, BytecodePools, RelocatableBytecodeFunction};

use super::{header_error, limit_error, table_error, StructuralValidationError};

pub(super) fn validate_writable_locals_and_loans(
    key: &str,
    function: &RelocatableBytecodeFunction,
    pools: &BytecodePools,
) -> Result<(), StructuralValidationError> {
    let frame = &function.frame_layout;
    if frame.writable_local_slots.len() as u64 > limits::MAX_SLOTS_PER_FRAME {
        return Err(limit_error(
            "MAX_SLOTS_PER_FRAME",
            limits::MAX_SLOTS_PER_FRAME,
            frame.writable_local_slots.len() as u64,
            &format!("functions[{key}].frameLayout.writableLocalSlots"),
        ));
    }

    let parameter_slots = frame
        .parameter_slots
        .iter()
        .map(|parameter| parameter.slot)
        .collect::<BTreeSet<_>>();
    let mut writable_slots = BTreeSet::new();
    let mut previous_slot = None;
    for (index, slot) in frame.writable_local_slots.iter().copied().enumerate() {
        if slot >= frame.slot_count {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.writableLocalSlots[{index}] slot {slot} out of bounds: slotCount {}",
                    frame.slot_count
                ),
            ));
        }
        if previous_slot.is_some_and(|previous| previous >= slot) {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.writableLocalSlots must be strictly increasing and unique at index {index}"
                ),
            ));
        }
        if parameter_slots.contains(&slot) {
            return Err(table_error(
                key,
                format!(
                    "frameLayout.writableLocalSlots[{index}] names incoming parameter slot {slot}"
                ),
            ));
        }
        previous_slot = Some(slot);
        writable_slots.insert(slot);
    }

    let mut total_loans = 0_u64;
    for (layout_index, layout) in function.call_loan_layouts.iter().enumerate() {
        if layout.loans.is_empty() {
            return Err(table_error(
                key,
                format!("callLoanLayouts[{layout_index}].loans must not be empty"),
            ));
        }
        if layout.loans.len() as u64 > limits::MAX_ARITY {
            return Err(limit_error(
                "MAX_ARITY",
                limits::MAX_ARITY,
                layout.loans.len() as u64,
                &format!("functions[{key}].callLoanLayouts[{layout_index}].loans"),
            ));
        }
        total_loans = total_loans
            .checked_add(layout.loans.len() as u64)
            .ok_or_else(|| StructuralValidationError::Arithmetic {
                context: format!("functions[{key}].callLoanLayouts total loans"),
            })?;
        if total_loans > limits::MAX_TABLE_ENTRIES {
            return Err(limit_error(
                "MAX_TABLE_ENTRIES",
                limits::MAX_TABLE_ENTRIES,
                total_loans,
                &format!("functions[{key}].callLoanLayouts total loans"),
            ));
        }

        let mut previous_parameter = None;
        for (loan_index, loan) in layout.loans.iter().enumerate() {
            let location =
                format!("functions[{key}].callLoanLayouts[{layout_index}].loans[{loan_index}]");
            if u64::from(loan.parameter_ordinal) >= limits::MAX_ARITY {
                return Err(limit_error(
                    "MAX_ARITY",
                    limits::MAX_ARITY,
                    u64::from(loan.parameter_ordinal) + 1,
                    &format!("{location}.parameterOrdinal"),
                ));
            }
            if previous_parameter.is_some_and(|previous| previous >= loan.parameter_ordinal) {
                return Err(table_error(
                    key,
                    format!(
                        "callLoanLayouts[{layout_index}].loans must be strictly ordered and unique by parameterOrdinal"
                    ),
                ));
            }
            previous_parameter = Some(loan.parameter_ordinal);

            if !writable_slots.contains(&loan.root_slot) {
                return Err(table_error(
                    key,
                    format!(
                        "callLoanLayouts[{layout_index}].loans[{loan_index}].rootSlot {} is not a declared writable local",
                        loan.root_slot
                    ),
                ));
            }
            let Some(BytecodePoolEntry::WritablePath(path)) =
                pools.writable_paths.get(loan.writable_path_ref as usize)
            else {
                return Err(header_error(format!(
                    "{location}.writablePathRef {} does not select a WritablePath",
                    loan.writable_path_ref
                )));
            };
            let Some(root_type_ref) = frame.slot_type_refs.get(loan.root_slot as usize) else {
                return Err(header_error(format!(
                    "{location}.rootSlot {} has no slotTypeRef",
                    loan.root_slot
                )));
            };
            if *root_type_ref != path.root_type_ref {
                return Err(table_error(
                    key,
                    format!(
                        "callLoanLayouts[{layout_index}].loans[{loan_index}] root slot type {root_type_ref} does not match writable path rootTypeRef {}",
                        path.root_type_ref
                    ),
                ));
            }
        }
    }
    Ok(())
}

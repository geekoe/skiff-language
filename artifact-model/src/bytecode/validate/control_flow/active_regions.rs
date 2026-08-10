use crate::bytecode::decode::DecodedFunction;
use crate::bytecode::dto::RelocatableBytecodeFunction;
use crate::bytecode::opcodes::{Opcode, OperandRole};

use super::super::{descriptor_mismatch, table_error, StructuralValidationError};

pub(super) fn validate_active_regions(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let mut enter_counts = vec![0u32; function.active_regions.len()];
    let mut leave_counts = vec![0u32; function.active_regions.len()];
    for instruction in &decoded.instructions {
        let Some(region_index) = instruction
            .descriptor
            .operand_word(OperandRole::ActiveRegion, &instruction.operand_words)
        else {
            continue;
        };
        let counts = match instruction.descriptor.kind {
            Opcode::EnterRegion => &mut enter_counts,
            Opcode::LeaveRegion => &mut leave_counts,
            _ => {
                return Err(descriptor_mismatch(
                    key,
                    instruction.pc,
                    "ActiveRegion role".to_string(),
                ));
            }
        };
        counts[region_index as usize] = counts[region_index as usize].saturating_add(1);
    }
    let mut previous: Option<(u32, u32)> = None;
    let mut open_regions = Vec::<(usize, u32)>::new();
    for (index, region) in function.active_regions.iter().enumerate() {
        if region.start_pc >= region.end_pc {
            return Err(table_error(
                key,
                format!(
                    "activeRegions[{index}] startPc {} >= endPc {}",
                    region.start_pc, region.end_pc
                ),
            ));
        }
        if decoded.header_pcs.binary_search(&region.start_pc).is_err()
            || (region.end_pc != function.words.len() as u32
                && decoded.header_pcs.binary_search(&region.end_pc).is_err())
        {
            return Err(table_error(
                key,
                format!("activeRegions[{index}] boundaries are not instruction boundaries"),
            ));
        }
        if let Some((start, end)) = previous {
            if region.start_pc < start || (region.start_pc == start && region.end_pc >= end) {
                return Err(table_error(
                    key,
                    format!("activeRegions[{index}] is not in canonical outer-first order"),
                ));
            }
        }
        while open_regions
            .last()
            .is_some_and(|(_, end)| *end <= region.start_pc)
        {
            open_regions.pop();
        }
        if let Some((parent_index, parent_end)) = open_regions.last() {
            if region.end_pc > *parent_end {
                return Err(table_error(
                    key,
                    format!(
                        "activeRegions[{index}] crosses activeRegions[{parent_index}] ending at {parent_end}"
                    ),
                ));
            }
        }
        match &region.kind {
            crate::bytecode::dto::ActiveRegionKind::Timeout { duration_ms, .. } => {
                if *duration_ms == 0 {
                    return Err(table_error(
                        key,
                        format!("activeRegions[{index}] timeout durationMs must be positive"),
                    ));
                }
            }
        }
        if enter_counts[index] != 1 || leave_counts[index] != 1 {
            return Err(table_error(
                key,
                format!(
                    "activeRegions[{index}] must have exactly one enter and leave (got {}, {})",
                    enter_counts[index], leave_counts[index]
                ),
            ));
        }
        previous = Some((region.start_pc, region.end_pc));
        open_regions.push((index, region.end_pc));
    }
    Ok(())
}

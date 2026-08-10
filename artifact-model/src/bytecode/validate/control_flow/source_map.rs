use crate::bytecode::decode::DecodedFunction;
use crate::bytecode::dto::{RelocatableBytecodeFunction, SourceMapEntry};
use crate::bytecode::opcodes::{contract_for_opcode, SourceContract, SourceOriginConstraint};

use super::super::{table_error, StructuralValidationError};

/// Monotonic lookup over the already-validated sorted, non-overlapping source
/// ranges. For nondecreasing instruction PCs, `index` advances at most
/// `entries.len()` times across the complete function.
struct SourceMapCoverageCursor<'a> {
    entries: &'a [SourceMapEntry],
    index: usize,
}

impl<'a> SourceMapCoverageCursor<'a> {
    fn new(entries: &'a [SourceMapEntry]) -> Self {
        Self { entries, index: 0 }
    }

    fn covering_entry(&mut self, pc: u32) -> Option<&'a SourceMapEntry> {
        while self
            .entries
            .get(self.index)
            .is_some_and(|entry| entry.end_pc <= pc)
        {
            self.index += 1;
        }
        self.entries
            .get(self.index)
            .filter(|entry| entry.start_pc <= pc && pc < entry.end_pc)
    }
}

pub(super) fn validate_source_map(
    key: &str,
    function: &RelocatableBytecodeFunction,
    decoded: &DecodedFunction,
) -> Result<(), StructuralValidationError> {
    let word_count = function.words.len() as u32;
    let mut previous_end: Option<u32> = None;
    for (index, entry) in function.source_map.iter().enumerate() {
        if entry.start_pc >= entry.end_pc {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] start {} >= end {}",
                    entry.start_pc, entry.end_pc
                ),
            ));
        }
        if entry.end_pc > word_count {
            return Err(table_error(
                key,
                format!(
                    "sourceMap[{index}] end {} outside function word range {word_count}",
                    entry.end_pc
                ),
            ));
        }
        if decoded.header_pcs.binary_search(&entry.start_pc).is_err()
            || (entry.end_pc != word_count
                && decoded.header_pcs.binary_search(&entry.end_pc).is_err())
        {
            return Err(table_error(
                key,
                format!("sourceMap[{index}] range is not instruction-boundary aligned"),
            ));
        }
        if let Some(previous_end) = previous_end {
            if previous_end > entry.start_pc {
                return Err(table_error(
                    key,
                    format!(
                        "sourceMap[{index}] start {} overlaps previous entry ending at {previous_end}",
                        entry.start_pc
                    ),
                ));
            }
        }
        previous_end = Some(entry.end_pc);
    }
    let mut coverage_cursor = SourceMapCoverageCursor::new(&function.source_map);
    for instruction in &decoded.instructions {
        let SourceContract::Required { origin, .. } =
            contract_for_opcode(instruction.descriptor.kind).source
        else {
            continue;
        };
        let entry = coverage_cursor.covering_entry(instruction.pc);
        let coverage = usize::from(entry.is_some());
        let Some(entry) = entry else {
            return Err(table_error(
                key,
                format!(
                    "{} at pc {} requires exactly one source/synthetic site (found {coverage})",
                    instruction.descriptor.mnemonic, instruction.pc
                ),
            ));
        };
        if origin == SourceOriginConstraint::SyntheticOnly
            && !matches!(&entry.site, crate::InstructionSourceSite::Synthetic { .. })
        {
            return Err(table_error(
                key,
                format!(
                    "{} at pc {} requires a synthetic instruction source site",
                    instruction.descriptor.mnemonic, instruction.pc
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::SourceMapCoverageCursor;
    use crate::bytecode::dto::SourceMapEntry;

    #[test]
    fn source_map_cursor_is_linear_over_dense_alternating_ranges() {
        const RANGE_COUNT: u32 = 4_096;
        let source_map = (0..RANGE_COUNT)
            .map(|index| SourceMapEntry {
                start_pc: index * 2,
                end_pc: index * 2 + 1,
                site: crate::InstructionSourceSite::Synthetic {
                    reason: crate::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
            })
            .collect::<Vec<_>>();
        let mut cursor = SourceMapCoverageCursor::new(&source_map);
        let mut previous_index = 0;

        for pc in 0..RANGE_COUNT * 2 {
            assert_eq!(cursor.covering_entry(pc).is_some(), pc % 2 == 0);
            assert!(cursor.index >= previous_index);
            assert!(cursor.index - previous_index <= 1);
            previous_index = cursor.index;
        }

        assert_eq!(cursor.index, source_map.len());
    }
}

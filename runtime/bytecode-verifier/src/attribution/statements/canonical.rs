use std::collections::{hash_map::Entry, HashMap, HashSet};

use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId};
use skiff_runtime_linked_bytecode::{FunctionIndex, InstructionIndex, LinkedFunction};

use crate::{
    admission::ExactStatementEntry, VerificationError, VerificationLimit, VerificationLimits,
};

use super::{
    check_limit, function_location_at, instruction_location_at, resource_overflow, violation,
};

trait StatementRow {
    fn instruction(&self) -> InstructionIndex;
    fn sequence_ordinal(&self) -> u32;
    fn attribution_id(&self) -> StatementAttributionId;
    fn site(&self) -> &InstructionSourceSite;
}

impl StatementRow for ExactStatementEntry {
    fn instruction(&self) -> InstructionIndex {
        ExactStatementEntry::instruction(self)
    }

    fn sequence_ordinal(&self) -> u32 {
        ExactStatementEntry::sequence_ordinal(self)
    }

    fn attribution_id(&self) -> StatementAttributionId {
        ExactStatementEntry::attribution_id(self)
    }

    fn site(&self) -> &InstructionSourceSite {
        ExactStatementEntry::site(self)
    }
}

#[cfg(test)]
impl StatementRow for skiff_artifact_model::StatementEntry {
    fn instruction(&self) -> InstructionIndex {
        InstructionIndex::new(self.pc)
    }

    fn sequence_ordinal(&self) -> u32 {
        self.sequence_ordinal
    }

    fn attribution_id(&self) -> StatementAttributionId {
        self.attribution_id
    }

    fn site(&self) -> &InstructionSourceSite {
        &self.site
    }
}

#[derive(Debug, Clone, Copy)]
struct OccurrenceStats {
    count: u64,
    maximum: u32,
}

pub(super) fn prove_canonical(
    entries: &[ExactStatementEntry],
    function: &LinkedFunction,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    prove_rows(
        entries,
        function.index(),
        function.instructions().len(),
        limits,
    )
}

#[cfg(test)]
pub(super) fn prove_canonical_rows(
    entries: &[skiff_artifact_model::StatementEntry],
    function: FunctionIndex,
    instruction_count: usize,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    prove_rows(entries, function, instruction_count, limits)
}

fn prove_rows<R: StatementRow>(
    entries: &[R],
    function: FunctionIndex,
    instruction_count: usize,
    limits: &VerificationLimits,
) -> Result<(), VerificationError> {
    let function_location = function_location_at(function);
    let mut identities = HashSet::with_capacity(entries.len());
    let mut statement_occurrences = HashMap::<u32, OccurrenceStats>::with_capacity(entries.len());
    let mut statement_order = Vec::with_capacity(entries.len());
    let mut expression_occurrences = HashMap::<u32, OccurrenceStats>::with_capacity(entries.len());
    let mut expression_order = Vec::with_capacity(entries.len());
    let mut generated_occurrences = OccurrenceStats {
        count: 0,
        maximum: 0,
    };
    let mut previous_instruction = None;
    let mut expected_sequence = 0_u32;
    let mut events_at_pc = 0_u64;

    for (row, entry) in entries.iter().enumerate() {
        let instruction = entry.instruction();
        let location = instruction_location_at(function, instruction);
        let instruction_ordinal =
            usize::try_from(instruction.get()).map_err(|_| resource_overflow(location))?;
        if instruction_ordinal >= instruction_count {
            return Err(violation(
                location,
                format!("authenticated statement event {row} is outside the instruction slice"),
            ));
        }
        match previous_instruction {
            None => {
                require_sequence_start(entry.sequence_ordinal(), row, instruction, location)?;
                events_at_pc = 1;
            }
            Some(previous) if instruction.get() < previous => {
                return Err(violation(
                    location,
                    format!(
                        "statement event {row} instruction {} precedes previous instruction {previous}",
                        instruction.get(),
                    ),
                ));
            }
            Some(previous) if instruction.get() == previous => {
                if entry.sequence_ordinal() != expected_sequence {
                    return Err(violation(
                        location,
                        format!(
                            "statement event {row} at instruction {} has sequenceOrdinal {}, expected {expected_sequence}",
                            instruction.get(),
                            entry.sequence_ordinal(),
                        ),
                    ));
                }
                events_at_pc = events_at_pc
                    .checked_add(1)
                    .ok_or_else(|| resource_overflow(location))?;
            }
            Some(_) => {
                require_sequence_start(entry.sequence_ordinal(), row, instruction, location)?;
                events_at_pc = 1;
            }
        }
        check_limit(
            VerificationLimit::StatementEventsPerPc,
            events_at_pc,
            limits.max_statement_events_per_pc,
            location,
        )?;

        let attribution_id = entry.attribution_id();
        if !identities.insert(attribution_id) {
            return Err(violation(
                location,
                format!("statement event {row} repeats attribution id {attribution_id:?}"),
            ));
        }
        match attribution_id {
            StatementAttributionId::Statement {
                statement_index,
                occurrence_ordinal,
            } => record_occurrence(
                &mut statement_occurrences,
                &mut statement_order,
                statement_index,
                occurrence_ordinal,
                location,
            )?,
            StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            } => record_occurrence(
                &mut expression_occurrences,
                &mut expression_order,
                expression_index,
                occurrence_ordinal,
                location,
            )?,
            StatementAttributionId::Generated { ordinal } => {
                generated_occurrences.count = generated_occurrences
                    .count
                    .checked_add(1)
                    .ok_or_else(|| resource_overflow(location))?;
                generated_occurrences.maximum = generated_occurrences.maximum.max(ordinal);
                if !matches!(entry.site(), InstructionSourceSite::Synthetic { .. }) {
                    return Err(violation(
                        location,
                        format!(
                            "statement event {row} has a Generated attribution id with a source site"
                        ),
                    ));
                }
            }
        }

        expected_sequence = entry
            .sequence_ordinal()
            .checked_add(1)
            .ok_or_else(|| violation(location, "statement sequenceOrdinal overflows u32"))?;
        previous_instruction = Some(instruction.get());
    }

    validate_dense_occurrences(
        "statement",
        &statement_occurrences,
        &statement_order,
        function_location,
    )?;
    validate_dense_occurrences(
        "expression",
        &expression_occurrences,
        &expression_order,
        function_location,
    )?;
    validate_dense_generated(generated_occurrences, function_location)
}

fn require_sequence_start(
    sequence: u32,
    row: usize,
    instruction: InstructionIndex,
    location: crate::VerificationLocation,
) -> Result<(), VerificationError> {
    if sequence != 0 {
        return Err(violation(
            location,
            format!(
                "statement event {row} starts instruction {} at sequenceOrdinal {sequence}, expected 0",
                instruction.get(),
            ),
        ));
    }
    Ok(())
}

fn record_occurrence(
    occurrences: &mut HashMap<u32, OccurrenceStats>,
    order: &mut Vec<u32>,
    source_index: u32,
    occurrence_ordinal: u32,
    location: crate::VerificationLocation,
) -> Result<(), VerificationError> {
    let stats = match occurrences.entry(source_index) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            order.push(source_index);
            entry.insert(OccurrenceStats {
                count: 0,
                maximum: 0,
            })
        }
    };
    stats.count = stats
        .count
        .checked_add(1)
        .ok_or_else(|| resource_overflow(location))?;
    stats.maximum = stats.maximum.max(occurrence_ordinal);
    Ok(())
}

fn validate_dense_occurrences(
    label: &str,
    occurrences: &HashMap<u32, OccurrenceStats>,
    order: &[u32],
    location: crate::VerificationLocation,
) -> Result<(), VerificationError> {
    for source_index in order {
        let stats = occurrences.get(source_index).ok_or_else(|| {
            violation(
                location,
                format!("{label} occurrence order has no matching counter"),
            )
        })?;
        let expected = u64::from(stats.maximum)
            .checked_add(1)
            .ok_or_else(|| resource_overflow(location))?;
        if expected != stats.count {
            return Err(violation(
                location,
                format!(
                    "{label} attribution occurrences for source index {source_index} must be dense 0..{}, found {} rows",
                    stats.maximum, stats.count,
                ),
            ));
        }
    }
    Ok(())
}

fn validate_dense_generated(
    stats: OccurrenceStats,
    location: crate::VerificationLocation,
) -> Result<(), VerificationError> {
    let expected = u64::from(stats.maximum)
        .checked_add(1)
        .ok_or_else(|| resource_overflow(location))?;
    if stats.count > 0 && expected != stats.count {
        return Err(violation(
            location,
            format!(
                "generated attribution ordinals must be dense 0..{}, found {} rows",
                stats.maximum, stats.count,
            ),
        ));
    }
    Ok(())
}

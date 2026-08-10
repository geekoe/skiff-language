use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::InstructionSourceSite;

/// Function-local identity of one source-authored or generated attribution
/// event. The owning [`crate::BytecodeFunctionOrigin`] supplies the full
/// package executable coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StatementAttributionId {
    Statement {
        statement_index: u32,
        occurrence_ordinal: u32,
    },
    Expression {
        expression_index: u32,
        occurrence_ordinal: u32,
    },
    Generated {
        ordinal: u32,
    },
}

impl StatementAttributionId {
    pub const fn class(self) -> StatementAttributionClass {
        match self {
            Self::Statement { .. } => StatementAttributionClass::Statement,
            Self::Expression { .. } => StatementAttributionClass::Expression,
            Self::Generated { .. } => StatementAttributionClass::Generated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatementAttributionClass {
    Statement,
    Expression,
    Generated,
}

impl StatementAttributionClass {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Statement => "statement",
            Self::Expression => "expression",
            Self::Generated => "generated",
        }
    }
}

/// Exact placement of one source event in a relocatable function.
///
/// Multiple events may share a PC. `sequence_ordinal` is dense from zero at
/// each PC and makes their execution order explicit without persisting frame-
/// or opcode-derived charges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StatementEntry {
    pub pc: u32,
    pub sequence_ordinal: u32,
    pub attribution_id: StatementAttributionId,
    pub site: InstructionSourceSite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementEntryValidationError {
    message: String,
}

impl StatementEntryValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for StatementEntryValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StatementEntryValidationError {}

/// Validates the canonical facts that are independent of an instruction
/// stream. Header membership and opcode-derived requirements are validated by
/// the bytecode structural validator. Deterministic per-source occurrence
/// diagnostics use ordered maps, so this pass is O(E log E) and O(E) space.
pub fn validate_statement_entries_canonical(
    entries: &[StatementEntry],
) -> Result<(), StatementEntryValidationError> {
    let mut identities = HashSet::with_capacity(entries.len());
    let mut statement_occurrences = BTreeMap::<u32, (u64, u32)>::new();
    let mut expression_occurrences = BTreeMap::<u32, (u64, u32)>::new();
    let mut generated_occurrences = (0_u64, 0_u32);
    let mut previous_pc = None;
    let mut expected_sequence = 0_u32;

    for (index, entry) in entries.iter().enumerate() {
        match previous_pc {
            None => {
                if entry.sequence_ordinal != 0 {
                    return invalid(format!(
                        "statement entry {index} starts pc {} at sequenceOrdinal {}, expected 0",
                        entry.pc, entry.sequence_ordinal
                    ));
                }
            }
            Some(previous) if entry.pc < previous => {
                return invalid(format!(
                    "statement entry {index} pc {} precedes previous pc {previous}",
                    entry.pc
                ));
            }
            Some(previous) if entry.pc == previous => {
                if entry.sequence_ordinal != expected_sequence {
                    return invalid(format!(
                        "statement entry {index} at pc {} has sequenceOrdinal {}, expected {expected_sequence}",
                        entry.pc, entry.sequence_ordinal
                    ));
                }
            }
            Some(_) => {
                if entry.sequence_ordinal != 0 {
                    return invalid(format!(
                        "statement entry {index} starts pc {} at sequenceOrdinal {}, expected 0",
                        entry.pc, entry.sequence_ordinal
                    ));
                }
            }
        }

        if !identities.insert(entry.attribution_id) {
            return invalid(format!(
                "statement entry {index} repeats attribution id {:?}",
                entry.attribution_id
            ));
        }
        match entry.attribution_id {
            StatementAttributionId::Statement {
                statement_index,
                occurrence_ordinal,
            } => record_occurrence(
                &mut statement_occurrences,
                statement_index,
                occurrence_ordinal,
            ),
            StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            } => record_occurrence(
                &mut expression_occurrences,
                expression_index,
                occurrence_ordinal,
            ),
            StatementAttributionId::Generated { ordinal } => {
                generated_occurrences.0 += 1;
                generated_occurrences.1 = generated_occurrences.1.max(ordinal);
            }
        }
        if matches!(
            entry.attribution_id,
            StatementAttributionId::Generated { .. }
        ) && !matches!(entry.site, InstructionSourceSite::Synthetic { .. })
        {
            return invalid(format!(
                "statement entry {index} has a Generated attribution id with a source site"
            ));
        }

        expected_sequence = entry.sequence_ordinal.checked_add(1).ok_or_else(|| {
            StatementEntryValidationError::new(format!(
                "statement entry {index} sequenceOrdinal overflows u32"
            ))
        })?;
        previous_pc = Some(entry.pc);
    }

    validate_dense_occurrences("statement", &statement_occurrences)?;
    validate_dense_occurrences("expression", &expression_occurrences)?;
    if generated_occurrences.0 > 0
        && u64::from(generated_occurrences.1) + 1 != generated_occurrences.0
    {
        return invalid(format!(
            "generated attribution ordinals must be dense 0..{}, found {} rows",
            generated_occurrences.1, generated_occurrences.0
        ));
    }

    Ok(())
}

fn record_occurrence(
    occurrences: &mut BTreeMap<u32, (u64, u32)>,
    source_index: u32,
    occurrence_ordinal: u32,
) {
    let row = occurrences.entry(source_index).or_insert((0, 0));
    row.0 += 1;
    row.1 = row.1.max(occurrence_ordinal);
}

fn validate_dense_occurrences(
    label: &str,
    occurrences: &BTreeMap<u32, (u64, u32)>,
) -> Result<(), StatementEntryValidationError> {
    for (source_index, (count, maximum)) in occurrences {
        if u64::from(*maximum) + 1 != *count {
            return invalid(format!(
                "{label} attribution occurrences for source index {source_index} must be dense 0..{maximum}, found {count} rows"
            ));
        }
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, StatementEntryValidationError> {
    Err(StatementEntryValidationError::new(message))
}

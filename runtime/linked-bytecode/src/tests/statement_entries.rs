use skiff_artifact_model::{
    InstructionSourceSite, SourcePosition, SourceSpanRef, StatementAttributionId,
    StatementEntryValidationError, SyntheticInstructionSiteReason,
};

use crate::{
    CandidateLocation, CandidateReferenceKind, FunctionIndex, InstructionIndex,
    LinkedBytecodeCandidate, LinkedBytecodeCandidateError, LinkedStatementEntry,
};

use super::fixtures::{function_with_statement_entries, minimal_parts};

fn source_site(source_id: u64) -> InstructionSourceSite {
    InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id,
            start: SourcePosition::new(1, 1),
            end: SourcePosition::new(1, 2),
        },
    }
}

fn synthetic_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

fn source_statement(
    instruction: u32,
    sequence_ordinal: u32,
    statement_index: u32,
    occurrence_ordinal: u32,
) -> LinkedStatementEntry {
    LinkedStatementEntry::new(
        InstructionIndex::new(instruction),
        sequence_ordinal,
        StatementAttributionId::Statement {
            statement_index,
            occurrence_ordinal,
        },
        source_site(u64::from(statement_index) + 1),
    )
}

fn source_expression(
    instruction: u32,
    sequence_ordinal: u32,
    expression_index: u32,
    occurrence_ordinal: u32,
) -> LinkedStatementEntry {
    LinkedStatementEntry::new(
        InstructionIndex::new(instruction),
        sequence_ordinal,
        StatementAttributionId::Expression {
            expression_index,
            occurrence_ordinal,
        },
        source_site(u64::from(expression_index) + 1),
    )
}

fn generated_statement(
    instruction: u32,
    sequence_ordinal: u32,
    ordinal: u32,
) -> LinkedStatementEntry {
    LinkedStatementEntry::new(
        InstructionIndex::new(instruction),
        sequence_ordinal,
        StatementAttributionId::Generated { ordinal },
        synthetic_site(),
    )
}

fn statement_candidate(
    instruction_count: usize,
    statement_entries: Vec<LinkedStatementEntry>,
) -> Result<LinkedBytecodeCandidate, LinkedBytecodeCandidateError> {
    LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![function_with_statement_entries(
        instruction_count,
        statement_entries,
    )]))
}

fn noncanonical_statement_entries(
    instruction_count: usize,
    statement_entries: Vec<LinkedStatementEntry>,
) -> StatementEntryValidationError {
    match statement_candidate(instruction_count, statement_entries)
        .expect_err("non-canonical statement entries must fail closed")
    {
        LinkedBytecodeCandidateError::NonCanonicalStatementEntries { function, source } => {
            assert_eq!(function, FunctionIndex::new(0));
            source
        }
        other => panic!("expected typed statement-entry error, found {other:?}"),
    }
}

#[test]
fn accepts_same_instruction_sequence_zero_and_one() {
    let candidate = statement_candidate(
        1,
        vec![source_statement(0, 0, 0, 0), source_expression(0, 1, 0, 0)],
    )
    .expect("same-instruction entries with dense sequence ordinals are canonical");

    let entries = candidate.functions()[0].statement_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].instruction(), InstructionIndex::new(0));
    assert_eq!(entries[0].sequence_ordinal(), 0);
    assert_eq!(entries[1].instruction(), InstructionIndex::new(0));
    assert_eq!(entries[1].sequence_ordinal(), 1);
}

#[test]
fn rejects_instruction_order_sequence_gap_and_missing_reset() {
    let order = noncanonical_statement_entries(
        2,
        vec![source_statement(1, 0, 0, 0), source_statement(0, 0, 1, 0)],
    );
    assert!(order.message().contains("pc 0 precedes previous pc 1"));

    let sequence_gap = noncanonical_statement_entries(
        1,
        vec![source_statement(0, 0, 0, 0), source_statement(0, 2, 1, 0)],
    );
    assert!(sequence_gap
        .message()
        .contains("sequenceOrdinal 2, expected 1"));

    let missing_reset = noncanonical_statement_entries(
        2,
        vec![source_statement(0, 0, 0, 0), source_statement(1, 1, 1, 0)],
    );
    assert!(missing_reset
        .message()
        .contains("starts pc 1 at sequenceOrdinal 1, expected 0"));
}

#[test]
fn rejects_duplicate_attribution_id() {
    let duplicate = noncanonical_statement_entries(
        1,
        vec![source_statement(0, 0, 7, 0), source_statement(0, 1, 7, 0)],
    );

    assert!(duplicate.message().contains("repeats attribution id"));
}

#[test]
fn rejects_source_occurrence_and_generated_ordinal_gaps() {
    let statement_gap = noncanonical_statement_entries(
        1,
        vec![source_statement(0, 0, 7, 0), source_statement(0, 1, 7, 2)],
    );
    assert!(statement_gap
        .message()
        .contains("statement attribution occurrences for source index 7"));

    let expression_gap = noncanonical_statement_entries(
        1,
        vec![source_expression(0, 0, 9, 0), source_expression(0, 1, 9, 2)],
    );
    assert!(expression_gap
        .message()
        .contains("expression attribution occurrences for source index 9"));

    let generated_gap = noncanonical_statement_entries(
        1,
        vec![generated_statement(0, 0, 0), generated_statement(0, 1, 2)],
    );
    assert!(generated_gap
        .message()
        .contains("generated attribution ordinals must be dense 0..2"));
}

#[test]
fn rejects_generated_attribution_with_source_site() {
    let generated_source = noncanonical_statement_entries(
        1,
        vec![LinkedStatementEntry::new(
            InstructionIndex::new(0),
            0,
            StatementAttributionId::Generated { ordinal: 0 },
            source_site(1),
        )],
    );

    assert!(generated_source
        .message()
        .contains("Generated attribution id with a source site"));
}

#[test]
fn checks_instruction_bounds_before_canonical_rules() {
    let error = statement_candidate(
        1,
        vec![LinkedStatementEntry::new(
            InstructionIndex::new(1),
            7,
            StatementAttributionId::Generated { ordinal: 3 },
            source_site(1),
        )],
    )
    .expect_err("out-of-bounds statement instruction must fail first");

    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::ReferenceOutOfBounds {
            location: CandidateLocation::Function { function },
            reference: CandidateReferenceKind::Instruction,
            index: 1,
            len: 1,
        } if function == FunctionIndex::new(0)
    ));
}

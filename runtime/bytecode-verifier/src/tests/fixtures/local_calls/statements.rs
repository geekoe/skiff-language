use skiff_artifact_model::{
    InstructionSourceSite, SourceMapEntry, StatementAttributionId, StatementEntry,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{
    InstructionBoundaryIndex, InstructionIndex, LinkedSourceMapEntry, LinkedStatementEntry,
};

use super::LocalCallCandidateCorruption;

pub(super) fn artifact_entries() -> Vec<StatementEntry> {
    vec![
        StatementEntry {
            pc: 0,
            sequence_ordinal: 0,
            attribution_id: statement_id(),
            site: call_site(),
        },
        StatementEntry {
            pc: 0,
            sequence_ordinal: 1,
            attribution_id: expression_id(),
            site: call_site(),
        },
        StatementEntry {
            pc: 4,
            sequence_ordinal: 0,
            attribution_id: generated_id(),
            site: budget_site(),
        },
    ]
}

pub(super) fn artifact_source_map() -> Vec<SourceMapEntry> {
    vec![
        SourceMapEntry {
            start_pc: 0,
            end_pc: 4,
            site: call_site(),
        },
        SourceMapEntry {
            start_pc: 4,
            end_pc: 5,
            site: budget_site(),
        },
    ]
}

pub(super) fn linked_entries(
    corruption: LocalCallCandidateCorruption,
) -> Box<[LinkedStatementEntry]> {
    let exact = || {
        vec![
            linked(0, 0, statement_id(), call_site()),
            linked(0, 1, expression_id(), call_site()),
            linked(1, 0, generated_id(), budget_site()),
        ]
    };
    let entries = match corruption {
        LocalCallCandidateCorruption::StatementInstruction => vec![
            linked(1, 0, statement_id(), call_site()),
            linked(1, 1, expression_id(), call_site()),
            linked(1, 2, generated_id(), budget_site()),
        ],
        LocalCallCandidateCorruption::StatementSequence => vec![
            linked(0, 0, statement_id(), call_site()),
            linked(1, 0, expression_id(), call_site()),
            linked(1, 1, generated_id(), budget_site()),
        ],
        LocalCallCandidateCorruption::StatementAttributionId => vec![
            linked(
                0,
                0,
                StatementAttributionId::Statement {
                    statement_index: 1,
                    occurrence_ordinal: 0,
                },
                call_site(),
            ),
            linked(0, 1, expression_id(), call_site()),
            linked(1, 0, generated_id(), budget_site()),
        ],
        LocalCallCandidateCorruption::StatementSite => vec![
            linked(0, 0, statement_id(), budget_site()),
            linked(0, 1, expression_id(), call_site()),
            linked(1, 0, generated_id(), budget_site()),
        ],
        _ => exact(),
    };
    entries.into_boxed_slice()
}

pub(super) fn linked_source_map() -> Vec<LinkedSourceMapEntry> {
    vec![
        LinkedSourceMapEntry::new(
            InstructionIndex::new(0),
            InstructionBoundaryIndex::new(1),
            call_site(),
        ),
        LinkedSourceMapEntry::new(
            InstructionIndex::new(1),
            InstructionBoundaryIndex::new(2),
            budget_site(),
        ),
    ]
}

fn linked(
    instruction: u32,
    sequence_ordinal: u32,
    attribution_id: StatementAttributionId,
    site: InstructionSourceSite,
) -> LinkedStatementEntry {
    LinkedStatementEntry::new(
        InstructionIndex::new(instruction),
        sequence_ordinal,
        attribution_id,
        site,
    )
}

fn statement_id() -> StatementAttributionId {
    StatementAttributionId::Statement {
        statement_index: 0,
        occurrence_ordinal: 0,
    }
}

fn expression_id() -> StatementAttributionId {
    StatementAttributionId::Expression {
        expression_index: 0,
        occurrence_ordinal: 0,
    }
}

fn generated_id() -> StatementAttributionId {
    StatementAttributionId::Generated { ordinal: 0 }
}

fn call_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
    }
}

fn budget_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    }
}

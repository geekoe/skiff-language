use skiff_artifact_model::{
    InstructionSourceSite, SourceMapEntry, StatementAttributionId, StatementEntry,
    SyntheticInstructionSiteReason,
};
use skiff_runtime_linked_bytecode::{
    InstructionBoundaryIndex, InstructionIndex, LinkedSourceMapEntry, LinkedStatementEntry,
};

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

pub(super) fn artifact_tail_entries() -> Vec<StatementEntry> {
    vec![StatementEntry {
        pc: 0,
        sequence_ordinal: 0,
        attribution_id: expression_id(),
        site: call_site(),
    }]
}

pub(super) fn artifact_call_only_entries() -> Vec<StatementEntry> {
    vec![StatementEntry {
        pc: 0,
        sequence_ordinal: 0,
        attribution_id: expression_id(),
        site: call_site(),
    }]
}

pub(super) fn artifact_tail_source_map() -> Vec<SourceMapEntry> {
    vec![SourceMapEntry {
        start_pc: 0,
        end_pc: 3,
        site: call_site(),
    }]
}

pub(super) fn artifact_resume_source_map() -> Vec<SourceMapEntry> {
    vec![SourceMapEntry {
        start_pc: 0,
        end_pc: 2,
        site: budget_site(),
    }]
}

pub(super) fn artifact_inout_source_map() -> Vec<SourceMapEntry> {
    vec![SourceMapEntry {
        start_pc: 0,
        end_pc: 5,
        site: call_site(),
    }]
}

pub(super) fn linked_entries() -> Box<[LinkedStatementEntry]> {
    vec![
        LinkedStatementEntry::new(InstructionIndex::new(0), 0, statement_id(), call_site()),
        LinkedStatementEntry::new(InstructionIndex::new(0), 1, expression_id(), call_site()),
        LinkedStatementEntry::new(InstructionIndex::new(1), 0, generated_id(), budget_site()),
    ]
    .into_boxed_slice()
}

pub(super) fn linked_source_map() -> Box<[LinkedSourceMapEntry]> {
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
    .into_boxed_slice()
}

pub(super) fn linked_tail_entries() -> Box<[LinkedStatementEntry]> {
    Box::new([LinkedStatementEntry::new(
        InstructionIndex::new(0),
        0,
        expression_id(),
        call_site(),
    )])
}

pub(super) fn linked_call_only_entries() -> Box<[LinkedStatementEntry]> {
    Box::new([LinkedStatementEntry::new(
        InstructionIndex::new(0),
        0,
        expression_id(),
        call_site(),
    )])
}

pub(super) fn linked_tail_source_map() -> Box<[LinkedSourceMapEntry]> {
    Box::new([LinkedSourceMapEntry::new(
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        call_site(),
    )])
}

pub(super) fn linked_resume_source_map() -> Box<[LinkedSourceMapEntry]> {
    Box::new([LinkedSourceMapEntry::new(
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        budget_site(),
    )])
}

pub(super) fn linked_inout_source_map() -> Box<[LinkedSourceMapEntry]> {
    Box::new([LinkedSourceMapEntry::new(
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        call_site(),
    )])
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

use std::collections::{BTreeMap, BTreeSet};

use super::{MirEmissionAnchor, MirSourceEvent, MirSourceEventPlanError};
use crate::mir::MirBlock;

#[derive(Debug, Default)]
struct StatementPlacementCounts {
    source: u32,
    generated: u32,
}

pub(super) fn validate_statement_anchor_cover(
    events: &[MirSourceEvent],
    blocks: &[MirBlock],
) -> Result<(), MirSourceEventPlanError> {
    let mut placements = BTreeMap::<u32, StatementPlacementCounts>::new();
    for event in events {
        let (statement_index, source_authored) = match event.anchor {
            MirEmissionAnchor::Statement {
                statement_index, ..
            } => (statement_index, true),
            MirEmissionAnchor::GeneratedStatement {
                statement_index, ..
            } => (statement_index, false),
            _ => continue,
        };
        let counts = placements.entry(statement_index).or_default();
        let count = if source_authored {
            &mut counts.source
        } else {
            &mut counts.generated
        };
        *count = count.checked_add(1).ok_or_else(|| {
            invalid(format!(
                "MIR statement {statement_index} has too many placement anchors"
            ))
        })?;
    }

    let mut reachable = BTreeSet::new();
    for statement in blocks.iter().flat_map(|block| &block.statements) {
        if !reachable.insert(statement.statement_index) {
            return Err(invalid(format!(
                "MIR statement index {} occurs in more than one block",
                statement.statement_index
            )));
        }
        match placements.get(&statement.statement_index) {
            Some(StatementPlacementCounts {
                source,
                generated: 0,
            }) if *source >= 1 => {}
            Some(StatementPlacementCounts {
                source: 0,
                generated: 1,
            }) => {}
            Some(counts) => {
                return Err(invalid(format!(
                    "MIR statement {} has {} source and {} generated placement anchors; expected one or more source anchors or exactly one generated anchor",
                    statement.statement_index, counts.source, counts.generated
                )))
            }
            None => {
                return Err(invalid(format!(
                    "MIR statement {} has no source or generated placement anchor",
                    statement.statement_index
                )))
            }
        }
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> MirSourceEventPlanError {
    MirSourceEventPlanError::new(message)
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        InstructionSourceSite, SourcePosition, SourceSpanRef, StatementAttributionId,
        SyntheticInstructionSiteReason,
    };

    use super::validate_statement_anchor_cover;
    use crate::mir::{
        MirBlock, MirEmissionAnchor, MirSourceEvent, MirStatementPlacement, MirStmt, MirStmtKind,
    };

    #[test]
    fn reachable_statement_without_a_placement_anchor_fails_closed() {
        let blocks = vec![MirBlock {
            id: 0,
            label: "entry".to_string(),
            statements: vec![MirStmt {
                statement_index: 7,
                span: None,
                kind: MirStmtKind::Return { value: None },
            }],
            successors: Vec::new(),
        }];
        let error = validate_statement_anchor_cover(&[], &blocks)
            .expect_err("missing statement placement must fail");
        assert!(error.message().contains("statement 7 has no"));
    }

    #[test]
    fn collapsed_source_statements_may_share_one_final_statement() {
        validate_statement_anchor_cover(
            &[source_event(7, 0), source_event(7, 1)],
            &single_statement_block(),
        )
        .expect("one or more source placements cover the final statement");
    }

    #[test]
    fn source_and_generated_statement_placements_cannot_mix() {
        let error = validate_statement_anchor_cover(
            &[source_event(7, 0), generated_event(7, 0)],
            &single_statement_block(),
        )
        .expect_err("source and generated placements must be disjoint");
        assert!(error.message().contains("1 source and 1 generated"));
    }

    #[test]
    fn generated_statement_requires_exactly_one_placement() {
        let error = validate_statement_anchor_cover(
            &[generated_event(7, 0), generated_event(7, 1)],
            &single_statement_block(),
        )
        .expect_err("multiple generated placements must fail closed");
        assert!(error.message().contains("0 source and 2 generated"));
    }

    fn single_statement_block() -> Vec<MirBlock> {
        vec![MirBlock {
            id: 0,
            label: "entry".to_string(),
            statements: vec![MirStmt {
                statement_index: 7,
                span: None,
                kind: MirStmtKind::Return { value: None },
            }],
            successors: Vec::new(),
        }]
    }

    fn source_event(statement_index: u32, occurrence_ordinal: u32) -> MirSourceEvent {
        MirSourceEvent {
            attribution_id: StatementAttributionId::Statement {
                statement_index,
                occurrence_ordinal,
            },
            site: InstructionSourceSite::Source {
                span: SourceSpanRef {
                    source_id: 0,
                    start: SourcePosition::new(1, 1),
                    end: SourcePosition::new(1, 2),
                },
            },
            anchor: MirEmissionAnchor::Statement {
                statement_index,
                occurrence_ordinal,
                placement: MirStatementPlacement::BeforeStatement,
            },
        }
    }

    fn generated_event(statement_index: u32, ordinal: u32) -> MirSourceEvent {
        MirSourceEvent {
            attribution_id: StatementAttributionId::Generated { ordinal },
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerDesugaring,
            },
            anchor: MirEmissionAnchor::GeneratedStatement {
                statement_index,
                placement: MirStatementPlacement::BeforeStatement,
            },
        }
    }
}

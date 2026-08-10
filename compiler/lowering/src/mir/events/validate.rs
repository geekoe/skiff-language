use std::collections::BTreeSet;

use skiff_artifact_model::{
    ExprIr, InstructionSourceSite, StatementAttributionId, SyntheticInstructionSiteReason,
};

use super::{
    MirControlFlowEdge, MirEmissionAnchor, MirSourceEvent, MirSourceEventPlan,
    MirSourceEventPlanError, MirSourceEventUnavailableReason,
};
use crate::mir::{MirBlock, MirExpression, MirStmt, MirStmtKind};

mod canonical;

pub(super) use canonical::validate_canonical_events;

pub(crate) fn finalize_mir_source_event_plan(
    plan: MirSourceEventPlan,
    expressions: &[MirExpression],
    blocks: &[MirBlock],
) -> Result<MirSourceEventPlan, MirSourceEventPlanError> {
    if plan.unavailable_reason().is_some() {
        return Ok(plan);
    }
    let mut events = plan.into_events().ok_or_else(|| {
        MirSourceEventPlanError::new("available MIR source event plan lost its event contents")
    })?;
    validate_canonical_events(&events)?;

    for event in &events {
        match validate_anchor(event, expressions, blocks)? {
            AnchorReachability::Reachable => {}
            AnchorReachability::Missing => {
                return Ok(MirSourceEventPlan::unavailable(
                    MirSourceEventUnavailableReason::MirAnchorNotReachable {
                        class: event.attribution_id.class(),
                    },
                ));
            }
        }
    }
    super::statements::validate_statement_anchor_cover(&events, blocks)?;
    append_budget_checkpoint_events(&mut events, blocks)?;
    for event in &events {
        if matches!(event.anchor, MirEmissionAnchor::BudgetCheckpoint { .. }) {
            validate_anchor(event, expressions, blocks)?.require_reachable()?;
        }
    }
    MirSourceEventPlan::checked_available(events)
}

fn validate_anchor(
    event: &MirSourceEvent,
    expressions: &[MirExpression],
    blocks: &[MirBlock],
) -> Result<AnchorReachability, MirSourceEventPlanError> {
    match event.anchor {
        MirEmissionAnchor::Statement {
            statement_index, ..
        }
        | MirEmissionAnchor::GeneratedStatement {
            statement_index, ..
        } => statement(blocks, statement_index).map(|value| {
            if value.is_some() {
                AnchorReachability::Reachable
            } else {
                AnchorReachability::Missing
            }
        }),
        MirEmissionAnchor::Expression {
            expression_index, ..
        } => expression(expressions, expression_index).map(|value| {
            if value.is_some() {
                AnchorReachability::Reachable
            } else {
                AnchorReachability::Missing
            }
        }),
        MirEmissionAnchor::LocalCall {
            expression_index, ..
        } => validate_local_call(expressions, expression_index, &event.site),
        MirEmissionAnchor::TailLocalCallCandidate {
            statement_index,
            expression_index,
            ..
        } => validate_tail_candidate(
            blocks,
            expressions,
            statement_index,
            expression_index,
            &event.site,
        ),
        MirEmissionAnchor::BudgetCheckpoint {
            loop_statement_index,
            edge,
        } => validate_budget_edge(blocks, loop_statement_index, edge),
    }
}

fn validate_local_call(
    expressions: &[MirExpression],
    expression_index: u32,
    event_site: &InstructionSourceSite,
) -> Result<AnchorReachability, MirSourceEventPlanError> {
    let Some(expression) = expression(expressions, expression_index)? else {
        return Ok(AnchorReachability::Missing);
    };
    let ExprIr::Call { call } = &expression.expression else {
        return invalid(format!(
            "local-call anchor references non-call expression {expression_index}"
        ));
    };
    if !crate::mir::abi::is_direct_target(&call.target) {
        return invalid(format!(
            "local-call anchor references non-direct expression {expression_index}"
        ));
    }
    if crate::task_call::is_task_submit_call(call) {
        return invalid(format!(
            "local-call anchor references dispatch expression {expression_index}"
        ));
    }
    if &call.site != event_site {
        return invalid(format!(
            "local-call source event site does not match call expression {expression_index} site"
        ));
    }
    Ok(AnchorReachability::Reachable)
}

fn validate_tail_candidate(
    blocks: &[MirBlock],
    expressions: &[MirExpression],
    statement_index: u32,
    expression_index: u32,
    event_site: &InstructionSourceSite,
) -> Result<AnchorReachability, MirSourceEventPlanError> {
    if validate_local_call(expressions, expression_index, event_site)?
        == AnchorReachability::Missing
    {
        return Ok(AnchorReachability::Missing);
    }
    let Some((_, statement)) = statement(blocks, statement_index)? else {
        return Ok(AnchorReachability::Missing);
    };
    if !matches!(
        &statement.kind,
        MirStmtKind::Return {
            value: Some(value)
        } if value.expression == expression_index
    ) {
        return invalid(format!(
            "tail-local anchor statement {statement_index} does not directly return expression {expression_index}"
        ));
    }
    Ok(AnchorReachability::Reachable)
}

fn append_budget_checkpoint_events(
    events: &mut Vec<MirSourceEvent>,
    blocks: &[MirBlock],
) -> Result<(), MirSourceEventPlanError> {
    let generated_count = events
        .iter()
        .filter(|event| {
            matches!(
                event.attribution_id,
                StatementAttributionId::Generated { .. }
            )
        })
        .count();
    let mut next_ordinal = u32::try_from(generated_count)
        .map_err(|_| MirSourceEventPlanError::new("too many generated MIR source events"))?;
    for block in blocks {
        for statement in &block.statements {
            let Some((body, continuation)) = loop_targets(block, statement) else {
                continue;
            };
            for edge in loop_back_edges(blocks, body, block.id, continuation)? {
                events.push(MirSourceEvent {
                    attribution_id: StatementAttributionId::Generated {
                        ordinal: next_ordinal,
                    },
                    site: InstructionSourceSite::Synthetic {
                        reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                    },
                    anchor: MirEmissionAnchor::BudgetCheckpoint {
                        loop_statement_index: statement.statement_index,
                        edge,
                    },
                });
                next_ordinal = next_ordinal.checked_add(1).ok_or_else(|| {
                    MirSourceEventPlanError::new(
                        "generated budget checkpoint ordinal exceeds u32::MAX",
                    )
                })?;
            }
        }
    }
    Ok(())
}

fn validate_budget_edge(
    blocks: &[MirBlock],
    loop_statement_index: u32,
    edge: MirControlFlowEdge,
) -> Result<AnchorReachability, MirSourceEventPlanError> {
    let Some((header, statement)) = statement(blocks, loop_statement_index)? else {
        return Ok(AnchorReachability::Missing);
    };
    let Some((body, continuation)) = loop_targets(header, statement) else {
        return invalid(format!(
            "budget checkpoint references non-loop statement {loop_statement_index}"
        ));
    };
    if edge.to_block() != header.id
        || !loop_back_edges(blocks, body, header.id, continuation)?.contains(&edge)
    {
        return invalid(format!(
            "budget checkpoint for statement {loop_statement_index} references non-loop edge {} -> {}",
            edge.from_block(),
            edge.to_block()
        ));
    }
    Ok(AnchorReachability::Reachable)
}

fn loop_targets(header: &MirBlock, statement: &MirStmt) -> Option<(u32, Option<u32>)> {
    match &statement.kind {
        MirStmtKind::While { body, .. } => Some((
            *body,
            header
                .successors
                .iter()
                .copied()
                .find(|successor| successor != body),
        )),
        MirStmtKind::ForIn {
            body, continuation, ..
        } => Some((*body, Some(*continuation))),
        _ => None,
    }
}

fn loop_back_edges(
    blocks: &[MirBlock],
    body: u32,
    header: u32,
    continuation: Option<u32>,
) -> Result<BTreeSet<MirControlFlowEdge>, MirSourceEventPlanError> {
    let mut pending = vec![body];
    let mut seen = BTreeSet::new();
    let mut edges = BTreeSet::new();
    while let Some(block_id) = pending.pop() {
        if block_id == header || Some(block_id) == continuation || !seen.insert(block_id) {
            continue;
        }
        let block = block_by_id(blocks, block_id)?.ok_or_else(|| {
            MirSourceEventPlanError::new(format!(
                "loop body references missing MIR block {block_id}"
            ))
        })?;
        for successor in &block.successors {
            if *successor == header {
                edges.insert(MirControlFlowEdge::new(block.id, header));
            } else if Some(*successor) != continuation {
                pending.push(*successor);
            }
        }
    }
    Ok(edges)
}

fn statement(
    blocks: &[MirBlock],
    statement_index: u32,
) -> Result<Option<(&MirBlock, &MirStmt)>, MirSourceEventPlanError> {
    let mut matches = blocks.iter().flat_map(|block| {
        block
            .statements
            .iter()
            .filter(move |statement| statement.statement_index == statement_index)
            .map(move |statement| (block, statement))
    });
    let first = matches.next();
    if matches.next().is_some() {
        return invalid(format!(
            "MIR statement index {statement_index} occurs in more than one block"
        ));
    }
    Ok(first)
}

fn expression(
    expressions: &[MirExpression],
    expression_index: u32,
) -> Result<Option<&MirExpression>, MirSourceEventPlanError> {
    let Some(expression) = expressions.get(expression_index as usize) else {
        return Ok(None);
    };
    if expression.index != expression_index {
        return invalid(format!(
            "MIR expression lookup {expression_index} found stored index {}",
            expression.index
        ));
    }
    Ok(Some(expression))
}

fn block_by_id(
    blocks: &[MirBlock],
    block_id: u32,
) -> Result<Option<&MirBlock>, MirSourceEventPlanError> {
    let Some(block) = blocks.get(block_id as usize) else {
        return Ok(None);
    };
    if block.id != block_id {
        return invalid(format!(
            "MIR block lookup {block_id} found stored id {}",
            block.id
        ));
    }
    Ok(Some(block))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnchorReachability {
    Reachable,
    Missing,
}

impl AnchorReachability {
    fn require_reachable(self) -> Result<(), MirSourceEventPlanError> {
        match self {
            Self::Reachable => Ok(()),
            Self::Missing => invalid("generated MIR source anchor is missing"),
        }
    }
}

fn invalid<T>(message: impl Into<String>) -> Result<T, MirSourceEventPlanError> {
    Err(MirSourceEventPlanError::new(message))
}

use std::collections::{BTreeMap, BTreeSet, HashSet};

use skiff_artifact_model::{InstructionSourceSite, StatementAttributionId};

use super::super::{MirEmissionAnchor, MirSourceEvent, MirSourceEventPlanError};

pub(in crate::mir::events) fn validate_canonical_events(
    events: &[MirSourceEvent],
) -> Result<(), MirSourceEventPlanError> {
    let mut ids = HashSet::with_capacity(events.len());
    let mut anchors = HashSet::with_capacity(events.len());
    let mut statements = BTreeMap::<u32, (u64, u32)>::new();
    let mut expressions = BTreeMap::<u32, (u64, u32)>::new();
    let mut generated = (0_u64, 0_u32);
    let mut specialized_calls = BTreeSet::new();

    for event in events {
        if !ids.insert(event.attribution_id) {
            return invalid(format!(
                "duplicate MIR source attribution {:?}",
                event.attribution_id
            ));
        }
        if !anchors.insert(event.anchor) {
            return invalid(format!(
                "duplicate MIR source emission anchor {:?}",
                event.anchor
            ));
        }
        validate_attribution_anchor(event)?;
        match event.attribution_id {
            StatementAttributionId::Statement {
                statement_index,
                occurrence_ordinal,
            } => record_occurrence(&mut statements, statement_index, occurrence_ordinal),
            StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            } => {
                record_occurrence(&mut expressions, expression_index, occurrence_ordinal);
                if matches!(
                    event.anchor,
                    MirEmissionAnchor::LocalCall { .. }
                        | MirEmissionAnchor::TailLocalCallCandidate { .. }
                ) && !specialized_calls.insert(expression_index)
                {
                    return invalid(format!(
                        "expression {expression_index} has more than one local-call emission anchor"
                    ));
                }
            }
            StatementAttributionId::Generated { ordinal } => {
                generated.0 += 1;
                generated.1 = generated.1.max(ordinal);
                if !matches!(event.site, InstructionSourceSite::Synthetic { .. }) {
                    return invalid(format!(
                        "generated MIR source event {ordinal} does not have a synthetic site"
                    ));
                }
            }
        }
    }
    validate_dense("statement", &statements)?;
    validate_dense("expression", &expressions)?;
    if generated.0 > 0 && u64::from(generated.1) + 1 != generated.0 {
        return invalid(format!(
            "generated MIR source event ordinals must be dense 0..{}, found {} events",
            generated.1, generated.0
        ));
    }
    Ok(())
}

fn validate_attribution_anchor(event: &MirSourceEvent) -> Result<(), MirSourceEventPlanError> {
    if let (
        StatementAttributionId::Expression {
            occurrence_ordinal: id_occurrence,
            ..
        },
        MirEmissionAnchor::LocalCall {
            occurrence_ordinal: anchor_occurrence,
            ..
        }
        | MirEmissionAnchor::TailLocalCallCandidate {
            occurrence_ordinal: anchor_occurrence,
            ..
        },
    ) = (event.attribution_id, event.anchor)
    {
        if id_occurrence != 0 || anchor_occurrence != 0 {
            return invalid(
                "specialized local-call anchors require expression occurrence ordinal zero",
            );
        }
    }

    let matches = match (event.attribution_id, event.anchor) {
        (
            StatementAttributionId::Statement {
                statement_index: id_index,
                occurrence_ordinal: id_occurrence,
            },
            MirEmissionAnchor::Statement {
                statement_index: anchor_index,
                occurrence_ordinal: anchor_occurrence,
                ..
            },
        ) => id_index == anchor_index && id_occurrence == anchor_occurrence,
        (
            StatementAttributionId::Expression {
                expression_index: id_index,
                occurrence_ordinal: id_occurrence,
            },
            MirEmissionAnchor::Expression {
                expression_index: anchor_index,
                occurrence_ordinal: anchor_occurrence,
            }
            | MirEmissionAnchor::LocalCall {
                expression_index: anchor_index,
                occurrence_ordinal: anchor_occurrence,
            }
            | MirEmissionAnchor::TailLocalCallCandidate {
                expression_index: anchor_index,
                occurrence_ordinal: anchor_occurrence,
                ..
            },
        ) => id_index == anchor_index && id_occurrence == anchor_occurrence,
        (
            StatementAttributionId::Generated { .. },
            MirEmissionAnchor::BudgetCheckpoint { .. }
            | MirEmissionAnchor::GeneratedStatement { .. },
        ) => true,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        invalid(format!(
            "MIR source attribution {:?} does not match anchor {:?}",
            event.attribution_id, event.anchor
        ))
    }
}

fn record_occurrence(
    occurrences: &mut BTreeMap<u32, (u64, u32)>,
    final_index: u32,
    occurrence_ordinal: u32,
) {
    let row = occurrences.entry(final_index).or_insert((0, 0));
    row.0 += 1;
    row.1 = row.1.max(occurrence_ordinal);
}

fn validate_dense(
    label: &str,
    occurrences: &BTreeMap<u32, (u64, u32)>,
) -> Result<(), MirSourceEventPlanError> {
    for (final_index, (count, maximum)) in occurrences {
        if u64::from(*maximum) + 1 != *count {
            return invalid(format!(
                "{label} {final_index} occurrence ordinals must be dense 0..{maximum}, found {count} events"
            ));
        }
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, MirSourceEventPlanError> {
    Err(MirSourceEventPlanError::new(message))
}

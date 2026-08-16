use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    InstructionSourceSite, StatementAttributionId, SyntheticInstructionSiteReason,
};
use skiff_compiler_source::{
    ExpressionKey, ExpressionOwnerKey, SourceEventFacts, SourceEventKey, SourceStatementKey,
};

use super::{
    MirEmissionAnchor, MirSourceEvent, MirSourceEventPlan, MirSourceEventPlanError,
    MirSourceEventUnavailableReason, MirStatementPlacement,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExpressionEventKind {
    Expression,
    LocalCall,
}

/// Function-local collector that joins source keys only after lowering has
/// produced exact File IR table indices.
pub(crate) struct MirSourceEventCollector<'a> {
    module_path: &'a str,
    owner: Option<ExpressionOwnerKey>,
    source_facts: Option<&'a SourceEventFacts>,
    next_statement_preorder: u32,
    statement_occurrences: BTreeMap<u32, u32>,
    expression_occurrences: BTreeMap<u32, u32>,
    next_generated_ordinal: u32,
    observed: BTreeSet<SourceEventKey>,
    unrepresented: BTreeSet<SourceEventKey>,
    events: Vec<MirSourceEvent>,
}

impl<'a> MirSourceEventCollector<'a> {
    pub(crate) fn new(
        module_path: &'a str,
        owner: Option<ExpressionOwnerKey>,
        source_facts: Option<&'a SourceEventFacts>,
    ) -> Self {
        Self {
            module_path,
            owner,
            source_facts,
            next_statement_preorder: 0,
            statement_occurrences: BTreeMap::new(),
            expression_occurrences: BTreeMap::new(),
            next_generated_ordinal: 0,
            observed: BTreeSet::new(),
            unrepresented: BTreeSet::new(),
            events: Vec::new(),
        }
    }

    pub(crate) fn next_statement_key(
        &mut self,
    ) -> Result<Option<SourceEventKey>, MirSourceEventPlanError> {
        let preorder = self.next_statement_preorder;
        self.next_statement_preorder = preorder.checked_add(1).ok_or_else(|| {
            MirSourceEventPlanError::new("source statement preorder exceeds u32::MAX")
        })?;
        Ok(self.owner.as_ref().map(|owner| {
            SourceEventKey::Statement(SourceStatementKey::new(
                self.module_path,
                owner.clone(),
                preorder,
            ))
        }))
    }

    pub(crate) fn record_statement(
        &mut self,
        key: Option<SourceEventKey>,
        statement_index: u32,
    ) -> Result<(), MirSourceEventPlanError> {
        let Some(key) = key else {
            return Ok(());
        };
        if self.source_facts.is_none() {
            return Ok(());
        }
        if !matches!(key, SourceEventKey::Statement(_)) {
            return Err(MirSourceEventPlanError::new(
                "expression source key cannot map to a statement index",
            ));
        }
        let occurrence_ordinal = next_occurrence(
            &mut self.statement_occurrences,
            statement_index,
            "statement",
        )?;
        let site = self.record_source_key(&key)?;
        self.events.push(MirSourceEvent {
            attribution_id: StatementAttributionId::Statement {
                statement_index,
                occurrence_ordinal,
            },
            site,
            anchor: MirEmissionAnchor::Statement {
                statement_index,
                occurrence_ordinal,
                placement: MirStatementPlacement::BeforeStatement,
            },
        });
        Ok(())
    }

    pub(crate) fn record_expression(
        &mut self,
        key: Option<ExpressionKey>,
        expression_index: u32,
        kind: ExpressionEventKind,
    ) -> Result<(), MirSourceEventPlanError> {
        let Some(key) = key.map(SourceEventKey::Expression) else {
            return Ok(());
        };
        if self.source_facts.is_none() {
            return Ok(());
        }
        let occurrence_ordinal = next_occurrence(
            &mut self.expression_occurrences,
            expression_index,
            "expression",
        )?;
        let site = self.record_source_key(&key)?;
        let anchor = match (kind, occurrence_ordinal) {
            (ExpressionEventKind::LocalCall, 0) => MirEmissionAnchor::LocalCall {
                expression_index,
                occurrence_ordinal,
            },
            _ => MirEmissionAnchor::Expression {
                expression_index,
                occurrence_ordinal,
            },
        };
        self.events.push(MirSourceEvent {
            attribution_id: StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            },
            site,
            anchor,
        });
        Ok(())
    }

    pub(crate) fn note_unrepresented_expression(
        &mut self,
        key: Option<ExpressionKey>,
    ) -> Result<(), MirSourceEventPlanError> {
        let Some(key) = key.map(SourceEventKey::Expression) else {
            return Ok(());
        };
        self.ensure_source_key(&key)?;
        if !self.observed.insert(key.clone()) {
            return Err(duplicate_source_key(&key));
        }
        self.unrepresented.insert(key);
        Ok(())
    }

    pub(crate) fn record_tail_expression_for_statement(
        &mut self,
        key: Option<SourceEventKey>,
        expression_index: u32,
    ) -> Result<(), MirSourceEventPlanError> {
        let Some(key) = key else {
            return Ok(());
        };
        if self.source_facts.is_none() {
            return Ok(());
        }
        if !matches!(key, SourceEventKey::Statement(_)) {
            return Err(MirSourceEventPlanError::new(
                "tail expression source key must be a statement key",
            ));
        }
        let occurrence_ordinal = next_occurrence(
            &mut self.expression_occurrences,
            expression_index,
            "expression",
        )?;
        let site = self.record_source_key(&key)?;
        self.events.push(MirSourceEvent {
            attribution_id: StatementAttributionId::Expression {
                expression_index,
                occurrence_ordinal,
            },
            site,
            anchor: MirEmissionAnchor::Expression {
                expression_index,
                occurrence_ordinal,
            },
        });
        Ok(())
    }

    pub(crate) fn promote_tail_local_candidate(
        &mut self,
        statement_index: u32,
        expression_index: u32,
    ) -> Result<(), MirSourceEventPlanError> {
        let mut promoted = 0_u32;
        for event in &mut self.events {
            let MirEmissionAnchor::LocalCall {
                expression_index: anchored_expression,
                occurrence_ordinal,
            } = event.anchor
            else {
                continue;
            };
            if anchored_expression != expression_index {
                continue;
            }
            event.anchor = MirEmissionAnchor::TailLocalCallCandidate {
                statement_index,
                expression_index,
                occurrence_ordinal,
            };
            promoted = promoted
                .checked_add(1)
                .ok_or_else(|| MirSourceEventPlanError::new("too many tail-local source events"))?;
        }
        if promoted > 1 {
            return Err(MirSourceEventPlanError::new(format!(
                "expression {expression_index} has multiple local-call anchors"
            )));
        }
        Ok(())
    }

    pub(crate) fn mark_dispatched_call(
        &mut self,
        expression_index: u32,
    ) -> Result<(), MirSourceEventPlanError> {
        let mut demoted = 0_u32;
        for event in &mut self.events {
            let occurrence_ordinal = match event.anchor {
                MirEmissionAnchor::LocalCall {
                    expression_index: anchored_expression,
                    occurrence_ordinal,
                }
                | MirEmissionAnchor::TailLocalCallCandidate {
                    expression_index: anchored_expression,
                    occurrence_ordinal,
                    ..
                } if anchored_expression == expression_index => occurrence_ordinal,
                _ => continue,
            };
            event.anchor = MirEmissionAnchor::Expression {
                expression_index,
                occurrence_ordinal,
            };
            demoted = demoted.checked_add(1).ok_or_else(|| {
                MirSourceEventPlanError::new("too many dispatched-call source events")
            })?;
        }
        if demoted > 1 {
            return Err(MirSourceEventPlanError::new(format!(
                "dispatch expression {expression_index} has multiple local-call anchors"
            )));
        }
        Ok(())
    }

    pub(crate) fn record_generated_statement(
        &mut self,
        statement_index: u32,
        reason: SyntheticInstructionSiteReason,
    ) -> Result<(), MirSourceEventPlanError> {
        let ordinal = self.take_generated_ordinal()?;
        self.events.push(MirSourceEvent {
            attribution_id: StatementAttributionId::Generated { ordinal },
            site: InstructionSourceSite::Synthetic { reason },
            anchor: MirEmissionAnchor::GeneratedStatement {
                statement_index,
                placement: MirStatementPlacement::BeforeStatement,
            },
        });
        Ok(())
    }

    pub(crate) fn finish(&self) -> Result<MirSourceEventPlan, MirSourceEventPlanError> {
        let Some(source_facts) = self.source_facts else {
            return Ok(MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ));
        };
        let Some(owner) = self.owner.as_ref() else {
            return Ok(MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ));
        };
        if !source_facts.contains_owner(self.module_path, owner) {
            return Ok(MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceOwnerNotProvided,
            ));
        }

        for fact in source_facts.iter().filter(|fact| {
            fact.key().module_path() == self.module_path && fact.key().owner() == owner
        }) {
            if !self.observed.contains(fact.key()) {
                return Ok(MirSourceEventPlan::unavailable(
                    MirSourceEventUnavailableReason::SourceEventNotRepresentable {
                        class: fact.key().attribution_class(),
                    },
                ));
            }
        }
        if let Some(key) = self.unrepresented.iter().next() {
            return Ok(MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceEventNotRepresentable {
                    class: key.attribution_class(),
                },
            ));
        }
        MirSourceEventPlan::checked_available(self.events.clone())
    }

    fn record_source_key(
        &mut self,
        key: &SourceEventKey,
    ) -> Result<InstructionSourceSite, MirSourceEventPlanError> {
        self.ensure_source_key(key)?;
        if !self.observed.insert(key.clone()) {
            return Err(duplicate_source_key(key));
        }
        let site = self
            .source_facts
            .and_then(|facts| facts.fact(key))
            .map(|fact| fact.site().clone())
            .ok_or_else(|| missing_source_key(key))?;
        if !matches!(site, InstructionSourceSite::Source { .. }) {
            return Err(MirSourceEventPlanError::new(format!(
                "source event {key:?} has a synthetic site"
            )));
        }
        Ok(site)
    }

    fn ensure_source_key(&self, key: &SourceEventKey) -> Result<(), MirSourceEventPlanError> {
        let Some(source_facts) = self.source_facts else {
            return Ok(());
        };
        if source_facts.fact(key).is_none() {
            return Err(missing_source_key(key));
        }
        Ok(())
    }

    fn take_generated_ordinal(&mut self) -> Result<u32, MirSourceEventPlanError> {
        let ordinal = self.next_generated_ordinal;
        self.next_generated_ordinal = ordinal.checked_add(1).ok_or_else(|| {
            MirSourceEventPlanError::new("generated source event ordinal exceeds u32::MAX")
        })?;
        Ok(ordinal)
    }
}

fn next_occurrence(
    occurrences: &mut BTreeMap<u32, u32>,
    final_index: u32,
    label: &str,
) -> Result<u32, MirSourceEventPlanError> {
    let next = occurrences.entry(final_index).or_default();
    let occurrence = *next;
    *next = next.checked_add(1).ok_or_else(|| {
        MirSourceEventPlanError::new(format!(
            "{label} {final_index} occurrence ordinal exceeds u32::MAX"
        ))
    })?;
    Ok(occurrence)
}

fn duplicate_source_key(key: &SourceEventKey) -> MirSourceEventPlanError {
    MirSourceEventPlanError::new(format!("source event {key:?} is consumed more than once"))
}

fn missing_source_key(key: &SourceEventKey) -> MirSourceEventPlanError {
    MirSourceEventPlanError::new(format!("source event facts omit consumed key {key:?}"))
}

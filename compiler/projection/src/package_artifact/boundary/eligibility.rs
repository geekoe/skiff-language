use skiff_artifact_model::{
    BoundaryOperationContract, BoundaryUnavailableReason, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValuePlan, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, CallableTargetFact,
};

use super::ordering::escape_lane_rank;

pub(super) fn semantic_unavailable_reasons(
    facts: &CallableSemanticFacts,
    operation_contract: Option<&BoundaryOperationContract>,
) -> Vec<BoundaryUnavailableReason> {
    let mut reasons = Vec::new();
    let detached_parameters = operation_contract.is_some_and(canonical_detached_parameters);
    match &facts.effects {
        CallableEffectSummary::Unknown { .. } => {
            push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
        }
        CallableEffectSummary::Analyzed { effects } => {
            effect_unavailable_reasons(effects, &mut reasons);
        }
    }
    match &facts.provenance {
        CallableProvenanceSummary::Unknown { reason } => match reason {
            skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending => {
                push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnsupportedControlFlow => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnsupportedHeapStore => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnknownCallTarget => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownCallTarget);
            }
        },
        CallableProvenanceSummary::Analyzed { escape_lanes, .. } => {
            for lane in escape_lanes {
                if matches!(lane, skiff_artifact_model::ValueEscapeLane::Database)
                    && detached_parameters
                {
                    continue;
                }
                push_reason(
                    &mut reasons,
                    BoundaryUnavailableReason::EscapesCallerValue { lane: *lane },
                );
            }
        }
    }
    if matches!(
        facts.effects,
        CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: true,
                ..
            }
        }
    ) && !reasons
        .iter()
        .any(|reason| matches!(reason, BoundaryUnavailableReason::EscapesCallerValue { .. }))
        && !(detached_parameters && has_only_materialized_database_escape(facts))
    {
        push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
    }
    if facts
        .resolved_call_targets
        .values()
        .any(|target| matches!(target, CallableTargetFact::Unknown))
    {
        push_reason(&mut reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
    reasons
}

fn effect_unavailable_reasons(
    effects: &CallableMayEffects,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    if effects.requires_same_heap_identity {
        push_reason(reasons, BoundaryUnavailableReason::RequiresSameHeapIdentity);
    }
    if effects.invokes_unknown_target {
        push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
}

fn has_only_materialized_database_escape(facts: &CallableSemanticFacts) -> bool {
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if !escape_lanes.is_empty()
                && escape_lanes
                .iter()
                .all(|lane| matches!(lane, skiff_artifact_model::ValueEscapeLane::Database))
    )
}

fn canonical_detached_parameters(contract: &BoundaryOperationContract) -> bool {
    contract
        .parameters
        .iter()
        .all(|parameter| canonical_detached_plan(&parameter.value_plan))
}

fn canonical_detached_plan(plan: &BoundaryValuePlan) -> bool {
    matches!(
        plan,
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: BoundaryValueEncoding::CanonicalValue,
            ..
        }
    )
}

pub(super) fn push_reason(
    reasons: &mut Vec<BoundaryUnavailableReason>,
    reason: BoundaryUnavailableReason,
) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

pub(super) fn normalize_reasons(reasons: &mut Vec<BoundaryUnavailableReason>) {
    reasons.sort_by_key(reason_sort_key);
    reasons.dedup();
}

fn reason_sort_key(reason: &BoundaryUnavailableReason) -> (u8, u8) {
    match reason {
        BoundaryUnavailableReason::AnalysisPending => (0, 0),
        BoundaryUnavailableReason::UnknownEffect => (1, 0),
        BoundaryUnavailableReason::UnknownCallTarget => (2, 0),
        BoundaryUnavailableReason::EscapesCallerValue { lane } => (3, escape_lane_rank(*lane)),
        BoundaryUnavailableReason::RequiresSameHeapIdentity => (4, 0),
        BoundaryUnavailableReason::CallbackAdapterUnavailable => (5, 0),
        BoundaryUnavailableReason::NativeAdapterUnavailable => (6, 0),
        BoundaryUnavailableReason::UnsupportedBoundaryType => (7, 0),
        BoundaryUnavailableReason::UnsupportedStream => (8, 0),
        BoundaryUnavailableReason::InOutNotAllowedAtServiceBoundary => (9, 0),
    }
}

#[cfg(test)]
mod tests;

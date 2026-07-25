use skiff_artifact_model::{
    BoundaryOperationContract, BoundaryUnavailableReason, BoundaryValueCarrier,
    BoundaryValueEncoding, BoundaryValuePlan, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, CallableTargetFact, ValueProvenance,
};

use super::ordering::escape_lane_rank;

pub(super) fn semantic_unavailable_reasons(
    facts: &CallableSemanticFacts,
    operation_contract: Option<&BoundaryOperationContract>,
) -> Vec<BoundaryUnavailableReason> {
    let mut reasons = Vec::new();
    let detached_wrapped_return =
        detached_wrapped_return_is_materialized(facts, operation_contract);
    let detached_parameters = operation_contract.is_some_and(canonical_detached_parameters);
    match &facts.effects {
        CallableEffectSummary::Unknown { .. } => {
            push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
        }
        CallableEffectSummary::Analyzed { effects } => {
            effect_unavailable_reasons(
                *effects,
                detached_wrapped_return,
                detached_parameters,
                facts,
                &mut reasons,
            );
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
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
            ..
        } => {
            if return_origins.iter().any(is_caller_parameter_origin)
                && (analyzed_effects(facts).is_some_and(|effects| effects.returns_caller_alias)
                    || !return_origins
                        .iter()
                        .any(|origin| matches!(origin, ValueProvenance::Fresh)))
                && !detached_wrapped_return
            {
                push_reason(&mut reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
            }
            if throw_origins.iter().any(is_caller_parameter_origin) {
                push_reason(&mut reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
            }
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
    effects: CallableMayEffects,
    detached_wrapped_return: bool,
    detached_parameters: bool,
    facts: &CallableSemanticFacts,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    if effects.writes_caller_reachable {
        push_reason(reasons, BoundaryUnavailableReason::WritesCallerReachable);
    }
    if effects.returns_caller_alias && !detached_wrapped_return {
        push_reason(reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
    }
    if effects.throws_caller_alias {
        push_reason(reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
    }
    if effects.requires_same_heap_identity
        && !(detached_parameters && has_only_materialized_database_escape(facts))
    {
        push_reason(reasons, BoundaryUnavailableReason::RequiresSameHeapIdentity);
    }
    if effects.invokes_unknown_target {
        push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
}

fn detached_wrapped_return_is_materialized(
    facts: &CallableSemanticFacts,
    operation_contract: Option<&BoundaryOperationContract>,
) -> bool {
    let Some(operation_contract) = operation_contract else {
        return false;
    };
    let BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        ..
    } = &operation_contract.return_value.value_plan
    else {
        return false;
    };
    let CallableEffectSummary::Analyzed { effects } = &facts.effects else {
        return false;
    };
    if !effects.returns_caller_alias
        || effects.invokes_unknown_target
        || !has_no_unmaterialized_escape(facts)
    {
        return false;
    }
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed {
            return_origins,
            direct_return_origins,
            escape_lanes,
            ..
        } if escape_lanes
                .iter()
                .all(|lane| matches!(lane, skiff_artifact_model::ValueEscapeLane::Database))
            && return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::Fresh))
            && return_origins
                .iter()
                .any(is_caller_parameter_origin)
            && direct_return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::Fresh))
            && direct_return_origins.iter().all(|origin| {
                matches!(origin, ValueProvenance::Fresh | ValueProvenance::Constant)
            })
    )
}

fn is_caller_parameter_origin(origin: &ValueProvenance) -> bool {
    matches!(
        origin,
        ValueProvenance::CallerParameter { .. } | ValueProvenance::CallerParameterProjection { .. }
    )
}

fn has_no_unmaterialized_escape(facts: &CallableSemanticFacts) -> bool {
    matches!(
        &facts.provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if escape_lanes
                .iter()
                .all(|lane| matches!(lane, skiff_artifact_model::ValueEscapeLane::Database))
    )
}

fn analyzed_effects(facts: &CallableSemanticFacts) -> Option<CallableMayEffects> {
    match facts.effects {
        CallableEffectSummary::Analyzed { effects } => Some(effects),
        CallableEffectSummary::Unknown { .. } => None,
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
        BoundaryUnavailableReason::WritesCallerReachable => (3, 0),
        BoundaryUnavailableReason::ReturnsCallerAlias => (4, 0),
        BoundaryUnavailableReason::ThrowsCallerAlias => (5, 0),
        BoundaryUnavailableReason::EscapesCallerValue { lane } => (6, escape_lane_rank(*lane)),
        BoundaryUnavailableReason::RequiresSameHeapIdentity => (7, 0),
        BoundaryUnavailableReason::CallbackAdapterUnavailable => (8, 0),
        BoundaryUnavailableReason::NativeAdapterUnavailable => (9, 0),
        BoundaryUnavailableReason::UnsupportedBoundaryType => (10, 0),
        BoundaryUnavailableReason::UnsupportedStream => (11, 0),
    }
}

use skiff_artifact_model::{
    BoundaryUnavailableReason, CallableEffectSummary, CallableMayEffects,
    CallableProvenanceSummary, CallableSemanticFacts, CallableTargetFact, ValueEscapeLane,
    ValueProvenance,
};

pub(super) fn semantic_unavailable_reasons(
    facts: &CallableSemanticFacts,
) -> Vec<BoundaryUnavailableReason> {
    let mut reasons = Vec::new();
    match &facts.effects {
        CallableEffectSummary::Unknown { .. } => {
            push_reason(&mut reasons, BoundaryUnavailableReason::AnalysisPending);
            push_reason(&mut reasons, BoundaryUnavailableReason::UnknownEffect);
        }
        CallableEffectSummary::Analyzed { effects } => {
            effect_unavailable_reasons(*effects, &mut reasons);
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
            skiff_artifact_model::CallableProvenanceUnknownReason::UnknownCallTarget => {
                push_reason(&mut reasons, BoundaryUnavailableReason::UnknownCallTarget);
            }
        },
        CallableProvenanceSummary::Analyzed {
            return_origins,
            throw_origins,
            escape_lanes,
        } => {
            if return_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::CallerParameter { .. }))
            {
                push_reason(&mut reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
            }
            if throw_origins
                .iter()
                .any(|origin| matches!(origin, ValueProvenance::CallerParameter { .. }))
            {
                push_reason(&mut reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
            }
            for lane in escape_lanes {
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
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    if effects.writes_caller_reachable {
        push_reason(reasons, BoundaryUnavailableReason::WritesCallerReachable);
    }
    if effects.returns_caller_alias {
        push_reason(reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
    }
    if effects.throws_caller_alias {
        push_reason(reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
    }
    if effects.requires_same_heap_identity {
        push_reason(reasons, BoundaryUnavailableReason::RequiresSameHeapIdentity);
    }
    if effects.invokes_unknown_target {
        push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
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

fn escape_lane_rank(lane: ValueEscapeLane) -> u8 {
    match lane {
        ValueEscapeLane::Capture => 0,
        ValueEscapeLane::Callback => 1,
        ValueEscapeLane::Stream => 2,
        ValueEscapeLane::Spawn => 3,
        ValueEscapeLane::Database => 4,
        ValueEscapeLane::Native => 5,
        ValueEscapeLane::External => 6,
    }
}

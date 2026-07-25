use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryCancellationContract, BoundaryErrorContract,
    BoundaryImplementationRequirements, BoundaryOperationContract, BoundaryStreamContract,
    BoundaryUnavailableReason, BoundaryValuePlan, BoundaryValuePlanUnavailableReason,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CallableTargetFact, PackageCallableId, ValueEscapeLane, ValueProvenance,
};

use super::{ProjectionError, ProjectionResult};

/// Re-derive boundary eligibility from typed facts instead of trusting an
/// artifact's `Available` discriminant or its duplicated requirements.
pub(super) fn validate_boundary_eligibility(
    callable_id: &PackageCallableId,
    contract: &BoundaryOperationContract,
    facts: &CallableSemanticFacts,
    requirements: &BoundaryImplementationRequirements,
) -> ProjectionResult<()> {
    let mut reasons = Vec::new();
    validate_effects(contract, &facts.effects, &facts.provenance, &mut reasons);
    validate_provenance(contract, &facts.effects, &facts.provenance, &mut reasons);
    validate_call_targets(facts, &mut reasons);
    validate_contract_features(contract, &mut reasons);

    // V1 has no typed native-adapter proof in PackageArtifact. An Available
    // projection therefore cannot turn an asserted native requirement into
    // adapter availability merely by carrying a matching string binding.
    if !requirements.native_capabilities.is_empty() {
        push_reason(
            &mut reasons,
            BoundaryUnavailableReason::NativeAdapterUnavailable,
        );
    }

    normalize_reasons(&mut reasons);
    if reasons.is_empty() {
        Ok(())
    } else {
        Err(ProjectionError::BoundaryEligibilityViolation {
            callable_id: callable_id.clone(),
            reasons,
        })
    }
}

fn validate_effects(
    contract: &BoundaryOperationContract,
    summary: &CallableEffectSummary,
    provenance: &CallableProvenanceSummary,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    let CallableEffectSummary::Analyzed { effects } = summary else {
        push_reason(reasons, BoundaryUnavailableReason::AnalysisPending);
        push_reason(reasons, BoundaryUnavailableReason::UnknownEffect);
        return;
    };
    effect_reasons(contract, *effects, provenance, reasons);
    if effects.may_suspend != contract.may_suspend {
        push_reason(reasons, BoundaryUnavailableReason::UnknownEffect);
    }
}

fn effect_reasons(
    contract: &BoundaryOperationContract,
    effects: CallableMayEffects,
    provenance: &CallableProvenanceSummary,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    let guarantee = &contract.effect_guarantee;
    if effects.writes_caller_reachable && guarantee.no_caller_reachable_mutation {
        push_reason(reasons, BoundaryUnavailableReason::WritesCallerReachable);
    }
    if effects.returns_caller_alias
        && guarantee.detached_return
        && !detached_wrapped_return_is_materialized(
            provenance,
            effects,
            &contract.return_value.value_plan,
        )
    {
        push_reason(reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
    }
    if effects.throws_caller_alias && guarantee.detached_error {
        push_reason(reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
    }
    let detached_parameters = canonical_detached_parameters(contract);
    if effects.escapes_caller_value
        && guarantee.no_caller_value_escape
        && !(detached_parameters && has_only_database_escape(provenance))
        && !matches!(
            provenance,
            CallableProvenanceSummary::Analyzed { escape_lanes, .. } if !escape_lanes.is_empty()
        )
    {
        push_reason(reasons, BoundaryUnavailableReason::UnknownEffect);
    }
    if effects.requires_same_heap_identity
        && guarantee.no_same_heap_identity
        && !(detached_parameters && has_only_database_escape(provenance))
    {
        push_reason(reasons, BoundaryUnavailableReason::RequiresSameHeapIdentity);
    }
    if effects.invokes_unknown_target {
        push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
    }
}

fn validate_provenance(
    contract: &BoundaryOperationContract,
    effects: &CallableEffectSummary,
    provenance: &CallableProvenanceSummary,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    let guarantee = &contract.effect_guarantee;
    match provenance {
        CallableProvenanceSummary::Unknown { reason } => match reason {
            skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending => {
                push_reason(reasons, BoundaryUnavailableReason::AnalysisPending);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnsupportedControlFlow => {
                push_reason(reasons, BoundaryUnavailableReason::UnknownEffect);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnsupportedHeapStore => {
                push_reason(reasons, BoundaryUnavailableReason::UnknownEffect);
            }
            skiff_artifact_model::CallableProvenanceUnknownReason::UnknownCallTarget => {
                push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
            }
        },
        CallableProvenanceSummary::Analyzed {
            return_origins,
            direct_return_origins,
            throw_origins,
            escape_lanes,
        } => {
            if guarantee.detached_return
                && return_origins.iter().any(is_caller_parameter_origin)
                && (matches!(
                    effects,
                    CallableEffectSummary::Analyzed {
                        effects: CallableMayEffects {
                            returns_caller_alias: true,
                            ..
                        }
                    } | CallableEffectSummary::Unknown { .. }
                ) || !return_origins
                    .iter()
                    .any(|origin| matches!(origin, ValueProvenance::Fresh)))
                && !detached_wrapped_return_origins(
                    return_origins,
                    direct_return_origins,
                    escape_lanes,
                )
            {
                push_reason(reasons, BoundaryUnavailableReason::ReturnsCallerAlias);
            }
            if guarantee.detached_error && throw_origins.iter().any(is_caller_parameter_origin) {
                push_reason(reasons, BoundaryUnavailableReason::ThrowsCallerAlias);
            }
            if guarantee.no_caller_value_escape {
                for lane in escape_lanes {
                    if matches!(lane, ValueEscapeLane::Database)
                        && canonical_detached_parameters(contract)
                    {
                        continue;
                    }
                    push_reason(
                        reasons,
                        BoundaryUnavailableReason::EscapesCallerValue { lane: *lane },
                    );
                }
            }
        }
    }
}

fn canonical_detached_parameters(contract: &BoundaryOperationContract) -> bool {
    contract.parameters.iter().all(|parameter| {
        matches!(
            parameter.value_plan,
            BoundaryValuePlan::Linkable {
                carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
                encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
                ..
            }
        )
    })
}

fn has_only_database_escape(provenance: &CallableProvenanceSummary) -> bool {
    matches!(
        provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if !escape_lanes.is_empty()
                && escape_lanes
                .iter()
                .all(|lane| matches!(lane, ValueEscapeLane::Database))
    )
}

fn detached_wrapped_return_is_materialized(
    provenance: &CallableProvenanceSummary,
    effects: CallableMayEffects,
    return_plan: &BoundaryValuePlan,
) -> bool {
    matches!(
        return_plan,
        BoundaryValuePlan::Linkable {
            carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            ..
        }
    ) && !effects.invokes_unknown_target
        && has_no_unmaterialized_escape(provenance)
        && matches!(
            provenance,
            CallableProvenanceSummary::Analyzed {
                return_origins,
                direct_return_origins,
                escape_lanes,
                ..
            } if detached_wrapped_return_origins(
                return_origins,
                direct_return_origins,
                escape_lanes,
            )
        )
}

fn has_no_unmaterialized_escape(provenance: &CallableProvenanceSummary) -> bool {
    matches!(
        provenance,
        CallableProvenanceSummary::Analyzed { escape_lanes, .. }
            if escape_lanes
                .iter()
                .all(|lane| matches!(lane, ValueEscapeLane::Database))
    )
}

fn detached_wrapped_return_origins(
    return_origins: &[ValueProvenance],
    direct_return_origins: &[ValueProvenance],
    escape_lanes: &[ValueEscapeLane],
) -> bool {
    escape_lanes
        .iter()
        .all(|lane| matches!(lane, ValueEscapeLane::Database))
        && return_origins
            .iter()
            .any(|origin| matches!(origin, ValueProvenance::Fresh))
        && return_origins.iter().any(is_caller_parameter_origin)
        && direct_return_origins
            .iter()
            .any(|origin| matches!(origin, ValueProvenance::Fresh))
        && direct_return_origins
            .iter()
            .all(|origin| matches!(origin, ValueProvenance::Fresh | ValueProvenance::Constant))
}

fn is_caller_parameter_origin(origin: &ValueProvenance) -> bool {
    matches!(
        origin,
        ValueProvenance::CallerParameter { .. } | ValueProvenance::CallerParameterProjection { .. }
    )
}

fn validate_call_targets(
    facts: &CallableSemanticFacts,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    for target in facts.resolved_call_targets.values() {
        let unknown = match target {
            CallableTargetFact::Unknown => true,
            CallableTargetFact::PackageDirect {
                package_callable_id,
            } => package_callable_id.is_empty(),
            CallableTargetFact::ContractOperation { operation_id } => {
                operation_id.as_str().is_empty()
            }
        };
        if unknown {
            push_reason(reasons, BoundaryUnavailableReason::UnknownCallTarget);
        }
    }
}

fn validate_contract_features(
    contract: &BoundaryOperationContract,
    reasons: &mut Vec<BoundaryUnavailableReason>,
) {
    for parameter in &contract.parameters {
        validate_value_plan(&parameter.value_plan, reasons);
    }
    validate_value_plan(&contract.return_value.value_plan, reasons);
    match &contract.errors {
        BoundaryErrorContract::None => {}
        BoundaryErrorContract::Typed { value_plan, .. } => validate_value_plan(value_plan, reasons),
        BoundaryErrorContract::Unsupported { .. } => {
            push_reason(reasons, BoundaryUnavailableReason::UnsupportedBoundaryType);
        }
    }
    match &contract.stream {
        BoundaryStreamContract::Unary => {}
        BoundaryStreamContract::ServerStream {
            item_value_plan, ..
        } => validate_value_plan(item_value_plan, reasons),
        BoundaryStreamContract::Unsupported { .. } => {
            push_reason(reasons, BoundaryUnavailableReason::UnsupportedStream);
        }
    }
    if matches!(
        contract.cancellation,
        BoundaryCancellationContract::Unsupported { .. }
    ) {
        push_reason(reasons, BoundaryUnavailableReason::UnsupportedBoundaryType);
    }
    if matches!(
        contract.callbacks,
        BoundaryCallbackContract::Unsupported { .. }
    ) {
        push_reason(
            reasons,
            BoundaryUnavailableReason::CallbackAdapterUnavailable,
        );
    }
}

fn validate_value_plan(plan: &BoundaryValuePlan, reasons: &mut Vec<BoundaryUnavailableReason>) {
    let BoundaryValuePlan::Unsupported { reason } = plan else {
        return;
    };
    let reason = match reason {
        BoundaryValuePlanUnavailableReason::NativeAdapterRequired => {
            BoundaryUnavailableReason::NativeAdapterUnavailable
        }
        BoundaryValuePlanUnavailableReason::CallbackAdapterRequired => {
            BoundaryUnavailableReason::CallbackAdapterUnavailable
        }
        BoundaryValuePlanUnavailableReason::LanguageUnsupported
        | BoundaryValuePlanUnavailableReason::UnknownType => {
            BoundaryUnavailableReason::UnsupportedBoundaryType
        }
    };
    push_reason(reasons, reason);
}

fn push_reason(reasons: &mut Vec<BoundaryUnavailableReason>, reason: BoundaryUnavailableReason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

fn normalize_reasons(reasons: &mut Vec<BoundaryUnavailableReason>) {
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

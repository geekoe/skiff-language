use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryUnavailableReason, CallableEffectSummary,
    CallableMayEffects, CallableProvenanceSummary, CallableProvenanceUnknownReason,
    CallableSemanticFacts, ExecutableExport, ExecutableSignatureIr, FileIrRef,
    OperationCallableKind, PackageCallableId, PackageCallableLinkFact, PackageCallableSignature,
    PackageLocalAbiSymbol, PackageTypeRef, PendingEffectCategory, TypeRefIr,
};

use super::{callable, LocalCallCandidateCorruption};

const TARGET_PUBLIC_ALIAS: &str = "pkg-callable:example.local-authority:fixture.target";

pub(super) struct PublicCallableAuthority {
    pub(super) callable_id: PackageCallableId,
    pub(super) symbol: PackageLocalAbiSymbol,
    pub(super) callable_link: PackageCallableLinkFact,
    pub(super) implementation_export: ExecutableExport,
    pub(super) semantic_facts: CallableSemanticFacts,
    pub(super) boundary_projection: BoundaryCallableProjection,
}

pub(super) fn target_summary(corruption: LocalCallCandidateCorruption) -> CallableEffectSummary {
    match corruption {
        LocalCallCandidateCorruption::TargetAnalyzedNoPending => analyzed_no_effects(),
        LocalCallCandidateCorruption::TargetAnalyzedMayPendingMismatch => {
            analyzed_may_pending_mismatch()
        }
        LocalCallCandidateCorruption::TargetAnalyzedDuplicateCategory => {
            analyzed_duplicate_pending_category()
        }
        LocalCallCandidateCorruption::TargetAnalyzedAbiMaySuspendMismatch => analyzed_pending(),
        _ => canonical_summary(),
    }
}

pub(super) fn target_alias_authority(
    corruption: LocalCallCandidateCorruption,
    canonical: &CallableEffectSummary,
    file: &FileIrRef,
    executable_index: u32,
) -> PublicCallableAuthority {
    let callable_id = callable(TARGET_PUBLIC_ALIAS);
    let effects = if corruption == LocalCallCandidateCorruption::TargetAliasSemanticSummaryDrift {
        analyzed_no_effects()
    } else {
        canonical.clone()
    };
    let signature = callable_signature(
        corruption == LocalCallCandidateCorruption::TargetAbiAliasMaySuspendDrift,
    );
    let symbol = PackageLocalAbiSymbol::Callable {
        callable_id: callable_id.clone(),
        signature: signature.clone(),
    };
    let implementation_export = ExecutableExport {
        file: file.clone(),
        executable_index,
        symbol: "fixture.target".to_string(),
        signature: ExecutableSignatureIr {
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            may_suspend: signature.may_suspend,
        },
    };
    let callable_link = PackageCallableLinkFact {
        callable_id: callable_id.clone(),
        target: implementation_export
            .operation_target_ref(callable_id.as_str(), OperationCallableKind::PublicFunction),
    };
    let semantic_facts = semantic_facts(effects);
    let boundary_projection = canonical_public_boundary_projection(&semantic_facts);
    PublicCallableAuthority {
        callable_id,
        symbol,
        callable_link,
        implementation_export,
        semantic_facts,
        boundary_projection,
    }
}

pub(super) fn callable_symbol(callable_id: PackageCallableId) -> PackageLocalAbiSymbol {
    callable_symbol_with_may_suspend(callable_id, false)
}

pub(super) fn semantic_facts(effects: CallableEffectSummary) -> CallableSemanticFacts {
    CallableSemanticFacts {
        effects,
        provenance: CallableProvenanceSummary::Unknown {
            reason: CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    }
}

fn canonical_public_boundary_projection(
    facts: &CallableSemanticFacts,
) -> BoundaryCallableProjection {
    let mut reasons = vec![BoundaryUnavailableReason::AnalysisPending];
    match &facts.effects {
        CallableEffectSummary::Unknown { .. } => {
            reasons.push(BoundaryUnavailableReason::UnknownEffect);
        }
        CallableEffectSummary::Analyzed { effects } => {
            if effects.escapes_caller_value {
                reasons.push(BoundaryUnavailableReason::UnknownEffect);
            }
            if effects.invokes_unknown_target {
                reasons.push(BoundaryUnavailableReason::UnknownCallTarget);
            }
            if effects.requires_same_heap_identity {
                reasons.push(BoundaryUnavailableReason::RequiresSameHeapIdentity);
            }
        }
    }
    BoundaryCallableProjection::Unavailable { reasons }
}

pub(super) fn canonical_summary() -> CallableEffectSummary {
    CallableEffectSummary::analysis_pending()
}

pub(super) fn analyzed_no_effects() -> CallableEffectSummary {
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        },
    }
}

fn callable_symbol_with_may_suspend(
    callable_id: PackageCallableId,
    may_suspend: bool,
) -> PackageLocalAbiSymbol {
    PackageLocalAbiSymbol::Callable {
        callable_id,
        signature: callable_signature(may_suspend),
    }
}

fn callable_signature(may_suspend: bool) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: Vec::new(),
        parameters: Vec::new(),
        return_type: PackageTypeRef::Local {
            local_type: TypeRefIr::builtin("void"),
        },
        may_suspend,
    }
}

fn analyzed_may_pending_mismatch() -> CallableEffectSummary {
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        },
    }
}

fn analyzed_pending() -> CallableEffectSummary {
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![PendingEffectCategory::Unknown],
            inout_path_effects: Vec::new(),
        },
    }
}

fn analyzed_duplicate_pending_category() -> CallableEffectSummary {
    CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: true,
            pending_effect_categories: vec![
                PendingEffectCategory::Unknown,
                PendingEffectCategory::Unknown,
            ],
            inout_path_effects: Vec::new(),
        },
    }
}

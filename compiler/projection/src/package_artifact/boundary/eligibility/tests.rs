use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryEffectGuarantee, BoundaryParameter, BoundaryReturn,
    BoundaryStreamContract, BoundaryValueLifetime, BoundaryValueOwner, ContractTypeRef,
    PendingEffectCategory, ValueEscapeLane, ValueProvenance,
};

use super::*;

#[test]
fn identity_observation_survives_detachment_and_database_materialization() {
    let detached_plan = |owner| BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    };
    let contract = BoundaryOperationContract {
        parameters: vec![BoundaryParameter {
            name: "value".to_string(),
            ty: ContractTypeRef::builtin("Json"),
            value_plan: detached_plan(BoundaryValueOwner::Caller),
        }],
        return_value: BoundaryReturn {
            ty: ContractTypeRef::builtin("Json"),
            value_plan: detached_plan(BoundaryValueOwner::Provider),
        },
        stream: BoundaryStreamContract::Unary,
        callbacks: BoundaryCallbackContract::None,
        effect_guarantee: BoundaryEffectGuarantee {
            detached_parameters: true,
            detached_return: true,
            detached_error: true,
            no_caller_reachable_mutation: true,
            no_caller_value_escape: true,
            no_same_heap_identity: false,
        },
    };
    let facts = CallableSemanticFacts {
        effects: CallableEffectSummary::Analyzed {
            effects: CallableMayEffects {
                escapes_caller_value: true,
                requires_same_heap_identity: true,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::Unknown],
                inout_path_effects: Vec::new(),
            },
        },
        provenance: CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: vec![ValueEscapeLane::Database],
        },
        resolved_call_targets: BTreeMap::new(),
    };

    assert_eq!(
        semantic_unavailable_reasons(&facts, Some(&contract)),
        vec![BoundaryUnavailableReason::RequiresSameHeapIdentity]
    );
}

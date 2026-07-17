use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, BoundaryErrorContract, BoundaryUnavailableReason,
    CallableEffectSummary, CallableMayEffects, CallableProvenanceSummary, CallableSemanticFacts,
    CallableTargetFact, ContractTypeId, ContractTypeRef, PackageCallableSignature,
    PackageRuntimeRequirements, PackageTypeRef, TypeRefIr, ValueEscapeLane, ValueProvenance,
};

use super::fixtures::{runtime_requirements, safe_facts, signature};
use crate::package_artifact::project_boundary_callable;

#[test]
fn safe_detached_callable_is_available_with_explicit_descriptor_and_requirements() {
    let parameter_type = ContractTypeId::new("contract-type:request");
    let error_type = ContractTypeId::new("contract-type:error");
    let mut signature = signature(TypeRefIr::native("string"));
    signature.parameters[0].ty = PackageTypeRef::Contract {
        contract_type_id: parameter_type.clone(),
    };
    signature.throw_types = vec![PackageTypeRef::Contract {
        contract_type_id: error_type.clone(),
    }];
    let runtime = runtime_requirements("async");
    let projection = project_boundary_callable(
        &"callable:run".into(),
        "run",
        "api",
        &signature,
        &safe_facts(),
        &runtime,
        &[],
    )
    .unwrap();

    let BoundaryCallableProjection::Available {
        descriptor,
        implementation_requirements,
    } = projection
    else {
        panic!("safe detached callable must be Available");
    };
    assert_eq!(descriptor.stable_key, "run");
    assert_eq!(descriptor.contract.parameters.len(), 1);
    assert_eq!(
        descriptor.contract.parameters[0].ty,
        ContractTypeRef::contract(parameter_type)
    );
    assert!(matches!(
        &descriptor.contract.errors,
        BoundaryErrorContract::Typed {
            payload_type: ContractTypeRef::Contract { contract_type_id },
            ..
        } if contract_type_id == &error_type
    ));
    assert!(descriptor.contract.effect_guarantee.detached_parameters);
    assert_eq!(implementation_requirements.config[0].path, "app.token");
    assert_eq!(implementation_requirements.state[0].key, "database");
    assert_eq!(
        implementation_requirements.runtime_capabilities,
        vec!["async"]
    );
}

#[test]
fn every_frozen_unavailable_reason_is_projected_fail_closed() {
    let unknown = CallableSemanticFacts {
        effects: CallableEffectSummary::analysis_pending(),
        provenance: CallableProvenanceSummary::Unknown {
            reason: skiff_artifact_model::CallableProvenanceUnknownReason::AnalysisPending,
        },
        resolved_call_targets: BTreeMap::new(),
    };
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unknown,
        &[
            BoundaryUnavailableReason::AnalysisPending,
            BoundaryUnavailableReason::UnknownEffect,
        ],
    );

    let mut unsafe_facts = safe_facts();
    unsafe_facts.effects = CallableEffectSummary::Analyzed {
        effects: CallableMayEffects {
            writes_caller_reachable: true,
            returns_caller_alias: true,
            throws_caller_alias: true,
            escapes_caller_value: true,
            requires_same_heap_identity: true,
            invokes_unknown_target: true,
            may_suspend: false,
        },
    };
    unsafe_facts.provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
        throw_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
        escape_lanes: vec![ValueEscapeLane::Capture],
    };
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unsafe_facts,
        &[
            BoundaryUnavailableReason::UnknownCallTarget,
            BoundaryUnavailableReason::WritesCallerReachable,
            BoundaryUnavailableReason::ReturnsCallerAlias,
            BoundaryUnavailableReason::ThrowsCallerAlias,
            BoundaryUnavailableReason::EscapesCallerValue {
                lane: ValueEscapeLane::Capture,
            },
            BoundaryUnavailableReason::RequiresSameHeapIdentity,
        ],
    );

    let mut unknown_target = safe_facts();
    unknown_target
        .resolved_call_targets
        .insert(0, CallableTargetFact::Unknown);
    assert_reasons(
        signature(TypeRefIr::native("string")),
        unknown_target,
        &[BoundaryUnavailableReason::UnknownCallTarget],
    );
    assert_reasons(
        signature(TypeRefIr::Function {
            params: Vec::new(),
            return_type: Box::new(TypeRefIr::native("void")),
        }),
        safe_facts(),
        &[BoundaryUnavailableReason::CallbackAdapterUnavailable],
    );
    assert_reasons(
        signature(TypeRefIr::native("Socket")),
        safe_facts(),
        &[BoundaryUnavailableReason::NativeAdapterUnavailable],
    );
    assert_reasons(
        signature(TypeRefIr::LocalType { type_index: 0 }),
        safe_facts(),
        &[BoundaryUnavailableReason::UnsupportedBoundaryType],
    );
    assert_reasons(
        signature(TypeRefIr::Native {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::native("string")],
        }),
        safe_facts(),
        &[BoundaryUnavailableReason::UnsupportedStream],
    );
}

fn assert_reasons(
    signature: PackageCallableSignature,
    facts: CallableSemanticFacts,
    expected: &[BoundaryUnavailableReason],
) {
    let projection = project_boundary_callable(
        &"callable:test".into(),
        "test",
        "api",
        &signature,
        &facts,
        &PackageRuntimeRequirements {
            config: Vec::new(),
            resources: Vec::new(),
            runtime_capabilities: Vec::new(),
        },
        &[],
    )
    .unwrap();
    let BoundaryCallableProjection::Unavailable { reasons } = projection else {
        panic!("unsafe callable must be Unavailable");
    };
    for reason in expected {
        assert!(
            reasons.contains(reason),
            "missing {reason:?} in {reasons:?}"
        );
    }
}

use skiff_artifact_identity::assign_service_contract_identities;
use skiff_artifact_model::*;

use super::*;

impl ProjectionFixture {
    fn synchronize_effects(&mut self, effects: CallableMayEffects) {
        self.implementation
            .callable_semantic_facts
            .get_mut(&self.callable_id)
            .unwrap()
            .effects = CallableEffectSummary::Analyzed { effects };
        let BoundaryCallableProjection::Available {
            implementation_requirements,
            ..
        } = self
            .implementation
            .boundary_projections
            .get_mut(&self.callable_id)
            .unwrap()
        else {
            unreachable!()
        };
        implementation_requirements.complete_may_effects = effects;
    }

    fn synchronize_provenance(&mut self, provenance: CallableProvenanceSummary) {
        self.implementation
            .callable_semantic_facts
            .get_mut(&self.callable_id)
            .unwrap()
            .provenance = provenance.clone();
        let BoundaryCallableProjection::Available {
            implementation_requirements,
            ..
        } = self
            .implementation
            .boundary_projections
            .get_mut(&self.callable_id)
            .unwrap()
        else {
            unreachable!()
        };
        implementation_requirements.provenance = provenance;
    }
}

#[test]
fn synchronized_unsafe_effect_mutations_cannot_forge_available() {
    let cases: &[(fn(&mut CallableMayEffects), BoundaryUnavailableReason)] = &[
        (
            |effects| effects.writes_caller_reachable = true,
            BoundaryUnavailableReason::WritesCallerReachable,
        ),
        (
            |effects| effects.returns_caller_alias = true,
            BoundaryUnavailableReason::ReturnsCallerAlias,
        ),
        (
            |effects| effects.throws_caller_alias = true,
            BoundaryUnavailableReason::ThrowsCallerAlias,
        ),
        (
            |effects| effects.requires_same_heap_identity = true,
            BoundaryUnavailableReason::RequiresSameHeapIdentity,
        ),
        (
            |effects| effects.invokes_unknown_target = true,
            BoundaryUnavailableReason::UnknownCallTarget,
        ),
    ];
    for (mutate, expected) in cases {
        let mut fixture = ProjectionFixture::new();
        let mut effects = no_effects();
        mutate(&mut effects);
        fixture.synchronize_effects(effects);
        fixture.refresh_implementation_ref();
        assert_eligibility_reason(&fixture, expected.clone());
    }

    let mut fixture = ProjectionFixture::new();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::Fresh],
        throw_origins: Vec::new(),
        escape_lanes: vec![ValueEscapeLane::Capture],
    };
    let mut effects = no_effects();
    effects.escapes_caller_value = true;
    fixture.synchronize_effects(effects);
    fixture.synchronize_provenance(provenance);
    fixture.refresh_implementation_ref();
    assert_eligibility_reason(
        &fixture,
        BoundaryUnavailableReason::EscapesCallerValue {
            lane: ValueEscapeLane::Capture,
        },
    );

    for (provenance, expected) in [
        (
            CallableProvenanceSummary::Analyzed {
                return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
                throw_origins: Vec::new(),
                escape_lanes: Vec::new(),
            },
            BoundaryUnavailableReason::ReturnsCallerAlias,
        ),
        (
            CallableProvenanceSummary::Analyzed {
                return_origins: Vec::new(),
                throw_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
                escape_lanes: Vec::new(),
            },
            BoundaryUnavailableReason::ThrowsCallerAlias,
        ),
    ] {
        let mut fixture = ProjectionFixture::new();
        fixture.synchronize_provenance(provenance);
        fixture.refresh_implementation_ref();
        assert_eligibility_reason(&fixture, expected);
    }
}

#[test]
fn canonical_return_materializes_only_a_fresh_wrapper_around_a_caller_value() {
    let mut fixture = ProjectionFixture::new();
    let mut effects = no_effects();
    effects.returns_caller_alias = true;
    fixture.synchronize_effects(effects);
    fixture.synchronize_provenance(CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    });
    fixture.refresh_implementation_ref();
    fixture
        .project()
        .expect("canonical encoding detaches the fresh return wrapper");

    let mut escaping = ProjectionFixture::new();
    let mut effects = no_effects();
    effects.returns_caller_alias = true;
    effects.escapes_caller_value = true;
    escaping.synchronize_effects(effects);
    escaping.synchronize_provenance(CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        throw_origins: Vec::new(),
        escape_lanes: vec![ValueEscapeLane::Capture],
    });
    escaping.refresh_implementation_ref();
    assert_eligibility_reason(
        &escaping,
        BoundaryUnavailableReason::EscapesCallerValue {
            lane: ValueEscapeLane::Capture,
        },
    );
}

#[test]
fn unknown_typed_facts_and_targets_cannot_forge_available() {
    let mut fixture = ProjectionFixture::new();
    fixture.synchronize_provenance(CallableProvenanceSummary::Unknown {
        reason: CallableProvenanceUnknownReason::UnsupportedControlFlow,
    });
    fixture.refresh_implementation_ref();
    assert_eligibility_reason(&fixture, BoundaryUnavailableReason::UnknownEffect);

    let mut fixture = ProjectionFixture::new();
    fixture
        .implementation
        .callable_semantic_facts
        .get_mut(&fixture.callable_id)
        .unwrap()
        .resolved_call_targets
        .insert(0, CallableTargetFact::Unknown);
    fixture.refresh_implementation_ref();
    assert_eligibility_reason(&fixture, BoundaryUnavailableReason::UnknownCallTarget);

    let mut fixture = ProjectionFixture::new();
    fixture
        .implementation
        .callable_semantic_facts
        .get_mut(&fixture.callable_id)
        .unwrap()
        .effects = CallableEffectSummary::analysis_pending();
    fixture.refresh_implementation_ref();
    assert_eligibility_reasons(
        &fixture,
        &[
            BoundaryUnavailableReason::AnalysisPending,
            BoundaryUnavailableReason::UnknownEffect,
        ],
    );
}

#[test]
fn unsupported_callback_and_native_claims_cannot_forge_available() {
    let mut callback = ProjectionFixture::new();
    for descriptor in callback.contract.operations.values_mut() {
        descriptor.contract.callbacks = BoundaryCallbackContract::Unsupported {
            reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
        };
    }
    assign_service_contract_identities(&mut callback.contract).unwrap();
    callback.input.contract = contract_ref(&callback.contract);
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = callback
        .implementation
        .boundary_projections
        .get_mut(&callback.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    operation_contract.callbacks = BoundaryCallbackContract::Unsupported {
        reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
    };
    callback.refresh_implementation_ref();
    assert_eligibility_reason(
        &callback,
        BoundaryUnavailableReason::CallbackAdapterUnavailable,
    );

    let mut native = ProjectionFixture::new();
    let BoundaryCallableProjection::Available {
        implementation_requirements,
        ..
    } = native
        .implementation
        .boundary_projections
        .get_mut(&native.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    implementation_requirements
        .native_capabilities
        .push("native.socket".to_string());
    native.input.resource_bindings.push(ResourceBinding {
        requirement_key: "native-socket".to_string(),
        capability: "native.socket".to_string(),
        resource_ref: "resource:native-socket".to_string(),
    });
    native.refresh_implementation_ref();
    assert_eligibility_reason(&native, BoundaryUnavailableReason::NativeAdapterUnavailable);
}

fn assert_eligibility_reason(fixture: &ProjectionFixture, expected: BoundaryUnavailableReason) {
    let error = fixture.project().unwrap_err();
    let ProjectionError::BoundaryEligibilityViolation { reasons, .. } = error else {
        panic!("expected boundary eligibility violation, got {error}");
    };
    assert!(
        reasons.contains(&expected),
        "expected {expected:?}, got {reasons:?}"
    );
}

fn assert_eligibility_reasons(fixture: &ProjectionFixture, expected: &[BoundaryUnavailableReason]) {
    let error = fixture.project().unwrap_err();
    let ProjectionError::BoundaryEligibilityViolation { reasons, .. } = error else {
        panic!("expected boundary eligibility violation, got {error}");
    };
    assert_eq!(reasons, expected);
}

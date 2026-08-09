use skiff_artifact_model::*;

use super::*;

impl ProjectionFixture {
    fn synchronize_effects(&mut self, effects: CallableMayEffects) {
        self.implementation
            .callable_semantic_facts
            .get_mut(&self.callable_id)
            .unwrap()
            .effects = CallableEffectSummary::Analyzed {
            effects: effects.clone(),
        };
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
        implementation_requirements.complete_may_effects = effects.clone();
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
fn rehashed_forged_plan_passes_identity_but_not_deployment_admission() {
    let mut fixture = ProjectionFixture::new();
    let contract_projection = serde_json::to_value(
        skiff_artifact_identity::service_protocol_identity_projection(&fixture.contract).unwrap(),
    )
    .unwrap();
    let package_projection = serde_json::to_value(
        skiff_artifact_identity::package_artifact_build_identity_projection(
            &fixture.implementation,
        )
        .unwrap(),
    )
    .unwrap();
    for descriptor in fixture.contract.operations.values_mut() {
        let BoundaryValuePlan::Linkable { owner, .. } =
            &mut descriptor.contract.return_value.value_plan
        else {
            unreachable!()
        };
        *owner = BoundaryValueOwner::Caller;
    }
    mechanically_rehash_forged_contract(&mut fixture.contract, contract_projection);
    fixture.input.contract = contract_ref(&fixture.contract);

    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = fixture
        .implementation
        .boundary_projections
        .get_mut(&fixture.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    let BoundaryValuePlan::Linkable { owner, .. } = &mut operation_contract.return_value.value_plan
    else {
        unreachable!()
    };
    *owner = BoundaryValueOwner::Caller;
    mechanically_rehash_forged_package(&mut fixture.implementation, package_projection);
    fixture.input.implementation = package_ref(&fixture.implementation);

    let package_identity_admitted =
        skiff_artifact_identity::validate_package_artifact_identities(&fixture.implementation)
            .is_ok();
    let contract_identity_admitted =
        skiff_artifact_identity::validate_service_contract_identities(&fixture.contract).is_ok();
    match fixture.project().unwrap_err() {
        ProjectionError::InvalidPackageBoundaryProjections { .. } => {
            assert!(package_identity_admitted);
            assert!(contract_identity_admitted);
        }
        ProjectionError::InvalidTypedArtifact { .. } => {
            assert!(!package_identity_admitted || !contract_identity_admitted);
        }
        error => panic!("expected identity or canonical boundary rejection, got {error}"),
    }
}

#[test]
fn synchronized_unsafe_effect_mutations_cannot_forge_available() {
    // The three aggregate alias flags were retired (R-084): ordinary aggregate
    // mutation/returns/throws are logical snapshots and cannot forge an
    // availability claim. The remaining unsafe effect mutations still reject.
    type UnsafeEffectMutation = (fn(&mut CallableMayEffects), BoundaryUnavailableReason);

    let cases: &[UnsafeEffectMutation] = &[
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

    let mut identity_after_database_materialization = ProjectionFixture::new();
    let mut effects = no_effects();
    effects.escapes_caller_value = true;
    effects.requires_same_heap_identity = true;
    identity_after_database_materialization.synchronize_effects(effects);
    identity_after_database_materialization.synchronize_provenance(
        CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::Fresh],
            direct_return_origins: vec![ValueProvenance::Fresh],
            throw_origins: Vec::new(),
            escape_lanes: vec![ValueEscapeLane::Database],
        },
    );
    for descriptor in identity_after_database_materialization
        .contract
        .operations
        .values_mut()
    {
        descriptor.contract.effect_guarantee.no_same_heap_identity = false;
    }
    identity_after_database_materialization.refresh_contract_ref();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = identity_after_database_materialization
        .implementation
        .boundary_projections
        .get_mut(&identity_after_database_materialization.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    operation_contract.effect_guarantee.no_same_heap_identity = false;
    identity_after_database_materialization.refresh_implementation_ref();
    assert_eligibility_reason(
        &identity_after_database_materialization,
        BoundaryUnavailableReason::RequiresSameHeapIdentity,
    );

    let mut fixture = ProjectionFixture::new();
    let provenance = CallableProvenanceSummary::Analyzed {
        return_origins: vec![ValueProvenance::Fresh],
        direct_return_origins: vec![ValueProvenance::Fresh],
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

    // R-084: caller-origin return/throw aggregates are logical snapshots and
    // no longer produce alias unavailability reasons.
    for provenance in [
        CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
            direct_return_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: vec![ValueProvenance::CallerParameter { index: 0 }],
            escape_lanes: Vec::new(),
        },
        CallableProvenanceSummary::Analyzed {
            return_origins: vec![ValueProvenance::CallerParameterProjection {
                index: 0,
                path: ValueProjectionPath::container_element(),
            }],
            direct_return_origins: vec![ValueProvenance::CallerParameterProjection {
                index: 0,
                path: ValueProjectionPath::container_element(),
            }],
            throw_origins: Vec::new(),
            escape_lanes: Vec::new(),
        },
        CallableProvenanceSummary::Analyzed {
            return_origins: Vec::new(),
            direct_return_origins: Vec::new(),
            throw_origins: vec![ValueProvenance::CallerParameterProjection {
                index: 0,
                path: ValueProjectionPath::field("error").unwrap(),
            }],
            escape_lanes: Vec::new(),
        },
    ] {
        let mut fixture = ProjectionFixture::new();
        fixture.synchronize_provenance(provenance);
        fixture.refresh_implementation_ref();
        fixture
            .project()
            .expect("caller-origin return/throw aggregates stay boundary-available (R-084)");
    }
}

#[test]
fn canonical_return_materializes_only_a_fresh_wrapper_around_a_caller_value() {
    let mut fixture = ProjectionFixture::new();
    fixture.synchronize_effects(no_effects());
    fixture.synchronize_provenance(CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        direct_return_origins: vec![ValueProvenance::Fresh],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    });
    fixture.refresh_implementation_ref();
    fixture
        .project()
        .expect("canonical encoding detaches the fresh return wrapper");

    let mut escaping = ProjectionFixture::new();
    let mut effects = no_effects();
    effects.escapes_caller_value = true;
    escaping.synchronize_effects(effects);
    escaping.synchronize_provenance(CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        direct_return_origins: vec![ValueProvenance::Fresh],
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

    // A fresh wrapper whose direct root may be the caller value is still a
    // logical snapshot at the boundary under R-084.
    let mut conditional_root = ProjectionFixture::new();
    conditional_root.synchronize_effects(no_effects());
    conditional_root.synchronize_provenance(CallableProvenanceSummary::Analyzed {
        return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        direct_return_origins: vec![
            ValueProvenance::Fresh,
            ValueProvenance::CallerParameter { index: 0 },
        ],
        throw_origins: Vec::new(),
        escape_lanes: Vec::new(),
    });
    conditional_root.refresh_implementation_ref();
    conditional_root
        .project()
        .expect("conditional caller-root return stays boundary-available (R-084)");
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
fn unsupported_stream_and_native_claims_and_orphan_callback_declaration_cannot_forge_available() {
    let mut stream = ProjectionFixture::new();
    for descriptor in stream.contract.operations.values_mut() {
        descriptor.contract.stream = BoundaryStreamContract::Unsupported {
            reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
        };
    }
    stream.refresh_contract_ref();
    let BoundaryCallableProjection::Available {
        operation_contract, ..
    } = stream
        .implementation
        .boundary_projections
        .get_mut(&stream.callable_id)
        .unwrap()
    else {
        unreachable!()
    };
    operation_contract.stream = BoundaryStreamContract::Unsupported {
        reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
    };
    stream.refresh_implementation_ref();
    assert_eligibility_reason(&stream, BoundaryUnavailableReason::UnsupportedStream);

    let mut callback = ProjectionFixture::new();
    for descriptor in callback.contract.operations.values_mut() {
        descriptor.contract.callbacks = BoundaryCallbackContract::Unsupported {
            reason: BoundaryFeatureUnavailableReason::LanguageUnsupported,
        };
    }
    callback.refresh_contract_ref();
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
    // CallbackCapability is canonical only for an exact, non-generic `any I`
    // value position. This fixture has no callback position, so its exact
    // callback declaration remains `None`; an orphan `Unsupported` claim is a
    // malformed operation contract rather than an availability reason.
    assert_projection_rejection_contains(
        &callback,
        "callback declaration is not canonical for operation value positions; expected=None",
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
    native.refresh_implementation_ref();
    assert_eligibility_reason(&native, BoundaryUnavailableReason::NativeAdapterUnavailable);
}

fn assert_projection_rejection_contains(fixture: &ProjectionFixture, expected: &str) {
    let error = fixture.project().unwrap_err();
    let message = match error {
        ProjectionError::InvalidPackageBoundaryProjections { source, .. } => source.to_string(),
        ProjectionError::InvalidTypedArtifact { identity_error, .. } => identity_error.to_string(),
        error => panic!("expected canonical boundary admission rejection, got {error}"),
    };
    assert!(
        message.contains(expected),
        "expected {expected:?}, got {message}"
    );
}

fn assert_eligibility_reason(fixture: &ProjectionFixture, expected: BoundaryUnavailableReason) {
    let error = fixture.project().unwrap_err();
    let message = match error {
        ProjectionError::InvalidPackageBoundaryProjections { source, .. } => source.to_string(),
        ProjectionError::InvalidTypedArtifact { identity_error, .. } => identity_error.to_string(),
        error => panic!("expected canonical boundary admission rejection, got {error}"),
    };
    assert!(
        message.contains(&format!("{expected:?}"))
            || (expected == BoundaryUnavailableReason::UnsupportedStream
                && message.contains("unsupported stream"))
            || (expected == BoundaryUnavailableReason::NativeAdapterUnavailable
                && message.contains("native_capabilities: [\"native.socket\"]")),
        "expected {expected:?}, got {message}"
    );
}

fn assert_eligibility_reasons(fixture: &ProjectionFixture, expected: &[BoundaryUnavailableReason]) {
    let error = fixture.project().unwrap_err();
    let message = match error {
        ProjectionError::InvalidPackageBoundaryProjections { source, .. } => source.to_string(),
        ProjectionError::InvalidTypedArtifact { identity_error, .. } => identity_error.to_string(),
        error => panic!("expected canonical boundary admission rejection, got {error}"),
    };
    for reason in expected {
        assert!(
            message.contains(&format!("{reason:?}")),
            "expected {reason:?}, got {message}"
        );
    }
}

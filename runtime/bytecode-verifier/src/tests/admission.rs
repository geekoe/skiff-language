use skiff_artifact_model::{
    current_platform_error_projection_registry_ref,
    validate_platform_error_projection_registry_ref_shape, CallableEffectSummary, TypeRefIr,
    BYTECODE_SCHEMA_VERSION,
};
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, CandidateTable, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate,
    LinkedBytecodeCandidateParts, LinkedTypeEntry, TypeIndex,
};

use crate::{
    admission::prove_admission, verify, VerificationError, VerificationLimit, VerificationLocation,
    VerificationObligation,
};

use super::fixtures::{
    candidate_for, candidate_for_concrete_types, candidate_for_with_authority_corruption,
    exact_hydration, exact_hydration_with_types, generous_limits, loader_backed_local_call,
    AuthorityPinCorruption, LocalCallCandidateCorruption,
};

#[test]
fn exact_empty_admission_completes_the_vacuous_effect_proof() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let image = verify(hydrated, candidate, &generous_limits())
        .expect("empty image has a real vacuous effect certificate");
    assert_eq!(image.functions().len(), 0);
}

#[test]
fn current_v7_platform_error_registry_getter_matrix_passes_exact_admission() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let current = current_platform_error_projection_registry_ref();
    assert_eq!(BYTECODE_SCHEMA_VERSION, "skiff-bytecode-v7");
    {
        let candidate_package = &candidate.packages()[0];
        let hydrated_package = hydrated.packages().values().next().unwrap();
        assert_eq!(candidate_package.schema_version(), BYTECODE_SCHEMA_VERSION);
        assert_eq!(
            candidate_package
                .authorities()
                .platform_error_projection_registry(),
            current
        );
        assert_eq!(hydrated.platform_error_projection_registry(), current);
        assert_eq!(
            hydrated_package.platform_error_projection_registry(),
            current
        );
        assert_eq!(
            &hydrated_package
                .artifact()
                .platform_error_projection_registry,
            current
        );
        assert_eq!(
            &hydrated_package
                .bytecode()
                .artifact()
                .platform_error_projection_registry,
            current
        );
        assert_eq!(
            hydrated_package
                .bytecode()
                .view()
                .platform_error_projection_registry(),
            current
        );
    }

    verify(hydrated, candidate, &generous_limits())
        .expect("the current v7 registry authority matrix must verify exactly");
}

#[test]
fn exact_effect_binding_is_dense_and_keeps_unknown_separate_from_abi_false() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let admission = prove_admission(&hydrated, &candidate, &generous_limits())
        .expect("exact hydration must produce P1 effect authority");
    let effects = admission.effect_binding();
    assert_eq!(effects.functions().len(), candidate.functions().len());

    let target = effects
        .function(super::fixtures::TARGET_FUNCTION_INDEX)
        .expect("dense target effect binding");
    assert_eq!(
        target.canonical_callable().as_str(),
        "pkg-callable:example.local-authority:top-level:fixture.target"
    );
    assert!(matches!(
        target.summary(),
        CallableEffectSummary::Unknown { .. }
    ));
    let declarations = target.local_abi_declarations();
    assert_eq!(declarations.len(), 2);
    assert!(declarations
        .iter()
        .all(|declaration| !declaration.may_suspend()));
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.callable().as_str())
            .collect::<Vec<_>>(),
        vec![
            "pkg-callable:example.local-authority:fixture.target",
            "pkg-callable:example.local-authority:top-level:fixture.target",
        ]
    );
}

#[test]
fn analyzed_target_does_not_upgrade_unknown_caller_effects() {
    let (hydrated, candidate) =
        loader_backed_local_call(LocalCallCandidateCorruption::TargetAnalyzedNoPending);
    let admission = prove_admission(&hydrated, &candidate, &generous_limits())
        .expect("consistent analyzed authority must bind exactly");
    let target = admission
        .effect_binding()
        .function(super::fixtures::TARGET_FUNCTION_INDEX)
        .expect("dense target effect binding");
    assert!(matches!(
        target.summary(),
        CallableEffectSummary::Analyzed { effects }
            if !effects.may_pending && effects.pending_effect_categories.is_empty()
    ));

    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("unknown caller must remain fail closed");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Function {
                function: skiff_runtime_linked_bytecode::FunctionIndex::new(0),
            },
        }
    );
}

#[test]
fn effect_binding_rejects_alias_and_analyzed_summary_drift() {
    let cases = [
        (
            LocalCallCandidateCorruption::TargetAbiAliasMaySuspendDrift,
            "aliases disagree",
        ),
        (
            LocalCallCandidateCorruption::TargetAnalyzedMayPendingMismatch,
            "mayPending disagrees",
        ),
        (
            LocalCallCandidateCorruption::TargetAnalyzedDuplicateCategory,
            "contain a duplicate",
        ),
    ];
    for (corruption, expected) in cases {
        assert_effect_binding_corruption_is_rejected(corruption, expected);
    }
}

#[test]
fn analyzed_pending_rejects_consistent_false_alias_declarations() {
    assert_effect_binding_corruption_is_rejected(
        LocalCallCandidateCorruption::TargetAnalyzedAbiMaySuspendMismatch,
        "maySuspend disagrees with canonical analyzed",
    );
}

#[test]
fn alias_semantic_summary_drift_is_rejected() {
    assert_effect_binding_corruption_is_rejected(
        LocalCallCandidateCorruption::TargetAliasSemanticSummaryDrift,
        "alias effect summary drifts",
    );
}

fn assert_effect_binding_corruption_is_rejected(
    corruption: LocalCallCandidateCorruption,
    expected: &str,
) {
    let (hydrated, candidate) = loader_backed_local_call(corruption);
    let error = prove_admission(&hydrated, &candidate, &generous_limits())
        .expect_err("non-canonical effect authority must fail in P1");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Function { function },
            detail,
        } if function == super::fixtures::TARGET_FUNCTION_INDEX
            && detail.contains(expected)
    ));
}

#[test]
fn candidate_package_set_omission_is_rejected_before_semantic_proofs() {
    let hydrated = exact_hydration();
    let candidate = LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
        packages: Vec::new(),
        functions: Vec::new(),
        operation_entries: Vec::new(),
        gateway_entries: Vec::new(),
        exact_local_targets: Vec::new(),
        service_operations: Vec::new(),
        actor_creates: Vec::new(),
        actor_methods: Vec::new(),
        interface_tables: Vec::new(),
        synthetic_callbacks: Vec::new(),
        callback_capture_layouts: Vec::new(),
        host_effect_adapters: Vec::new(),
        intrinsics: Vec::new(),
        types: Vec::new(),
        shapes: Vec::new(),
        constants: Vec::new(),
        constant_roots: Vec::new(),
        frozen_constant_nodes: Vec::new(),
        resume_sites: Vec::new(),
        writable_paths: Vec::new(),
    })
    .unwrap();
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Image,
            ..
        }
    ));
}

#[test]
fn corrupt_candidate_schema_pin_is_rejected() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, Some("skiff-bytecode-v999"));
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Table {
                table: CandidateTable::Packages,
                row: 0,
            },
            ..
        }
    ));
}

fn assert_authority_pin_corruption_is_rejected_before_p2(
    corruption: AuthorityPinCorruption,
    authority: &str,
) {
    let hydrated = exact_hydration();
    let candidate = candidate_for_with_authority_corruption(&hydrated, None, Some(corruption));
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    let (obligation, location, detail) = match error {
        VerificationError::SemanticViolation {
            obligation,
            location,
            detail,
        } => (obligation, location, detail),
        other => {
            panic!("corrupt {authority} pin reached P2 instead of failing P1: {other:?}")
        }
    };
    assert_eq!(obligation, VerificationObligation::ExactHydrationBinding);
    assert_eq!(
        location,
        VerificationLocation::Table {
            table: CandidateTable::Packages,
            row: 0,
        }
    );
    assert!(
        detail.contains(authority),
        "P1 rejection did not identify the corrupt {authority} pin: {detail}"
    );
}

#[test]
fn corrupt_native_value_lifecycle_registry_pin_is_rejected_before_p2() {
    assert_authority_pin_corruption_is_rejected_before_p2(
        AuthorityPinCorruption::NativeValueLifecycleRegistry,
        "native value lifecycle registry",
    );
}

#[test]
fn corrupt_value_lifecycle_policy_pin_is_rejected_before_p2() {
    assert_authority_pin_corruption_is_rejected_before_p2(
        AuthorityPinCorruption::ValueLifecyclePolicy,
        "value lifecycle policy",
    );
}

#[test]
fn corrupt_host_effect_registry_pin_is_rejected_before_p2() {
    assert_authority_pin_corruption_is_rejected_before_p2(
        AuthorityPinCorruption::HostEffectRegistry,
        "host effect registry",
    );
}

#[test]
fn corrupt_intrinsic_registry_pin_is_rejected_before_p2() {
    assert_authority_pin_corruption_is_rejected_before_p2(
        AuthorityPinCorruption::IntrinsicRegistry,
        "intrinsic registry",
    );
}

#[test]
fn same_v1_different_valid_registry_fingerprint_is_rejected_before_p2() {
    let hydrated = exact_hydration();
    let candidate = candidate_for_with_authority_corruption(
        &hydrated,
        None,
        Some(AuthorityPinCorruption::PlatformErrorProjectionRegistry),
    );
    let candidate_registry = candidate.packages()[0]
        .authorities()
        .platform_error_projection_registry();
    let current = current_platform_error_projection_registry_ref();
    validate_platform_error_projection_registry_ref_shape(candidate_registry)
        .expect("the historical descriptor must retain a strict valid v1 shape");
    assert_eq!(candidate_registry.registry_id(), current.registry_id());
    assert_eq!(
        candidate_registry.registry_version(),
        current.registry_version()
    );
    assert_ne!(candidate_registry.fingerprint(), current.fingerprint());
    let historical_fingerprint = candidate_registry.fingerprint().to_string();
    let current_fingerprint = current.fingerprint().to_string();

    let error = verify(hydrated, candidate, &generous_limits())
        .expect_err("a historical candidate registry pin must fail before instruction proofs");
    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail,
    } = error
    else {
        panic!("historical registry pin reached a later verifier phase: {error:?}");
    };
    assert_eq!(obligation, VerificationObligation::ExactHydrationBinding);
    assert_eq!(
        location,
        VerificationLocation::Table {
            table: CandidateTable::Packages,
            row: 0,
        }
    );
    assert!(detail.contains("platform error projection registry"));
    assert!(
        !detail.contains(historical_fingerprint.as_str()),
        "historical registry fingerprint leaked into verifier failure detail"
    );
    assert!(
        !detail.contains(current_fingerprint.as_str()),
        "current registry fingerprint leaked into verifier failure detail"
    );
}

#[test]
fn wrong_candidate_registry_pin_is_not_repaired_from_hydration_or_runtime() {
    let hydrated = exact_hydration();
    let candidate = candidate_for_with_authority_corruption(
        &hydrated,
        None,
        Some(AuthorityPinCorruption::PlatformErrorProjectionRegistry),
    );
    let candidate_registry = candidate.packages()[0]
        .authorities()
        .platform_error_projection_registry();
    let current = current_platform_error_projection_registry_ref();
    let hydrated_package = hydrated.packages().values().next().unwrap();
    assert_ne!(candidate_registry, current);
    assert_eq!(hydrated.platform_error_projection_registry(), current);
    assert_eq!(
        hydrated_package.platform_error_projection_registry(),
        current
    );
    assert_eq!(
        &hydrated_package
            .artifact()
            .platform_error_projection_registry,
        current
    );
    assert_eq!(
        &hydrated_package
            .bytecode()
            .artifact()
            .platform_error_projection_registry,
        current
    );
    assert_eq!(
        hydrated_package
            .bytecode()
            .view()
            .platform_error_projection_registry(),
        current
    );

    let error = prove_admission(&hydrated, &candidate, &generous_limits())
        .expect_err("P1 must consume the candidate receipt rather than reconstructing current");
    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Table {
                table: CandidateTable::Packages,
                row: 0,
            },
            detail,
        } if detail.contains("candidate")
            && detail.contains("deployment hydration receipt")
            && detail.contains("package hydration receipt")
    ));
}

#[test]
fn package_budget_is_enforced_before_binding() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let mut limits = generous_limits();
    limits.max_image_table_entries = 0;
    let error = verify(hydrated, candidate, &limits).unwrap_err();

    assert_eq!(
        error,
        VerificationError::LimitExceeded {
            limit: VerificationLimit::ImageTableEntries,
            actual: 1,
            max: 0,
            location: VerificationLocation::Table {
                table: CandidateTable::Packages,
                row: 0,
            },
        }
    );
}

#[test]
fn fixed_image_and_function_ceilings_precede_attribution_row_scans() {
    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let mut limits = generous_limits();
    limits.max_image_table_entries = 1;
    limits.max_functions = 0;
    limits.max_statement_events_per_pc = 0;
    let error = verify(hydrated, candidate, &limits).unwrap_err();
    assert_eq!(
        error,
        VerificationError::LimitExceeded {
            limit: VerificationLimit::ImageTableEntries,
            actual: 2,
            max: 1,
            location: VerificationLocation::Table {
                table: CandidateTable::Functions,
                row: 0,
            },
        }
    );

    let (hydrated, candidate) = loader_backed_local_call(LocalCallCandidateCorruption::None);
    let mut limits = generous_limits();
    limits.max_functions = 1;
    limits.max_statement_events_per_pc = 0;
    let error = verify(hydrated, candidate, &limits).unwrap_err();
    assert_eq!(
        error,
        VerificationError::LimitExceeded {
            limit: VerificationLimit::Functions,
            actual: 2,
            max: 1,
            location: VerificationLocation::Image,
        }
    );
}

#[test]
fn out_of_bounds_type_origin_coordinate_is_rejected_by_p1() {
    let hydrated = exact_hydration_with_types(vec![TypeRefIr::builtin("string")]);
    let build_id = hydrated.packages().keys().next().unwrap().clone();
    let linked = LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(build_id, ArtifactTypeIndex::new(1), None).unwrap(),
        TypeRefIr::builtin("string"),
        None,
    );
    let candidate = candidate_for_concrete_types(&hydrated, vec![linked], Vec::new())
        .expect("candidate-local validation cannot authorize an admitted pool coordinate");
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert!(matches!(
        error,
        VerificationError::SemanticViolation {
            obligation: VerificationObligation::ExactHydrationBinding,
            location: VerificationLocation::Table {
                table: CandidateTable::Types,
                row: 0,
            },
            detail,
        } if detail.contains("exact admitted artifact row")
    ));
}

#[test]
fn statement_instruction_sequence_id_and_site_corruption_fail_in_p1() {
    let cases = [
        (
            LocalCallCandidateCorruption::StatementInstruction,
            "instruction",
        ),
        (
            LocalCallCandidateCorruption::StatementSequence,
            "sequence ordinal",
        ),
        (
            LocalCallCandidateCorruption::StatementAttributionId,
            "attribution id",
        ),
        (LocalCallCandidateCorruption::StatementSite, "source site"),
    ];
    for (corruption, expected) in cases {
        let (hydrated, candidate) = loader_backed_local_call(corruption);
        let error = verify(hydrated, candidate, &generous_limits())
            .expect_err("corrupt raw statement placement must fail before P2");
        assert!(matches!(
            error,
            VerificationError::SemanticViolation {
                obligation: VerificationObligation::ExactHydrationBinding,
                location: VerificationLocation::Function { .. },
                detail,
            } if detail.contains(expected)
        ));
    }
}

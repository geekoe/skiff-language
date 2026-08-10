use skiff_artifact_model::{CallableEffectSummary, TypeRefIr};
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
fn exact_empty_admission_reaches_effect_and_no_pending_gate() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Image,
        }
    );
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
fn analyzed_no_pending_binding_remains_exact_but_gate_stays_closed() {
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
        .expect_err("plumbed analyzed facts must not implement the semantic gate");
    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::EffectAndNoPending,
            location: VerificationLocation::Image,
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

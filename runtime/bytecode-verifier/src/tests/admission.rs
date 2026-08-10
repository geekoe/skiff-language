use skiff_runtime_linked_bytecode::{
    CandidateTable, LinkedBytecodeCandidate, LinkedBytecodeCandidateParts,
};

use crate::{
    verify, VerificationError, VerificationLimit, VerificationLocation, VerificationObligation,
};

use super::fixtures::{
    candidate_for, candidate_for_with_authority_corruption, exact_hydration, generous_limits,
    AuthorityPinCorruption,
};

#[test]
fn exact_empty_admission_reaches_the_next_fail_closed_proof() {
    let hydrated = exact_hydration();
    let candidate = candidate_for(&hydrated, None);
    let error = verify(hydrated, candidate, &generous_limits()).unwrap_err();

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::StackAndSlotState,
            location: VerificationLocation::Image,
        }
    );
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

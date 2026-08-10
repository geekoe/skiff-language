use skiff_runtime_linked_bytecode::{CandidateTable, ConstantIndex};

use crate::{VerificationError, VerificationLocation, VerificationObligation};

use super::fixtures::frozen_constants::{
    fixture,
    FrozenAuthorityPresence::{Empty, NonEmpty},
};

#[test]
fn empty_authorities_build_one_sealed_empty_heap() {
    let fixture = fixture(Empty, Empty);

    let heap =
        crate::verifier::prove_and_build_empty_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect("exact empty authorities prove an empty constant heap");

    assert!(heap.is_empty());
    assert_eq!(heap.len(), 0);
    assert!(heap.get(ConstantIndex::new(0)).is_none());
}

#[test]
fn admitted_authority_erased_by_candidate_is_an_image_violation() {
    let fixture = fixture(NonEmpty, Empty);

    let error =
        crate::verifier::prove_and_build_empty_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect_err("candidate erasure must not produce a partial heap");

    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail,
    } = error
    else {
        panic!("expected a frozen-constant semantic violation, got {error:?}");
    };
    assert_eq!(obligation, VerificationObligation::FrozenConstantSafety);
    assert_eq!(location, VerificationLocation::Image);
    assert!(detail.contains("erased"));
}

#[test]
fn candidate_authority_absent_from_hydration_is_a_first_row_violation() {
    let fixture = fixture(Empty, NonEmpty);

    let error =
        crate::verifier::prove_and_build_empty_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect_err("candidate invention must not produce a partial heap");

    let VerificationError::SemanticViolation {
        obligation,
        location,
        detail,
    } = error
    else {
        panic!("expected a frozen-constant semantic violation, got {error:?}");
    };
    assert_eq!(obligation, VerificationObligation::FrozenConstantSafety);
    assert_eq!(
        location,
        VerificationLocation::Table {
            table: CandidateTable::Constants,
            row: 0,
        }
    );
    assert!(detail.contains("introduced"));
}

#[test]
fn nonempty_exact_authority_fails_closed_without_a_partial_heap() {
    let fixture = fixture(NonEmpty, NonEmpty);

    let error =
        crate::verifier::prove_and_build_empty_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect_err("unsupported non-empty materialization must fail closed");

    assert_eq!(
        error,
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::FrozenConstantSafety,
            location: VerificationLocation::Table {
                table: CandidateTable::Constants,
                row: 0,
            },
        }
    );
}

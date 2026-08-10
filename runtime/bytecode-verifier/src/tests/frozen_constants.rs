use skiff_runtime_linked_bytecode::{CandidateTable, ConstantIndex};
use skiff_runtime_model::vm_value::{CompactTypeTag, ValueFlags, ValueSlot, VmHandle};

use crate::{
    VerificationError, VerificationLocation, VerificationObligation, VerifiedConstantHeap,
};

use super::fixtures::frozen_constants::{
    fixture, literal_fixture, ConstantCorruption,
    FrozenAuthorityPresence::{Empty, NonEmpty},
    FrozenLiteralKind,
};

#[test]
fn empty_authorities_build_one_sealed_empty_heap() {
    let fixture = fixture(Empty, Empty);

    let heap =
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect("exact empty authorities prove an empty constant heap");

    assert!(heap.is_empty());
    assert_eq!(heap.len(), 0);
    assert!(heap.get(ConstantIndex::new(0)).is_none());
}

#[test]
fn admitted_authority_erased_by_candidate_is_an_image_violation() {
    let fixture = fixture(NonEmpty, Empty);

    let error =
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
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
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
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
fn exact_null_authority_materializes_as_an_immediate() {
    assert!(build_heap(FrozenLiteralKind::Null) == Some(ValueSlot::null()));
}

#[test]
fn exact_bool_authority_materializes_as_an_immediate() {
    assert!(build_heap(FrozenLiteralKind::Bool) == Some(ValueSlot::bool(true)));
}

#[test]
fn exact_number_authority_materializes_as_an_immediate() {
    assert!(build_heap(FrozenLiteralKind::Number) == Some(ValueSlot::number(2.5)));
}

#[test]
fn exact_string_authority_materializes_as_an_image_pinned_const_ref() {
    assert!(
        build_heap(FrozenLiteralKind::String)
            == Some(ValueSlot::const_ref(
                VmHandle::new(0),
                CompactTypeTag::new(0),
                ValueFlags::new(0)
            ))
    );
}

#[test]
fn aggregate_frozen_node_is_proof_unavailable() {
    let fixture = literal_fixture(FrozenLiteralKind::Null, ConstantCorruption::AggregateNode);
    assert_eq!(
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect_err("aggregate constant nodes must fail closed"),
        VerificationError::ProofUnavailable {
            obligation: VerificationObligation::FrozenConstantSafety,
            location: VerificationLocation::Table {
                table: CandidateTable::FrozenConstantNodes,
                row: 0,
            },
        }
    );
}

#[test]
fn missing_source_node_reference_fails_closed() {
    assert_constant_corruption(
        ConstantCorruption::MissingNodeOrigin,
        "linked constant node does not match its exact source node",
    );
}

#[test]
fn missing_source_type_origin_fails_closed() {
    assert_constant_corruption(
        ConstantCorruption::MissingTypeOrigin,
        "linked constant type does not match its exact source type",
    );
}

#[test]
fn mismatched_literal_carrier_fails_closed() {
    assert_constant_corruption(ConstantCorruption::TypeMismatch, "declared as builtin bool");
}

#[test]
fn mismatched_literal_plan_fails_closed() {
    assert_constant_corruption(ConstantCorruption::WrongPlan, "lifecycle plan differs");
}

fn build_heap(kind: FrozenLiteralKind) -> Option<ValueSlot> {
    let fixture = literal_fixture(kind, ConstantCorruption::None);
    let heap: VerifiedConstantHeap =
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect("exact literal authority materializes");
    assert_eq!(heap.len(), 1);
    heap.get(ConstantIndex::new(0))
}

fn assert_constant_corruption(corruption: ConstantCorruption, detail_fragment: &str) {
    let fixture = literal_fixture(FrozenLiteralKind::Null, corruption);
    let error =
        crate::verifier::prove_and_build_constant_heap(&fixture.hydrated, &fixture.candidate)
            .expect_err("corrupted literal authority must fail closed");
    let VerificationError::SemanticViolation {
        obligation, detail, ..
    } = error
    else {
        panic!("expected a frozen-constant semantic violation, got {error:?}");
    };
    assert_eq!(obligation, VerificationObligation::FrozenConstantSafety);
    assert!(
        detail.contains(detail_fragment),
        "expected detail to contain {detail_fragment:?}, got {detail:?}"
    );
}

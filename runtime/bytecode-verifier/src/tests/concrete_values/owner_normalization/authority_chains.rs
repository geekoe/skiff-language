use skiff_artifact_model::TypeRefIr;
use skiff_runtime_linked_bytecode::{
    ArtifactTypeIndex, LinkedArtifactPoolOrigin, LinkedTypeEntry, TypeIndex,
};

use crate::{
    concrete_values::{classify_after_owner_authority_reset_for_test, prove_types_and_plans},
    tests::fixtures::{
        candidate_for, candidate_for_concrete_types, generous_limits, owner_authority_fixture,
        OwnerAuthorityFixture, OwnerRequirementMode, OwnerTypeSurface, OWNER_TYPE_PATH,
    },
    VerificationError,
};

use super::{assert_owner_violation, exact_target_type};

#[test]
fn alias_pinned_private_dependency_survives_duplicate_exact_target_edges() {
    let fixture = owner_authority_fixture(
        OwnerRequirementMode::DuplicateExact,
        OwnerTypeSurface::Private,
    );
    prove_fixture_row(&fixture, 1)
        .expect("exact source alias authorizes the private descriptor closure");
}

#[test]
fn exact_public_alias_authorizes_its_same_build_private_descriptor_closure() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Exact, OwnerTypeSurface::Public);
    prove_fixture_row(&fixture, 1)
        .expect("an exact alias carries package-build authority into its public descriptor");
}

#[test]
fn unpinned_public_alias_cannot_open_its_private_descriptor_closure() {
    let fixture = owner_authority_fixture(OwnerRequirementMode::Unpinned, OwnerTypeSurface::Public);
    assert_owner_violation(
        prove_fixture_row(&fixture, 1).map(|()| TypeRefIr::builtin("never")),
        "exact-build authority",
    );
}

#[test]
fn duplicate_package_id_edges_do_not_become_ambient_descriptor_authority() {
    let fixture = owner_authority_fixture(
        OwnerRequirementMode::DuplicateExact,
        OwnerTypeSurface::Public,
    );
    assert_owner_violation(
        prove_fixture_row(&fixture, 2).map(|()| TypeRefIr::builtin("never")),
        "exact-build authority",
    );
}

#[test]
fn private_dependency_authority_is_cleared_before_the_next_row() {
    let fixture = owner_authority_fixture(
        OwnerRequirementMode::DuplicateExact,
        OwnerTypeSurface::Private,
    );
    let candidate = candidate_for(&fixture.hydrated, None);
    let result = classify_after_owner_authority_reset_for_test(
        &fixture.hydrated,
        &candidate,
        &fixture.caller_build_id,
        &fixture.target.package_build_id,
        &exact_target_type(&fixture, OWNER_TYPE_PATH),
        &generous_limits(),
    );
    assert_owner_violation(
        result.map(|()| TypeRefIr::builtin("never")),
        "exact-build authority",
    );
}

fn prove_fixture_row(
    fixture: &OwnerAuthorityFixture,
    artifact_index: u32,
) -> Result<(), VerificationError> {
    let expected = exact_target_type(fixture, OWNER_TYPE_PATH);
    let linked = LinkedTypeEntry::new(
        TypeIndex::new(0),
        LinkedArtifactPoolOrigin::new(
            fixture.caller_build_id.clone(),
            ArtifactTypeIndex::new(artifact_index),
            None,
        )
        .unwrap(),
        expected,
        None,
    );
    let candidate = candidate_for_concrete_types(&fixture.hydrated, vec![linked], Vec::new())
        .expect("owner authority candidate is locally valid");
    prove_types_and_plans(&fixture.hydrated, &candidate, &generous_limits()).map(|_| ())
}

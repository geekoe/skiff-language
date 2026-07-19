use skiff_artifact_model::{CallTargetIr, ExprIr, PackageCallableId, PackageRefIr};

use crate::emission::artifact::{PublishedFileIrArtifact, PublishedResourceArtifact};

use super::{
    fixture, materialize_package_artifact, package_requirement, push_package_call,
    refresh_file_and_artifact_identities, PackageArtifact,
};

#[test]
fn materializer_rejects_invalid_package_call_reference_pairs_before_identity_validation() {
    let (mut artifact, mut file, resource) = fixture();
    artifact.package_requirements = vec![package_requirement("direct", "example.com/direct")];
    push_package_call(
        &mut file,
        PackageRefIr::Dependency {
            dependency_ref: "direct".to_string(),
        },
    );
    refresh_file_and_artifact_identities(&mut artifact, &mut file);

    let mut missing = file.clone();
    missing.unit.external_refs.package_callables.clear();
    assert_invalid_pair_error(
        &artifact,
        &missing,
        &resource,
        "has no matching packageCallables",
    );

    let mut orphan = file.clone();
    orphan.unit.constants[0].body.expressions.clear();
    assert_invalid_pair_error(&artifact, &orphan, &resource, "is not referenced");

    let mut mismatch = file.clone();
    let CallTargetIr::PackageCallable {
        package_callable_id,
        ..
    } = first_call_target(&mut mismatch)
    else {
        unreachable!()
    };
    *package_callable_id = PackageCallableId::new("callable:other");
    assert_invalid_pair_error(&artifact, &mismatch, &resource, "has no exact entry");

    let mut duplicate = file.clone();
    duplicate
        .unit
        .external_refs
        .package_callables
        .push(duplicate.unit.external_refs.package_callables[0].clone());
    assert_invalid_pair_error(&artifact, &duplicate, &resource, "have the same packageRef");
}

fn first_call_target(file: &mut PublishedFileIrArtifact) -> &mut CallTargetIr {
    let ExprIr::Call { call } = &mut file.unit.constants[0].body.expressions[0] else {
        unreachable!()
    };
    &mut call.target
}

fn assert_invalid_pair_error(
    artifact: &PackageArtifact,
    file: &PublishedFileIrArtifact,
    resource: &PublishedResourceArtifact,
    expected: &str,
) {
    let error = materialize_package_artifact(
        artifact,
        std::slice::from_ref(file),
        std::slice::from_ref(resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("invalid package-call references") && error.contains(expected),
        "unexpected error: {error}"
    );
}

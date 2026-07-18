use skiff_artifact_identity::{assign_file_ir_identity, assign_package_artifact_identities};
use skiff_artifact_model::{
    PackageArtifact, PackageCallableId, PackageCallableRef, PackageLocalAbiIdentity, PackageRefIr,
    PackageRequirement,
};

use crate::emission::{
    artifact::PublishedFileIrArtifact, package_artifact::materialize_package_artifact,
};

use super::fixture;

#[test]
fn materializer_accepts_covered_package_callable_coordinates_with_extra_requirements() {
    let (mut projected, mut file, resource) = fixture();
    file.unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::Dependency {
            dependency_ref: "direct".to_string(),
        }));
    file.unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::PackageId {
            package_id: "example.com/transitive".to_string(),
        }));
    projected.package_requirements = vec![
        package_requirement("direct", "example.com/direct"),
        package_requirement("transitive", "example.com/transitive"),
        package_requirement("unused", "example.com/unused"),
    ];
    refresh_file_and_artifact_identities(&mut projected, &mut file);

    materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap();
}

#[test]
fn materializer_rejects_unknown_alias_and_package_id_coordinates() {
    let (mut alias_artifact, mut alias_file, resource) = fixture();
    alias_file
        .unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::Dependency {
            dependency_ref: "missing".to_string(),
        }));
    refresh_file_and_artifact_identities(&mut alias_artifact, &mut alias_file);
    let alias_error = materialize_package_artifact(
        &alias_artifact,
        std::slice::from_ref(&alias_file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        alias_error
            .contains("packageCallables references unknown package dependency alias missing"),
        "unexpected error: {alias_error}"
    );

    let (mut id_artifact, mut id_file, resource) = fixture();
    id_file
        .unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::PackageId {
            package_id: "example.com/missing".to_string(),
        }));
    refresh_file_and_artifact_identities(&mut id_artifact, &mut id_file);
    let id_error = materialize_package_artifact(
        &id_artifact,
        std::slice::from_ref(&id_file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        id_error.contains("packageCallables references unknown package id example.com/missing"),
        "unexpected error: {id_error}"
    );
}

#[test]
fn materializer_rejects_package_callable_external_self_references() {
    let (mut alias_artifact, mut alias_file, resource) = fixture();
    alias_file
        .unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::Dependency {
            dependency_ref: "self".to_string(),
        }));
    alias_artifact.package_requirements = vec![package_requirement(
        "self",
        alias_artifact.package_id.as_str(),
    )];
    refresh_file_and_artifact_identities(&mut alias_artifact, &mut alias_file);

    let alias_error = materialize_package_artifact(
        &alias_artifact,
        std::slice::from_ref(&alias_file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        alias_error.contains(
            "packageCallables contains unrewritten external self reference through dependency alias self"
        ),
        "unexpected error: {alias_error}"
    );

    let (mut id_artifact, mut id_file, resource) = fixture();
    id_file
        .unit
        .external_refs
        .package_callables
        .push(package_callable(PackageRefIr::PackageId {
            package_id: id_artifact.package_id.clone(),
        }));
    refresh_file_and_artifact_identities(&mut id_artifact, &mut id_file);

    let id_error = materialize_package_artifact(
        &id_artifact,
        std::slice::from_ref(&id_file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        id_error.contains(
            "packageCallables contains unrewritten external self reference through package id example.com/pkg"
        ),
        "unexpected error: {id_error}"
    );
}

fn package_requirement(alias: &str, package_id: &str) -> PackageRequirement {
    PackageRequirement {
        alias: alias.to_string(),
        package_id: package_id.to_string(),
        exact_version: "1.0.0".to_string(),
        expected_local_abi: PackageLocalAbiIdentity::new(format!("abi:{package_id}")),
    }
}

fn package_callable(package_ref: PackageRefIr) -> PackageCallableRef {
    PackageCallableRef {
        package_ref,
        package_callable_id: PackageCallableId::new("callable:run"),
    }
}

fn refresh_file_and_artifact_identities(
    artifact: &mut PackageArtifact,
    file: &mut PublishedFileIrArtifact,
) {
    assign_file_ir_identity(&mut file.unit).unwrap();
    file.identity = file.unit.file_ir_identity.clone();
    artifact.files[0].file_ir_identity = file.identity.clone();
    assign_package_artifact_identities(artifact).unwrap();
}

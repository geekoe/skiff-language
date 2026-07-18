use skiff_artifact_identity::{assign_file_ir_identity, assign_package_artifact_identities};
use skiff_artifact_model::{
    PackageArtifact, PackageLocalAbiIdentity, PackageOperationSymbolRef, PackageRefIr,
    PackageRequirement, PackageSymbolRef, PublicationOperationKind,
};

use crate::emission::{
    artifact::PublishedFileIrArtifact, package_artifact::materialize_package_artifact,
};

use super::fixture;

#[test]
fn materializer_accepts_covered_alias_and_id_refs_with_extra_graph_requirements() {
    let (mut projected, mut file, resource) = fixture();
    file.unit
        .external_refs
        .package_symbols
        .push(package_symbol(PackageRefIr::Dependency {
            dependency_ref: "direct".to_string(),
        }));
    file.unit
        .external_refs
        .package_operation_symbols
        .push(package_operation_symbol(PackageRefIr::PackageId {
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
        .package_symbols
        .push(package_symbol(PackageRefIr::Dependency {
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
        alias_error.contains("unknown package dependency alias missing"),
        "unexpected error: {alias_error}"
    );

    let (mut id_artifact, mut id_file, resource) = fixture();
    id_file
        .unit
        .external_refs
        .package_operation_symbols
        .push(package_operation_symbol(PackageRefIr::PackageId {
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
        id_error.contains("unknown package id example.com/missing"),
        "unexpected error: {id_error}"
    );
}

#[test]
fn materializer_rejects_unrewritten_external_self_reference() {
    let (mut projected, mut file, resource) = fixture();
    file.unit
        .external_refs
        .package_symbols
        .push(package_symbol(PackageRefIr::PackageId {
            package_id: projected.package_id.clone(),
        }));
    refresh_file_and_artifact_identities(&mut projected, &mut file);

    let error = materialize_package_artifact(
        &projected,
        std::slice::from_ref(&file),
        std::slice::from_ref(&resource),
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("unrewritten external self reference"),
        "unexpected error: {error}"
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

fn package_symbol(package: PackageRefIr) -> PackageSymbolRef {
    PackageSymbolRef {
        package,
        symbol_path: "Thing".to_string(),
        abi_expectation: None,
    }
}

fn package_operation_symbol(package_ref: PackageRefIr) -> PackageOperationSymbolRef {
    PackageOperationSymbolRef {
        package_ref,
        operation: skiff_artifact_model::OperationAbiRef {
            operation_abi_id: "operation".to_string(),
            kind: PublicationOperationKind::PublicFunction,
            public_path: "run".to_string(),
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            display_name: "run".to_string(),
        },
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

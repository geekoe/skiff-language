use super::*;

#[test]
fn package_body_change_changes_build_identity_not_local_abi_identity() {
    let base = package_fixture("hello");
    let body_changed = package_fixture("changed");

    assert_ne!(build_identity(&base), build_identity(&body_changed));
    assert_eq!(local_abi_identity(&base), local_abi_identity(&body_changed));
}

#[test]
fn package_local_abi_is_distinct_from_the_nested_publication_identity() {
    let unit = package_fixture("hello");
    let local_abi = local_abi_identity(&unit);
    let publication_abi =
        publication_abi_identity(&unit.publication_abi).expect("publication ABI identity");

    assert_ne!(local_abi, publication_abi);
    assert_eq!(publication_abi, unit.publication_abi.abi_identity);
    assert!(local_abi.starts_with(PACKAGE_LOCAL_ABI_IDENTITY_PREFIX));
    assert!(publication_abi.starts_with(PUBLICATION_ABI_IDENTITY_PREFIX));
    assert_ne!(
        package_local_abi_hash(&unit).unwrap(),
        publication_abi_hash(&unit.publication_abi).unwrap()
    );
}

#[test]
fn package_implementation_links_identity_preserves_v1_wire_golden() {
    let unit = package_fixture("hello");

    assert_eq!(
        package_implementation_links_identity(&unit.implementation_links)
            .expect("implementation links identity"),
        // Canonical implementation-link wire with File IR v5 target identity.
        "skiff-package-implementation-links-v1:sha256:4fb0ad52def218e2c4e8433639758e77466e43cf88cb15e9051a918300ddcf90"
    );
}

#[test]
fn package_implementation_links_identity_changes_with_link_targets() {
    let unit = package_fixture("hello");
    let original = package_implementation_links_identity(&unit.implementation_links)
        .expect("original implementation links identity");
    let mut changed = unit.implementation_links;
    changed
        .functions
        .get_mut("run")
        .expect("fixture function link")
        .symbol = "renamed".to_string();

    assert_ne!(
        package_implementation_links_identity(&changed)
            .expect("changed implementation links identity"),
        original
    );
}

#[test]
fn package_identity_validation_rejects_nested_and_outer_tampering() {
    let assigned = package_fixture("hello");
    validate_package_unit_identities(&assigned).expect("assigned package must validate");

    let mut nested_publication = assigned.clone();
    nested_publication.publication_abi.abi_identity = "tampered-publication".to_string();
    assert!(matches!(
        validate_package_unit_identities(&nested_publication),
        Err(ArtifactIdentityError::PublicationAbiIdentityMismatch { .. })
    ));

    let mut nested_operation = assigned.clone();
    nested_operation.publication_abi.operation_abi[0]
        .operation
        .operation_abi_id = "tampered-operation".to_string();
    assert!(matches!(
        validate_package_unit_identities(&nested_operation),
        Err(ArtifactIdentityError::InvalidPublicationAbiSurface { .. })
    ));

    let mut outer_abi = assigned.clone();
    outer_abi.abi_identity = "tampered-local-abi".to_string();
    assert!(matches!(
        validate_package_unit_identities(&outer_abi),
        Err(ArtifactIdentityError::PackageAbiIdentityMismatch { .. })
    ));

    let mut outer_build = assigned;
    outer_build.build_identity = "tampered-build".to_string();
    assert!(matches!(
        validate_package_unit_identities(&outer_build),
        Err(ArtifactIdentityError::PackageBuildIdentityMismatch { .. })
    ));
}

#[test]
fn package_validation_rejects_a_mismatched_nested_publication_coordinate() {
    let mut unit = package_fixture("hello");
    unit.publication_abi.publication_id = "example.com/other".to_string();

    assert!(matches!(
        validate_package_unit_identities(&unit),
        Err(ArtifactIdentityError::PackagePublicationCoordinateMismatch { .. })
    ));
}

#[test]
fn package_validation_rejects_tampered_implementation_operation_refs() {
    let assigned = package_fixture("hello");

    let mut mismatched_key = assigned.clone();
    let target = mismatched_key
        .implementation_links
        .operation_targets
        .pop_first()
        .expect("fixture operation target")
        .1;
    mismatched_key
        .implementation_links
        .operation_targets
        .insert("tampered-key".to_string(), target);
    assert!(matches!(
        validate_package_unit_identities(&mismatched_key),
        Err(ArtifactIdentityError::InvalidPackageIdentityInput { .. })
    ));

    let mut mismatched_ref = assigned;
    let target = mismatched_ref
        .implementation_links
        .operation_targets
        .values_mut()
        .next()
        .expect("fixture operation target");
    let PackageOperationTarget::LocalExecutable { operation, .. } = target else {
        panic!("fixture target must be a local executable");
    };
    operation.public_path = "other".to_string();
    assert!(matches!(
        validate_package_unit_identities(&mismatched_ref),
        Err(ArtifactIdentityError::InvalidPublicationAbiSurface { .. })
    ));
}

#[test]
fn assign_package_unit_identities_sets_nested_and_outer_identities() {
    let mut unit = package_fixture("hello");
    unit.publication_abi.publication_id = "stale-publication".to_string();
    unit.publication_abi.version = "0.0.0".to_string();
    unit.publication_abi.abi_identity = "stale-publication-abi".to_string();
    unit.abi_identity = "stale-package-abi".to_string();
    unit.build_identity = "stale-build".to_string();

    let (build_identity, abi_identity) =
        assign_package_unit_identities(&mut unit).expect("assign package identities");

    assert_eq!(unit.publication_abi.publication_id, unit.package_id);
    assert_eq!(unit.publication_abi.version, unit.version);
    assert_eq!(unit.build_identity, build_identity);
    assert_eq!(unit.abi_identity, abi_identity);
    assert_eq!(
        unit.publication_abi.abi_identity,
        publication_abi_identity(&unit.publication_abi).expect("publication ABI identity")
    );
    validate_package_unit_identities(&unit).expect("assigned package identities validate");
}

#[test]
fn resource_content_changes_package_build_identity_not_local_abi_identity() {
    let mut first = package_fixture("hello");
    first.resources = vec![resource_ref("prompts/system.md", "aaa")];
    let mut second = package_fixture("hello");
    second.resources = vec![resource_ref("prompts/system.md", "bbb")];

    assert_ne!(build_identity(&first), build_identity(&second));
    assert_eq!(local_abi_identity(&first), local_abi_identity(&second));
}

#[test]
fn resource_refs_change_service_unit_and_dynamic_build_identity_not_protocol_identity() {
    let mut first = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
    first.resources = vec![resource_ref("prompts/system.md", "aaa")];
    let mut second = first.clone();
    second.resources = vec![resource_ref("prompts/system.md", "bbb")];

    assert_eq!(first.protocol_identity, second.protocol_identity);
    assert_ne!(
        service_unit_identity(&first).expect("first service unit identity"),
        service_unit_identity(&second).expect("second service unit identity")
    );
    let first_dynamic = runtime_program_dynamic_build_id(
        &runtime_program_service_unit_identity_bytes(&first).expect("first dynamic bytes"),
        [],
    );
    let second_dynamic = runtime_program_dynamic_build_id(
        &runtime_program_service_unit_identity_bytes(&second).expect("second dynamic bytes"),
        [],
    );
    assert_ne!(first_dynamic, second_dynamic);
}

pub(super) fn build_identity(unit: &PackageUnit) -> String {
    package_build_identity(unit).expect("package build identity")
}

pub(super) fn local_abi_identity(unit: &PackageUnit) -> String {
    package_local_abi_identity(unit).expect("package local ABI identity")
}

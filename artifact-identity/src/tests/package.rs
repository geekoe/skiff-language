use super::*;

#[test]
fn package_body_change_changes_build_identity_not_abi_identity() {
    let base = package_fixture("hello");
    let body_changed = package_fixture("changed");

    assert_ne!(
        package_build_identity(&base).expect("base build identity"),
        package_build_identity(&body_changed).expect("changed build identity")
    );
    assert_eq!(
        package_abi_identity(&base).expect("base abi identity"),
        package_abi_identity(&body_changed).expect("changed abi identity")
    );
}

#[test]
fn package_abi_identity_uses_publication_abi_surface() {
    let mut unit = package_fixture("hello");
    let package_abi = package_abi_identity(&unit).expect("package abi identity");
    let package_hash = package_abi_hash(&unit).expect("package abi hash");
    let publication_hash =
        publication_abi_hash(&unit.publication_abi).expect("publication abi hash");
    let publication_abi =
        publication_abi_identity(&unit.publication_abi).expect("publication abi identity");

    assert_eq!(package_hash, publication_hash);
    assert_ne!(package_abi, publication_abi);
    assert_eq!(publication_abi, unit.publication_abi.abi_identity);
    assert!(package_abi.starts_with(PACKAGE_ABI_IDENTITY_PREFIX));
    assert!(publication_abi.starts_with(PUBLICATION_ABI_IDENTITY_PREFIX));

    let original = package_abi;
    let original_hash = package_hash;
    let original_publication = publication_abi;
    let link = unit
        .implementation_links
        .functions
        .get_mut("run")
        .expect("run implementation link");
    link.executable_index = 42;
    link.signature.return_type = TypeRefIr::native("number");

    assert_eq!(
        original,
        package_abi_identity(&unit).expect("implementation changed package abi identity")
    );
    assert_eq!(
        original_hash,
        package_abi_hash(&unit).expect("implementation changed package abi hash")
    );
    assert_eq!(
        original_publication,
        publication_abi_identity(&unit.publication_abi).expect("publication abi identity")
    );

    unit.publication_abi.operation_abi[0]
        .public_signature
        .return_type = TypeRefIr::native("number");
    let changed_publication_abi =
        publication_abi_identity(&unit.publication_abi).expect("changed publication abi identity");
    assert_ne!(original_publication, changed_publication_abi);
    assert_ne!(
        original,
        package_abi_identity(&unit).expect("changed publication package abi identity")
    );
    assert_ne!(
        original_hash,
        package_abi_hash(&unit).expect("changed publication package abi hash")
    );
}

#[test]
fn package_identity_validation_rejects_stale_build_or_abi_identity() {
    let mut unit = package_fixture("hello");
    unit.build_identity = package_build_identity(&unit).expect("build identity");
    unit.abi_identity = "stale-abi".to_string();

    let error = validate_package_unit_identities(&unit).expect_err("stale ABI must fail");
    assert!(matches!(
        error,
        ArtifactIdentityError::PackageAbiIdentityMismatch { .. }
    ));

    unit.abi_identity = package_abi_identity(&unit).expect("abi identity");
    validate_package_unit_identities(&unit).expect("computed identities should validate");
    unit.build_identity = "stale-build".to_string();
    let error = validate_package_unit_identities(&unit).expect_err("stale build must fail");
    assert!(matches!(
        error,
        ArtifactIdentityError::PackageBuildIdentityMismatch { .. }
    ));
}

#[test]
fn resource_refs_change_package_build_identity_not_abi_identity() {
    let mut first = package_fixture("hello");
    first.resources = vec![resource_ref("prompts/system.md", "aaa")];
    let mut second = package_fixture("hello");
    second.resources = vec![resource_ref("prompts/system.md", "bbb")];

    assert_ne!(
        package_build_identity(&first).expect("first build identity"),
        package_build_identity(&second).expect("second build identity")
    );
    assert_eq!(
        package_abi_identity(&first).expect("first ABI identity"),
        package_abi_identity(&second).expect("second ABI identity")
    );
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

#[test]
fn assign_package_unit_identities_sets_publication_and_package_identities() {
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

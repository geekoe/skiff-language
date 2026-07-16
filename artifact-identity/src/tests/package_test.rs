use super::*;

#[test]
fn package_test_build_identity_excludes_persisted_entrypoint_id() {
    let mut assembly = package_test_assembly_fixture();
    let original_identity =
        package_test_build_identity(&assembly).expect("package test build identity");
    let original_projection = canonical_package_test_build_identity_value(&assembly)
        .expect("package test build identity projection");

    assert!(original_projection.get("testBuildIdentity").is_none());
    assert!(original_projection.get("sourceMap").is_none());
    assert!(original_projection
        .pointer("/testEntrypoints/0/entrypointId")
        .is_none());

    assembly.test_entrypoints[0].entrypoint_id =
        "skiff-package-test-entrypoint-v1:sha256:tampered".to_string();

    assert_eq!(
        original_identity,
        package_test_build_identity(&assembly)
            .expect("entrypoint id should not affect package test build identity")
    );
    assert_eq!(
        original_projection,
        canonical_package_test_build_identity_value(&assembly)
            .expect("entrypoint id should not affect package test build projection")
    );
}

#[test]
fn package_test_build_identity_includes_entrypoint_config_and_effect_metadata() {
    let mut assembly = package_test_assembly_fixture();
    let original_identity =
        package_test_build_identity(&assembly).expect("package test build identity");

    assembly.test_entrypoints[0]
        .config_and_effect_metadata
        .config
        .insert("first.secret".to_string(), MetadataValue::Bool(true));

    assert_ne!(
        original_identity,
        package_test_build_identity(&assembly)
            .expect("entrypoint metadata should affect package test build identity")
    );
}

#[test]
fn package_test_entrypoint_id_derivation_uses_build_identity_and_local_id() {
    let local_id = package_test_entrypoint_local_id(
        "example.com/pkg",
        "1.0.0",
        "tests/pkg.test.skiff",
        0,
        "runs internal helper",
    )
    .expect("entrypoint local id");
    let changed_local_id = package_test_entrypoint_local_id(
        "example.com/pkg",
        "1.0.0",
        "tests/pkg.test.skiff",
        1,
        "runs internal helper",
    )
    .expect("changed entrypoint local id");

    let entrypoint_id =
        derive_package_test_entrypoint_id("skiff-package-test-build-v1:sha256:aaaaaaaa", &local_id)
            .expect("entrypoint id");
    let changed_build_entrypoint_id =
        derive_package_test_entrypoint_id("skiff-package-test-build-v1:sha256:bbbbbbbb", &local_id)
            .expect("changed build entrypoint id");
    let changed_local_entrypoint_id = derive_package_test_entrypoint_id(
        "skiff-package-test-build-v1:sha256:aaaaaaaa",
        &changed_local_id,
    )
    .expect("changed local entrypoint id");

    assert!(local_id.starts_with(PACKAGE_TEST_ENTRYPOINT_LOCAL_ID_PREFIX));
    assert!(entrypoint_id.starts_with(PACKAGE_TEST_ENTRYPOINT_ID_PREFIX));
    assert_ne!(local_id, changed_local_id);
    assert_ne!(entrypoint_id, changed_build_entrypoint_id);
    assert_ne!(entrypoint_id, changed_local_entrypoint_id);
}

#[test]
fn package_test_identity_validation_recomputes_entrypoint_ids() {
    let mut assembly = package_test_assembly_fixture();
    assembly.test_build_identity =
        package_test_build_identity(&assembly).expect("package test build identity");
    assembly.test_entrypoints[0].entrypoint_id = derive_package_test_entrypoint_id(
        &assembly.test_build_identity,
        &assembly.test_entrypoints[0].entrypoint_local_id,
    )
    .expect("entrypoint id");

    validate_package_test_assembly_identity(&assembly)
        .expect("matching package test identities should validate");

    assembly.test_entrypoints[0].entrypoint_id =
        "skiff-package-test-entrypoint-v1:sha256:tampered".to_string();
    let error = validate_package_test_assembly_identity(&assembly)
        .expect_err("tampered entrypoint id must fail validation");
    assert!(matches!(
        error,
        ArtifactIdentityError::PackageTestEntrypointIdMismatch { .. }
    ));
}

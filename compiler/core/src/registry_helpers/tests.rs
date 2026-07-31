use super::{
    is_valid_official_registry_path, validate_official_registry_package_path,
    validate_std_registry_package_id,
};
use crate::id::SKIFF_STD_PUBLICATION_ID;

#[test]
fn std_registry_package_id_allows_only_std_package() {
    validate_std_registry_package_id(SKIFF_STD_PUBLICATION_ID).unwrap();

    let error = validate_std_registry_package_id("skiff.run/other").unwrap_err();

    assert_eq!(
            error.to_string(),
            "std registry package skiff.run/other is invalid; std registry can only declare skiff.run/std"
        );
}

#[test]
fn official_registry_path_allows_only_known_safe_forms() {
    assert!(is_valid_official_registry_path("."));
    assert!(is_valid_official_registry_path("std"));
    assert!(is_valid_official_registry_path("../std"));

    assert!(!is_valid_official_registry_path(""));
    assert!(!is_valid_official_registry_path("  "));
    assert!(!is_valid_official_registry_path("std/core"));
    assert!(!is_valid_official_registry_path("../../std"));
    assert!(!is_valid_official_registry_path(".."));
    assert!(!is_valid_official_registry_path("std\\core"));
}

#[test]
fn official_registry_path_error_is_context_free() {
    let error =
        validate_official_registry_package_path(SKIFF_STD_PUBLICATION_ID, "std/core").unwrap_err();

    assert_eq!(
        error.to_string(),
        "std registry package skiff.run/std has invalid path std/core"
    );
}

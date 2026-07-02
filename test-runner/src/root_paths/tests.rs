use super::*;
use std::fs;

use skiff_compiler::test_support::{
    project_fixtures::TestDir, TestPackageApiEntry, TestPackageManifest,
};

fn official_std_manifest() -> TestPackageManifest {
    TestPackageManifest {
        id: SKIFF_STD_PUBLICATION_ID.to_string(),
        version: "1.0.0".to_string(),
        api: vec![TestPackageApiEntry::module("api", "api")],
        dependencies: Vec::new(),
        path: PathBuf::from("package.yml"),
        synthetic: false,
    }
}

#[test]
fn official_package_friend_test_module_path_uses_internal_production_identity() {
    let manifest = official_std_manifest();
    let export_sources = BTreeMap::from([(PathBuf::from("api.skiff"), "std.api".to_string())]);

    let friend_module_path = package_module_path(
        &manifest,
        Path::new("internal.live.test.skiff"),
        Some(Path::new("internal.skiff")),
        true,
        &export_sources,
    );
    let integration_module_path = package_module_path(
        &manifest,
        Path::new("integration/internal.live.test.skiff"),
        None,
        true,
        &export_sources,
    );

    assert_eq!(friend_module_path, "std.internal.__test");
    assert_eq!(integration_module_path, "integration.internal.live.__test");
}

#[test]
fn official_std_export_sources_skip_prelude_owned_api_entries() {
    let temp = TestDir::new("skiff-test-runner", "std-prelude-api-source");
    let std_dir = temp.path().join("std");
    let prelude_dir = temp.path().join("prelude");
    fs::create_dir_all(&std_dir).unwrap();
    fs::create_dir_all(&prelude_dir).unwrap();
    fs::write(
        std_dir.join("http.skiff"),
        "function ok() -> bool { return true }\n",
    )
    .unwrap();
    fs::write(
        prelude_dir.join("error.skiff"),
        "type DecodeError { message: string }\n",
    )
    .unwrap();

    let mut manifest = official_std_manifest();
    manifest.path = std_dir.join("package.yml");
    manifest.api = vec![
        TestPackageApiEntry::source("http.ok", "http", "ok"),
        TestPackageApiEntry::source("error.DecodeError", "error", "DecodeError"),
    ];

    let export_sources = export_source_paths(&manifest, &std_dir).unwrap();

    assert_eq!(
        export_sources,
        BTreeMap::from([(PathBuf::from("http.skiff"), "std.http".to_string())])
    );
}

#[test]
fn user_package_export_sources_reject_missing_api_source() {
    let temp = TestDir::new("skiff-test-runner", "user-missing-api-source");
    let package_dir = temp.path().join("package");
    fs::create_dir_all(&package_dir).unwrap();
    let manifest = TestPackageManifest {
        id: "example.com/package".to_string(),
        version: "1.0.0".to_string(),
        api: vec![TestPackageApiEntry::source(
            "missing.Value",
            "missing",
            "Value",
        )],
        dependencies: Vec::new(),
        path: package_dir.join("package.yml"),
        synthetic: false,
    };

    let error = export_source_paths(&manifest, &package_dir).unwrap_err();
    assert!(error
        .to_string()
        .contains("package example.com/package api source missing has no source file"));
}

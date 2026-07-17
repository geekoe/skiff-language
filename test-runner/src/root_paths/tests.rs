use super::*;
use std::fs;

use skiff_compiler::test_support::{
    project_fixtures::TestDir, TestPackageApiEntry, TestPackageManifest,
};
use skiff_compiler::SourceTreeFile;
use skiff_syntax::parser::parse_source;

use crate::visibility::{production_symbols_for_ast, service_production_exports};

fn parsed_service_source(module_path: &str, is_test_file: bool, text: &str) -> ParsedSource {
    ParsedSource {
        source: SourceTreeFile {
            module_path: module_path.to_string(),
            file_path: PathBuf::from(module_path.replace('.', "/")).with_extension("skiff"),
            is_test_file,
            byte_len: text.len() as u64,
        },
        text: text.to_string(),
        ast: parse_source(text).expect("source should parse"),
    }
}

fn parsed_package_source(module_path: &str, is_test_file: bool, text: &str) -> PackageTestSource {
    PackageTestSource {
        relative_path: PathBuf::from(module_path.replace('.', "/")).with_extension("skiff"),
        module_path: module_path.to_string(),
        is_test_file,
        text: text.to_string(),
        ast: parse_source(text).expect("source should parse"),
    }
}

fn official_std_manifest() -> TestPackageManifest {
    TestPackageManifest {
        id: SKIFF_STD_PUBLICATION_ID.to_string(),
        version: "1.0.0".to_string(),
        api: vec![TestPackageApiEntry::module("api", "api")],
        dependencies: Vec::new(),
        resources: Vec::new(),
        path: PathBuf::from("package.yml"),
        synthetic: false,
    }
}

#[test]
fn official_package_test_module_path_uses_its_own_source_identity() {
    let manifest = official_std_manifest();
    let export_sources = BTreeMap::from([(PathBuf::from("api.skiff"), "std.api".to_string())]);

    let colocated_module_path = package_module_path(
        &manifest,
        Path::new("internal.live.test.skiff"),
        true,
        &export_sources,
    );
    let integration_module_path = package_module_path(
        &manifest,
        Path::new("integration/internal.live.test.skiff"),
        true,
        &export_sources,
    );

    assert_eq!(colocated_module_path, "std.internal.live.__test");
    assert_eq!(
        integration_module_path,
        "std.integration.internal.live.__test"
    );
}

#[test]
fn service_test_root_index_exposes_all_current_top_level_symbol_kinds() {
    let production = parsed_service_source(
        "internal.models",
        false,
        r#"
            type Secret { value: number }
            type Record { id: string }
            db object Record { name "record"; primary key(id) }
            function secret() -> number { return 7 }
        "#,
    );
    let test = parsed_service_source(
        "checks.visibility",
        true,
        r#"
            function echo(value: root.internal.models.Secret) -> root.internal.models.Secret {
                return value
            }

            test "current service symbols are visible" {
                const rows = db find many root.internal.models.Record {}
                assert root.internal.models.secret() == 7
                assert rows.length >= 0
            }
        "#,
    );
    let exports = service_production_exports(std::slice::from_ref(&production));

    resolve_service_test_root_paths(vec![test], &[production], &exports)
        .expect("all current service top-level symbols should resolve");
}

#[test]
fn dependency_root_index_contains_only_exported_symbols() {
    let dependency = parse_source(
        r#"
            function visible() -> number { return 1 }
            function hidden() -> number { return 2 }
        "#,
    )
    .unwrap();
    let mut symbols = production_symbols_for_ast(&dependency, true);
    symbols.symbols.get_mut("visible").unwrap().exported = true;
    let exports = BTreeMap::from([("dependency.api".to_string(), symbols)]);
    let test = parsed_service_source(
        "checks.dependency",
        true,
        r#"
            test "dependency private symbol is hidden" {
                assert root.dependency.api.hidden() == 2
            }
        "#,
    );

    let error = resolve_service_test_root_paths(vec![test], &[], &exports)
        .expect_err("dependency private symbol must stay outside the root index");
    assert!(error.to_string().contains("hidden"));
}

#[test]
fn package_test_root_index_exposes_all_current_top_level_symbol_kinds() {
    let production = parsed_package_source(
        "internal.models",
        false,
        r#"
            type Secret { value: number }
            type Record { id: string }
            db object Record { name "record"; primary key(id) }
            function secret() -> number { return 7 }
        "#,
    );
    let test = parsed_package_source(
        "checks.visibility.__test",
        true,
        r#"
            function echo(value: root.internal.models.Secret) -> root.internal.models.Secret {
                return value
            }

            test "current package symbols are visible" {
                const rows = db find many root.internal.models.Record {}
                assert root.internal.models.secret() == 7
                assert rows.length >= 0
            }
        "#,
    );

    resolve_package_test_root_paths(vec![test], &[production])
        .expect("all current package top-level symbols should resolve");
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
        resources: Vec::new(),
        path: package_dir.join("package.yml"),
        synthetic: false,
    };

    let error = export_source_paths(&manifest, &package_dir).unwrap_err();
    assert!(error
        .to_string()
        .contains("package example.com/package api source missing has no source file"));
}

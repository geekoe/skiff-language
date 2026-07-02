use std::{
    fs,
    path::{Path, PathBuf},
};

mod common;
use common::{
    test_runner::{
        assert_counts, assert_failed, assert_failed_result_or_error, assert_passed,
        matching_results, run_tests, run_tests_with_package_dirs,
    },
    TestDir,
};
use skiff_artifact_model::ConfigAndEffectMetadata;
use skiff_compiler::{
    test_support::{
        compile_source_file_ir_artifact_for_test, list_package_test_assemblies,
        project_fixtures::PackageProjectBuilder, write_package_test_artifact_root,
        TestPackageTestArtifactInput, TestPackageTestEntrypointInput,
        TestPackageTestFileIrArtifact,
    },
    PublicationId,
};

fn package_store_path(root: &Path, package_id: &str, version: &str) -> PathBuf {
    root.join(
        PublicationId::parse(package_id)
            .expect("test package id should be valid")
            .artifact_path(),
    )
    .join(version)
}

fn public_answer_api_yml() -> &'static str {
    "api: { publicAnswer: api.publicAnswer }\n"
}

fn math_project_with_api_yml(name: &str, api_yml: &str) -> PackageProjectBuilder {
    PackageProjectBuilder::new(name).with_manifest("example.com/math", api_yml)
}

fn write_api_surface(package_dir: &Path, source_module: &str, symbols: &[&str]) {
    write_api_yml_for_module(package_dir, source_module, source_module, symbols);
}

fn write_main_surface(package_dir: &Path, symbols: &[&str]) {
    write_api_yml_for_module(package_dir, "main", "main", symbols);
}

fn write_api_yml_for_module(
    package_dir: &Path,
    public_module: &str,
    source_module: &str,
    symbols: &[&str],
) {
    let entries = symbols
        .iter()
        .map(|symbol| format!("{symbol}: {source_module}.{symbol}"))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        package_dir.join("api.yml"),
        format!("{public_module}: {{ {entries} }}\n"),
    )
    .unwrap();
}

fn empty_metadata() -> ConfigAndEffectMetadata {
    ConfigAndEffectMetadata::default()
}

#[test]
fn package_test_artifact_writer_writes_assembly_dev_pointer_and_listing() {
    let temp = TestDir::new("skiff-compiler", "package-test-artifact-writer");
    let artifact_root = temp.path().join("artifacts");
    let production = compile_source_file_ir_artifact_for_test(
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        "api.skiff",
        "api",
        "package-production",
    )
    .expect("production file should compile");
    let test_file = compile_source_file_ir_artifact_for_test(
        r#"
            function packageTestEntry() -> number {
                return 42
            }
        "#,
        "api.test.skiff",
        "api.__test",
        "package-test",
    )
    .expect("test file should compile");

    let written = write_package_test_artifact_root(TestPackageTestArtifactInput {
        artifact_root: artifact_root.clone(),
        package_id: "example.com/math".to_string(),
        package_version: "1.0.0".to_string(),
        package_dependencies: Vec::new(),
        production_package_unit: None,
        production_config_and_effect_metadata: empty_metadata(),
        package_test_config_and_effect_metadata: empty_metadata(),
        production_files: vec![production],
        dependency_packages: Vec::new(),
        test_files: vec![TestPackageTestFileIrArtifact {
            source_path: "api.test.skiff".to_string(),
            module_path: "api.__test".to_string(),
            file_ir: test_file.unit,
            explicit_const_type_annotations: Default::default(),
        }],
        entrypoints: vec![TestPackageTestEntrypointInput {
            display_name: "package entry runs".to_string(),
            source_path: "api.test.skiff".to_string(),
            module_path: "api.__test".to_string(),
            test_ordinal: 0,
            executable_index: 0,
            executable_local_id: "packageTestEntry".to_string(),
            symbol: Some("api.__test.packageTestEntry".to_string()),
            default_run: true,
            config_and_effect_metadata: empty_metadata(),
        }],
    })
    .expect("package test artifacts should write");

    assert!(artifact_root.join(&written.package_unit_path).is_file());
    assert!(artifact_root.join(&written.assembly_path).is_file());
    assert!(artifact_root.join(&written.dev_pointer_path).is_file());
    assert!(written
        .assembly_path
        .starts_with("assemblies/package-tests/example~com~~math/"));
    assert!(written
        .dev_pointer_path
        .starts_with("dev/package-tests/example~com~~math/"));
    assert_eq!(written.entrypoints.len(), 1);
    assert!(written.entrypoints[0]
        .entrypoint_local_id
        .starts_with("skiff-package-test-entrypoint-local-v1:sha256:"));
    assert!(written.entrypoints[0]
        .entrypoint_id
        .starts_with("skiff-package-test-entrypoint-v1:sha256:"));

    let assembly: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join(&written.assembly_path)).unwrap(),
    )
    .unwrap();
    let test_file_path = assembly["testFiles"][0]["fileIrPath"]
        .as_str()
        .expect("test file ref should include path");
    assert!(test_file_path.starts_with("units/files/"));
    assert!(artifact_root.join(test_file_path).is_file());
    assert_eq!(
        assembly["productionPackageUnit"]["unitPath"],
        written.package_unit_path
    );
    assert_eq!(
        assembly["testEntrypoints"][0]["entrypointId"],
        written.entrypoints[0].entrypoint_id
    );
    let scope_entrypoints = assembly["linkPolicy"]["testFileScopes"][0]["entrypointLocalIds"]
        .as_array()
        .expect("test file scope must list entrypoint local ids");
    assert_eq!(scope_entrypoints.len(), 1);
    assert_eq!(
        scope_entrypoints[0],
        written.entrypoints[0].entrypoint_local_id
    );

    let listed =
        list_package_test_assemblies(&artifact_root, "example.com/math").expect("list pointers");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].test_build_identity, written.test_build_identity);
    assert_eq!(
        listed[0].package_test_assembly.assembly_path,
        written.assembly_path
    );
}

#[test]
fn package_test_artifact_writer_keeps_production_identity_when_test_only_file_changes() {
    let temp = TestDir::new(
        "skiff-compiler",
        "package-test-artifact-production-identity",
    );
    let artifact_root = temp.path().join("artifacts");
    let production = compile_source_file_ir_artifact_for_test(
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        "api.skiff",
        "api",
        "package-production",
    )
    .expect("production file should compile");

    let write_with_test_body = |source: &str| {
        let test_file = compile_source_file_ir_artifact_for_test(
            source,
            "api.test.skiff",
            "api.__test",
            "package-test",
        )
        .expect("test file should compile");
        write_package_test_artifact_root(TestPackageTestArtifactInput {
            artifact_root: artifact_root.clone(),
            package_id: "example.com/math".to_string(),
            package_version: "1.0.0".to_string(),
            package_dependencies: Vec::new(),
            production_package_unit: None,
            production_config_and_effect_metadata: empty_metadata(),
            package_test_config_and_effect_metadata: empty_metadata(),
            production_files: vec![production.clone()],
            dependency_packages: Vec::new(),
            test_files: vec![TestPackageTestFileIrArtifact {
                source_path: "api.test.skiff".to_string(),
                module_path: "api.__test".to_string(),
                file_ir: test_file.unit,
                explicit_const_type_annotations: Default::default(),
            }],
            entrypoints: vec![TestPackageTestEntrypointInput {
                display_name: "package entry runs".to_string(),
                source_path: "api.test.skiff".to_string(),
                module_path: "api.__test".to_string(),
                test_ordinal: 0,
                executable_index: 0,
                executable_local_id: "packageTestEntry".to_string(),
                symbol: Some("api.__test.packageTestEntry".to_string()),
                default_run: true,
                config_and_effect_metadata: empty_metadata(),
            }],
        })
        .expect("package test artifacts should write")
    };
    let first = write_with_test_body(
        r#"
            function packageTestEntry() -> number {
                return 42
            }
        "#,
    );
    let second = write_with_test_body(
        r#"
            function packageTestEntry() -> number {
                return 43
            }
        "#,
    );

    assert_eq!(
        first.package_unit_path, second.package_unit_path,
        "test-only file changes must not rewrite production PackageUnit identity/path"
    );
    assert_ne!(
        first.test_build_identity, second.test_build_identity,
        "test-only file changes should produce a new package test build"
    );
    let production_unit: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(artifact_root.join(&first.package_unit_path)).unwrap(),
    )
    .unwrap();
    let production_files = production_unit["files"].as_array().unwrap();
    assert_eq!(production_files.len(), 1);
    assert_eq!(production_files[0]["modulePath"], "api");
    assert!(!serde_json::to_string(&production_unit)
        .unwrap()
        .contains("api.test.skiff"));
}

#[test]
fn cli_test_package_test_file_uses_exported_package_api() {
    let package = math_project_with_api_yml("package-test-exported", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function secretOffset() -> number {
                return 40
            }

            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
        )
        .write_source(
            "api.test.skiff",
            r#"
            test "package test uses exported api" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        );

    let summary = run_tests(&package.path("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package test uses exported api");
}

#[test]
fn cli_test_package_test_file_import_prefix_does_not_authorize_exported_module() {
    let package = math_project_with_api_yml("package-test-import-prefix", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api.test.skiff",
            r#"
            import api

            test "package prefix import is not enough" {
                assert api.publicAnswer() == 42
            }
        "#,
        );

    assert_failed_result_or_error(
        &package.path("api.test.skiff"),
        "package prefix import is not enough",
        &["production function api.publicAnswer must be imported before use"],
    );
}

#[test]
fn cli_test_package_test_file_root_reference_authorizes_exported_module() {
    let package =
        math_project_with_api_yml("package-test-root-public-api", public_answer_api_yml())
            .write_source(
                "api.skiff",
                r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
            )
            .write_source(
                "api.test.skiff",
                r#"
            import api

            test "package root reference authorizes exported api" {
                assert root.api.publicAnswer() == 42
            }
        "#,
            );

    let summary = run_tests(&package.path("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package root reference authorizes exported api");
}

#[test]
fn cli_test_package_test_file_cannot_import_self_public_root() {
    let package = math_project_with_api_yml(
        "package-test-self-public-root-import",
        public_answer_api_yml(),
    )
    .write_source(
        "api.skiff",
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .write_source(
        "api.test.skiff",
        r#"
            import api

            test "package self public root import is rejected" {
                assert api.publicAnswer() == 42
            }
        "#,
    );

    assert_failed_result_or_error(
        &package.path("api.test.skiff"),
        "package self public root import is rejected",
        &["unresolved root api", "api.publicAnswer"],
    );
}

#[test]
fn cli_test_package_test_file_unqualified_symbol_import_is_not_runtime_bound() {
    let package = math_project_with_api_yml(
        "package-test-symbol-import-runtime",
        public_answer_api_yml(),
    )
    .write_source(
        "api.skiff",
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .write_source(
        "api.test.skiff",
        r#"
            test "symbol import does not create runtime local binding" {
                assert publicAnswer() == 42
            }
        "#,
    );

    let summary = run_tests(&package.path("api.test.skiff"));
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "symbol import does not create runtime local binding",
        "production function api.publicAnswer must be accessed through an imported module",
    );
}
#[test]
fn cli_test_package_test_helper_does_not_override_production_private_helper() {
    let package = math_project_with_api_yml("package-test-helper-shadow", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function secretOffset() -> number {
                return 40
            }

            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
        )
        .write_source(
            "api.test.skiff",
            r#"
            function secretOffset() -> number {
                return 0
            }

            test "test helper does not override production private helper" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        );

    let summary = run_tests(&package.path("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "test helper does not override production private helper",
    );
}
#[test]
fn cli_test_package_test_helper_does_not_satisfy_missing_production_helper() {
    let package = math_project_with_api_yml(
        "package-test-helper-missing-shadow",
        public_answer_api_yml(),
    )
    .write_source(
        "api.skiff",
        r#"
            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
    )
    .write_source(
        "api.test.skiff",
        r#"
            function secretOffset() -> number {
                return 40
            }

            test "test helper does not satisfy missing production helper" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    );

    assert_failed_result_or_error(
        &package.path("api.test.skiff"),
        "test helper does not satisfy missing production helper",
        &["function call callee `secretOffset` is not resolved by the File IR unit emitter"],
    );
}

#[test]
fn cli_test_package_test_helper_module_is_not_visible_to_production_code() {
    let temp = TestDir::new("skiff-compiler", "package-test-helper-module-private");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return root.api.__test.secretOffset() + 2
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            function secretOffset() -> number {
                return 40
            }

            test "production cannot call test helper module" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    assert_failed_result_or_error(
        &package_dir.join("api.test.skiff"),
        "production cannot call test helper module",
        &["invalid root reference"],
    );
}

#[test]
fn cli_test_package_test_helper_is_not_package_wide_root_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-helper-cross-file-root");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            function fileOnlyHelper() -> number {
                return 40
            }

            test "owner test can use local helper" {
                assert fileOnlyHelper() == 40
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("consumer.test.skiff"),
        r#"
            test "other test cannot use helper through root" {
                assert root.api.__test.fileOnlyHelper() == 40
            }
        "#,
    )
    .unwrap();

    assert_failed_result_or_error(
        &package_dir,
        "other test cannot use helper through root",
        &["invalid root reference", "api.__test.fileOnlyHelper"],
    );
}

#[test]
fn cli_test_package_production_code_cannot_call_embedded_test_entry() {
    let temp = TestDir::new("skiff-compiler", "package-embedded-test-entry-private");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    let source_file = package_dir.join("api.skiff");
    fs::write(
        &source_file,
        r#"
            function publicAnswer() -> number {
                return __skiff_test_0()
            }

            test "production cannot call embedded test entry" {
                assert publicAnswer() == 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);

    assert_failed_result_or_error(
        &source_file,
        "production cannot call embedded test entry",
        &["production source cannot contain test declarations"],
    );
}

#[test]
fn cli_test_package_test_import_does_not_satisfy_missing_manifest_dependency() {
    let temp = TestDir::new("skiff-compiler", "package-test-import-missing-dep");
    let package_dir = temp.path().join("pkg");
    let dependency_dir = package_store_path(
        &package_dir.join(".skiff-packages"),
        "example.com/secret",
        "1.0.0",
    );
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/main
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return root.api.secret()
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        dependency_dir.join("package.yml"),
        r#"
id: example.com/secret
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        dependency_dir.join("api.skiff"),
        r#"
            function secret() -> number {
                return 7
            }
        "#,
    )
    .unwrap();
    write_api_surface(&dependency_dir, "api", &["secret"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "test import does not add production dependency" {
                assert root.api.publicAnswer() == 7
            }
        "#,
    )
    .unwrap();

    assert_failed_result_or_error(
        &package_dir.join("api.test.skiff"),
        "test import does not add production dependency",
        &["invalid root reference"],
    );
}

#[test]
fn cli_test_package_export_can_use_short_source_file_path() {
    let temp = TestDir::new("skiff-compiler", "package-test-short-export-path");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "short export path imports package module" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "short export path imports package module");
}

#[test]
fn cli_test_package_flatten_export_uses_package_public_root() {
    let temp = TestDir::new("skiff-compiler", "package-test-flatten-root");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "flatten export imports package root" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "flatten export imports package root");
}

#[test]
fn cli_test_package_friend_test_file_can_access_production_private_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-friend-private");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function secretOffset() -> number {
                return 40
            }

            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            function helper() -> number {
                return root.api.secretOffset()
            }

            test "package friend test can call private api" {
                assert root.api.secretOffset() == 40
                assert helper() == 40
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package friend test can call private api");
}

#[test]
fn cli_test_package_integration_test_file_can_access_current_package_private_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-integration-private");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(package_dir.join("integration_tests")).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function secretOffset() -> number {
                return 40
            }

            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("integration_tests").join("api.test.skiff"),
        r#"
            function helperLeak() -> number {
                return root.api.secretOffset()
            }

            test "package integration test can call private api" {
                assert helperLeak() == 40
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("integration_tests").join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package integration test can call private api");
}

#[test]
fn cli_test_package_friend_test_const_does_not_hide_private_root_call() {
    let temp = TestDir::new("skiff-compiler", "package-test-friend-const-private-call");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function secretOffset() -> number {
                return 40
            }

            function publicAnswer() -> number {
                return secretOffset() + 2
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package friend test local const can coexist with private root call" {
                const secretOffset = 1
                assert root.api.secretOffset() == 40
                assert secretOffset == 1
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "package friend test local const can coexist with private root call",
    );
}

#[test]
fn cli_test_package_test_file_cannot_access_dependency_private_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-dependency-private");
    let package_dir = temp.path().join("pkg");
    let dependency_dir = package_store_path(
        &package_dir.join(".skiff-packages"),
        "example.com/secret",
        "1.0.0",
    );
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/main
version: 1.0.0
packages:
    - id: example.com/secret
      version: 1.0.0
      alias: depsecret
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("main.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_main_surface(&package_dir, &["publicAnswer"]);
    fs::write(
        dependency_dir.join("package.yml"),
        r#"
id: example.com/secret
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        dependency_dir.join("api.skiff"),
        r#"
            function hidden() -> number {
                return 7
            }

            function visible() -> number {
                return hidden()
            }
        "#,
    )
    .unwrap();
    write_api_surface(&dependency_dir, "api", &["visible"]);
    fs::write(
        package_dir.join("main.test.skiff"),
        r#"
            import depsecret

            test "package test cannot call dependency private api" {
                assert depsecret.hidden() == 7
            }
        "#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(
        &package_dir.join("main.test.skiff"),
        vec![package_dir.join(".skiff-packages")],
    );
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "package test cannot call dependency private api",
        "package dependency `depsecret` does not export public operation `hidden`",
    );
}

#[test]
fn cli_test_package_test_uses_internal_production_module_through_exported_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-internal-source");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/split
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return root.internal.secret() + 35
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("internal.skiff"),
        r#"
            function secret() -> number {
                return 7
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package test links internal production module" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package test links internal production module");
}

#[test]
fn cli_test_package_friend_test_does_not_blackbox_scan_production_body() {
    let temp = TestDir::new("skiff-compiler", "package-friend-test-internal-source");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/split
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return root.internal.secret() + 35
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("internal.skiff"),
        r#"
            function secret() -> number {
                return 7
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "friend test can exercise production internal module through exported api" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "friend test can exercise production internal module through exported api",
    );
}

#[test]
fn cli_test_package_duplicate_test_names_run_distinct_bodies() {
    let temp = TestDir::new("skiff-compiler", "package-duplicate-test-names");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "duplicate name" {
                assert root.api.publicAnswer() == 42
            }

            test "duplicate name" {
                assert false, "second duplicate body should run"
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 1);
    assert_eq!(matching_results(&summary, "duplicate name").len(), 2);
    assert!(matching_results(&summary, "duplicate name")
        .iter()
        .any(|result| result.passed && !result.skipped));
    assert_failed(
        &summary,
        "duplicate name",
        "second duplicate body should run",
    );
}

#[test]
fn cli_test_package_binding_provider_uses_api_yml_public_interface_selector() {
    let temp = TestDir::new("skiff-compiler", "package-binding-provider-api-yml");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/bindings
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            interface ManagedClient {
                function send(self: Self, input: string) -> string
            }

            function callManaged(input: string, managed: any ManagedClient) -> string {
                return managed.send(input)
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["ManagedClient", "callManaged"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            type TestClient implements root.api.ManagedClient {}

            const managed: TestClient = TestClient {}

            impl TestClient {
                function send(self: TestClient, input: string) -> string {
                    return input
                }
            }

            test "package binding provider uses api yml selector" {
                assert root.api.callManaged("from-provider", managed as root.api.ManagedClient) == "from-provider"
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package binding provider uses api yml selector");
}

#[test]
fn cli_test_package_test_file_can_access_unexported_module_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-internal-private");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/split
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("internal.skiff"),
        r#"
            function secret() -> number {
                return 7
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package test can call unexported module api" {
                assert root.internal.secret() == 7
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package test can call unexported module api");
}

#[test]
fn cli_test_package_integration_test_file_can_use_private_production_type() {
    let temp = TestDir::new("skiff-compiler", "package-integration-test-private-type");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(package_dir.join("integration_tests")).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            type SecretBox {
                value: number,
            }

            function publicAnswer() -> number {
                const box = SecretBox { value: 42 }
                return box.value
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("integration_tests").join("api.test.skiff"),
        r#"
            function leak() -> root.api.SecretBox {
                return root.api.SecretBox { value: 40 }
            }

            test "package integration test can use private type" {
                assert root.api.publicAnswer() == 42
                assert leak().value == 40
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("integration_tests").join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package integration test can use private type");
}

#[test]
fn cli_test_package_integration_test_file_can_use_private_type_inside_anonymous_record_type() {
    let temp = TestDir::new(
        "skiff-compiler",
        "package-integration-test-private-record-type",
    );
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(package_dir.join("integration_tests")).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            type SecretBox {
                value: number,
            }

            function publicAnswer() -> number {
                const box = SecretBox { value: 42 }
                return box.value
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("integration_tests").join("api.test.skiff"),
        r#"
            function leak() -> { box: root.api.SecretBox } {
                return { box: root.api.SecretBox { value: 40 } }
            }

            test "package integration test can use private type in record type" {
                assert root.api.publicAnswer() == 42
                assert leak().box.value == 40
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("integration_tests").join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "package integration test can use private type in record type",
    );
}

#[test]
fn cli_test_package_test_file_must_import_exported_production_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-missing-import");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    write_api_surface(&package_dir, "api", &["publicAnswer"]);
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package test must import exported api" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package test must import exported api");
}

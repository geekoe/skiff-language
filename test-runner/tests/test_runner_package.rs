use std::{
    fs,
    path::{Path, PathBuf},
};

mod common;
use common::{
    test_runner::{
        assert_counts, assert_failed, assert_not_run, assert_passed, run_tests, run_tests_error,
        run_tests_with_package_dirs,
    },
    TestDir,
};
use skiff_compiler::{test_support::project_fixtures::PackageProjectBuilder, PublicationId};

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

fn write_api_yml(root: &Path, api_yml: &str) {
    fs::write(root.join("api.yml"), api_yml).unwrap();
}

#[test]
fn cli_test_service_uses_package_dependency_alias() {
    let temp = TestDir::new("skiff-compiler", "service-package-alias-test");
    let service_dir = temp.path().join("service");
    let packages_dir = temp.path().join("package-store");
    let cloud_dir = package_store_path(&packages_dir, "google.com/cloud", "1.0.0");
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::create_dir_all(service_dir.join("internal")).unwrap();
    fs::create_dir_all(cloud_dir.join("cloud")).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
packages:
  - id: google.com/cloud
    version: 1.0.0
    alias: gcloud
"#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("example.skiff"),
        r#"
            interface ExampleService {
                function run() -> string
            }
"#,
    )
    .unwrap();
    fs::write(
        service_dir.join("internal").join("example.skiff"),
        r#"
            import gcloud

            function runAlias() -> string {
                return gcloud.storage.upload()
            }

            type ExampleImpl implements root.api.example.ExampleService {}
            impl ExampleImpl {
              function run(self: ExampleImpl) -> string {
                return runAlias()
              }
            }

"#,
    )
    .unwrap();
    fs::write(
        service_dir.join("internal").join("example.test.skiff"),
        r#"
            test "service package alias call works" {
                assert root.internal.example.runAlias() == "ok"
            }
"#,
    )
    .unwrap();
    fs::write(
        cloud_dir.join("package.yml"),
        r#"
id: google.com/cloud
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&cloud_dir, "storage: { upload: cloud.storage.upload }\n");
    fs::write(
        cloud_dir.join("cloud").join("storage.skiff"),
        r#"
            function upload() -> string {
                return "ok"
            }
"#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(&service_dir, vec![packages_dir]);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "service package alias call works");
}

#[test]
fn cli_test_package_uses_dependency_alias() {
    let temp = TestDir::new("skiff-compiler", "package-alias-test");
    let package_dir = temp.path().join("pkg");
    let packages_dir = temp.path().join("package-store");
    let cloud_dir = package_store_path(&packages_dir, "google.com/cloud", "1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(cloud_dir.join("cloud")).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/facade
version: 1.0.0
packages:
    - id: google.com/cloud
      version: 1.0.0
      alias: gcloud
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "facade: { facade: facade.facade }\n");
    fs::write(
        package_dir.join("facade.skiff"),
        r#"
            import gcloud

            function facade() -> string {
                return gcloud.storage.upload()
            }

"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("facade.test.skiff"),
        r#"
            test "package dependency alias call works" {
                assert root.facade.facade() == "ok"
            }
"#,
    )
    .unwrap();
    fs::write(
        cloud_dir.join("package.yml"),
        r#"
id: google.com/cloud
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&cloud_dir, "storage: { upload: cloud.storage.upload }\n");
    fs::write(
        cloud_dir.join("cloud").join("storage.skiff"),
        r#"
            function upload() -> string {
                return "ok"
            }
"#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(&package_dir, vec![packages_dir]);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package dependency alias call works");
}

#[test]
fn cli_test_package_links_url_like_std_dependency_alias() {
    let project = PackageProjectBuilder::new("package-url-like-std-link")
        .write_root_file(
            "package.yml",
            r#"
id: example.com/usesstd
version: 1.0.0
"#,
        )
        .write_root_file("api.yml", "api: { makeHeader: api.makeHeader }\n")
        .write_source(
            "api.skiff",
            r#"
            import std

            function makeHeader() -> std.http.HttpHeader {
                return std.http.HttpHeader { name: "x-test", value: "ok" }
            }
"#,
        )
        .write_source(
            "api.test.skiff",
            r#"
            test "package URL-like std dependency links" {
                const header = root.api.makeHeader()
                assert header.name == "x-test"
                assert header.value == "ok"
            }
"#,
        );

    let summary = run_tests(project.root());
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package URL-like std dependency links");
}

#[test]
fn cli_test_package_test_file_can_import_manifest_dependency_alias_public_api() {
    let temp = TestDir::new("skiff-compiler", "package-test-dependency-alias-public-api");
    let package_dir = temp.path().join("pkg");
    let packages_dir = temp.path().join("package-store");
    let dependency_dir = package_store_path(&packages_dir, "example.com/deplib", "1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/facade
version: 1.0.0
packages:
    - id: example.com/deplib
      version: 1.0.0
      alias: deplib
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "facade: { facade: facade.facade }\n");
    fs::write(
        package_dir.join("facade.skiff"),
        r#"
            function facade() -> string {
                return "facade"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("facade.test.skiff"),
        r#"
            import deplib

            test "package test dependency alias public api import works" {
                assert deplib.depapi.answer.value() == "dep"
                assert root.facade.facade() == "facade"
            }
"#,
    )
    .unwrap();
    fs::write(
        dependency_dir.join("package.yml"),
        r#"
id: example.com/deplib
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(
        &dependency_dir,
        "depapi: { answer: { value: internal.answer } }\n",
    );
    fs::write(
        dependency_dir.join("internal.skiff"),
        r#"
            function answer() -> string {
                return "dep"
            }
"#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(&package_dir, vec![packages_dir]);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "package test dependency alias public api import works",
    );
}

#[test]
fn cli_test_package_reads_per_run_config_require_optional_and_has() {
    let temp = TestDir::new("skiff-compiler", "package-test-config");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/configpkg
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "api: { readSecret: api.readSecret }\n");
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function readSecret() -> string {
                return config.require<string>("app.secret")
            }

            function readOptional() -> string? {
                return config.optional<string>("app.optional")
            }

            function hasSecret() -> boolean {
                return config.has("app.secret")
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package per-run config works" {
                assert root.api.readSecret() == "service-secret"
                assert root.api.readOptional() == "optional-value"
                assert root.api.hasSecret() == true
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "config": {
    "app": {
      "secret": "service-secret",
      "optional": "optional-value"
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&package_dir);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package per-run config works");
}

#[test]
fn cli_test_package_file_batch_keeps_per_test_config_shape_independent() {
    let temp = TestDir::new("skiff-compiler", "package-test-batched-config-isolation");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/batchedconfig
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "api: { marker: api.marker }\n");
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function marker() -> string {
                return "ok"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "first package config is isolated" {
                assert config.require<string>("first.secret") == "first"
            }

            test "second package config is isolated" {
                assert config.require<string>("second.secret") == "second"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "configs": {
    "api.__test::first package config is isolated": {
      "first": {
        "secret": "first"
      }
    },
    "api.__test::second package config is isolated": {
      "second": {
        "secret": "second"
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&package_dir);
    assert_counts(&summary, 2, 0, 0);
    assert_passed(&summary, "first package config is isolated");
    assert_passed(&summary, "second package config is isolated");
}

#[test]
fn cli_test_package_missing_required_config_fails() {
    let temp = TestDir::new("skiff-compiler", "package-test-missing-config");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/missingconfig
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "api: { readSecret: api.readSecret }\n");
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function readSecret() -> string {
                return config.require<string>("app.secret")
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package missing required config fails" {
                assert root.api.readSecret() == "unused"
            }
"#,
    )
    .unwrap();

    let summary = run_tests(&package_dir);
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "package missing required config fails",
        "app.secret",
    );
}

#[test]
fn cli_test_package_dependency_config_overlay_uses_alias() {
    let temp = TestDir::new("skiff-compiler", "package-test-dependency-config");
    let package_dir = temp.path().join("pkg");
    let packages_dir = temp.path().join("package-store");
    let dependency_dir = package_store_path(&packages_dir, "example.com/deplib", "1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(&dependency_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/facade
version: 1.0.0
packages:
    - id: example.com/deplib
      version: 1.0.0
      alias: deplib
"#,
    )
    .unwrap();
    write_api_yml(
        &package_dir,
        "facade: { readDepSecret: facade.readDepSecret }\n",
    );
    fs::write(
        package_dir.join("facade.skiff"),
        r#"
            import deplib

            function readDepSecret() -> string {
                return deplib.depapi.readSecret()
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("facade.test.skiff"),
        r#"
            test "package dependency config overlay works" {
                assert root.facade.readDepSecret() == "dep-secret"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "config": {
    "packages": {
      "deplib": {
        "dep": {
          "secret": "dep-secret"
        }
      }
    }
  }
}
"#,
    )
    .unwrap();
    fs::write(
        dependency_dir.join("package.yml"),
        r#"
id: example.com/deplib
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&dependency_dir, "depapi: { readSecret: dep.readSecret }\n");
    fs::write(
        dependency_dir.join("dep.skiff"),
        r#"
            function readSecret() -> string {
                return config.require<string>("dep.secret")
            }
"#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(&package_dir, vec![packages_dir]);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package dependency config overlay works");
}

#[test]
fn cli_test_package_dependency_config_unknown_alias_fails() {
    let temp = TestDir::new("skiff-compiler", "package-test-dependency-config-unknown");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/facade
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, "facade: { facade: facade.facade }\n");
    fs::write(
        package_dir.join("facade.skiff"),
        r#"
            function facade() -> string {
                return "facade"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("facade.test.skiff"),
        r#"
            test "package dependency config unknown alias fails" {
                assert root.facade.facade() == "facade"
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "config": {
    "packages": {
      "unknown": {}
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&package_dir);
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "package dependency config unknown alias fails",
        "packages.unknown",
    );
}

#[test]
fn cli_test_package_production_source_scan_skips_dependency_dir() {
    let temp = TestDir::new("skiff-compiler", "package-test-skip-dependencies");
    let package_dir = temp.path().join("pkg");
    let dependency_dir = package_store_path(
        &package_dir.join(".skiff-packages"),
        "example.com/deplib",
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
    - id: example.com/deplib
      version: 1.0.0
      alias: deplib
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, public_answer_api_yml());
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
"#,
    )
    .unwrap();
    fs::write(
        dependency_dir.join("package.yml"),
        r#"
id: example.com/deplib
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(
        &dependency_dir,
        "api: { dependencyAnswer: api.dependencyAnswer }\n",
    );
    fs::write(
        dependency_dir.join("api.skiff"),
        r#"
            import std

            function dependencyAnswer() -> number {
                return 7
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package source scan skips dependency directory" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(
        &package_dir.join("api.test.skiff"),
        vec![package_dir.join(".skiff-packages")],
    );
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package source scan skips dependency directory");
}

#[test]
fn cli_test_explicit_package_source_file_runs_friend_test_files() {
    let project = PackageProjectBuilder::new("package-test-source-plus-sibling")
        .with_manifest("example.com/math", public_answer_api_yml())
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
            test "sibling test-only file" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        )
        .write_source(
            "api.live.test.skiff",
            r#"
            test "sibling live test-only file" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        );
    let source_file = project.path("api.skiff");

    let summary = run_tests(&source_file);
    assert_counts(&summary, 2, 0, 0);
    assert_passed(&summary, "sibling test-only file");
    assert_passed(&summary, "sibling live test-only file");
}

#[test]
fn cli_test_explicit_package_test_file_runs_default_run_false() {
    let project = PackageProjectBuilder::new("package-test-explicit-default-run")
        .with_manifest("example.com/math", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api.live.test.skiff",
            r#"
            test defaultRun false

            test "explicit package default-run false test" {
                assert true
            }
        "#,
        );
    let explicit_test = project.path("api.live.test.skiff");

    let summary = run_tests(&explicit_test);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "explicit package default-run false test");
}

#[test]
fn cli_test_package_directory_skips_default_run_false_files() {
    let project = PackageProjectBuilder::new("package-test-default-run")
        .with_manifest("example.com/math", public_answer_api_yml())
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
            test "fast package test" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        )
        .write_source(
            "api.live.test.skiff",
            r#"
            test defaultRun false

            test "explicit package live" {
                assert false, "package live should only run when explicitly selected"
            }
        "#,
        );
    let slow_test = project.path("api.live.test.skiff");

    let directory_summary = run_tests(project.root());
    assert_counts(&directory_summary, 1, 0, 0);
    assert_passed(&directory_summary, "fast package test");
    assert_not_run(&directory_summary, "explicit package live");

    let file_summary = run_tests(&slow_test);
    assert_counts(&file_summary, 0, 0, 1);
    assert_failed(
        &file_summary,
        "explicit package live",
        "package live should only run when explicitly selected",
    );
}

#[test]
fn cli_test_package_directory_rejects_production_embedded_test() {
    let project = PackageProjectBuilder::new("package-test-production-embedded")
        .with_manifest("example.com/math", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }

            test "production embedded test is forbidden" {
                assert true
            }
        "#,
        );

    let error = run_tests_error(project.root());
    assert!(error.contains("production"));
    assert!(error.contains("test"));
}

#[test]
fn cli_test_package_live_friend_uses_test_module_identity() {
    let project = PackageProjectBuilder::new("package-test-live-module-identity")
        .with_manifest("example.com/math", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api.live.test.skiff",
            r#"
            test "live friend module identity" {
                assert root.api.publicAnswer() == 42
            }
        "#,
        );

    let summary = run_tests(&project.path("api.live.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_eq!(summary.results[0].module_path, "api.__test");
    assert_ne!(summary.results[0].module_path, "api.live");
    assert_passed(&summary, "live friend module identity");
}

#[test]
fn cli_test_package_non_exported_live_friend_uses_production_module_identity() {
    let project = PackageProjectBuilder::new("package-test-non-exported-live-module-identity")
        .with_manifest("example.com/math", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "internal.skiff",
            r#"
            function privateHelper() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "internal.live.test.skiff",
            r#"
            test "non-exported live friend module identity" {
                assert root.internal.privateHelper() == 42
            }
        "#,
        );

    let summary = run_tests(&project.path("internal.live.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_eq!(summary.results[0].module_path, "internal.__test");
    assert_ne!(summary.results[0].module_path, "internal.live.__test");
    assert_passed(&summary, "non-exported live friend module identity");
}

#[test]
fn cli_test_package_friend_file_name_must_match_unique_production_file() {
    let project = PackageProjectBuilder::new("package-test-friend-ambiguous")
        .with_manifest("example.com/math", public_answer_api_yml())
        .write_source(
            "api.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api.live.skiff",
            r#"
            function publicLiveAnswer() -> number {
                return 43
            }
        "#,
        )
        .write_source(
            "api.live.test.skiff",
            r#"
            test "ambiguous package friend test file" {
                assert true
            }
        "#,
        );

    let error = run_tests_error(project.root());
    assert!(error.contains("ambiguous friend test"));
    assert!(error.contains("api.skiff"));
    assert!(error.contains("api.live.skiff"));
}

#[test]
fn cli_test_package_directory_skips_dot_target_and_node_modules_dirs() {
    let temp = TestDir::new("skiff-compiler", "package-test-ignored-dirs");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(package_dir.join(".cache")).unwrap();
    fs::create_dir_all(package_dir.join("target")).unwrap();
    fs::create_dir_all(package_dir.join("node_modules").join("leftpad")).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(&package_dir, public_answer_api_yml());
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "package visible test" {
                assert true
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join(".cache").join("cached.test.skiff"),
        r#"
            test "dot directory test should be ignored" {
                assert false, "dot directory should not be discovered"
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join(".cache").join("cached.skiff"),
        "this is not valid production source",
    )
    .unwrap();
    fs::write(
        package_dir.join("target").join("generated.test.skiff"),
        r#"
            test "target directory test should be ignored" {
                assert false, "target directory should not be discovered"
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("target").join("generated.skiff"),
        "this is not valid production source",
    )
    .unwrap();
    fs::write(
        package_dir
            .join("node_modules")
            .join("leftpad")
            .join("dependency.test.skiff"),
        r#"
            test "node_modules directory test should be ignored" {
                assert false, "node_modules directory should not be discovered"
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir
            .join("node_modules")
            .join("leftpad")
            .join("dependency.skiff"),
        "this is not valid production source",
    )
    .unwrap();

    let summary = run_tests(&package_dir);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package visible test");
    assert_not_run(&summary, "dot directory test should be ignored");
    assert_not_run(&summary, "target directory test should be ignored");
    assert_not_run(&summary, "node_modules directory test should be ignored");
}

// Regression for defect 2 (§1.1): a package test that names a *dependency* type
// explicitly and reads one of its record fields. The old package compilation path
// fed empty dependency publications, so the dependency type's shape was invisible
// and the field access failed with `unknown field`.
#[test]
fn cli_test_package_test_reads_named_dependency_type_field() {
    let temp = TestDir::new("skiff-compiler", "package-named-dep-type-field");
    let package_dir = temp.path().join("pkg");
    let packages_dir = temp.path().join("package-store");
    let dep_dir = package_store_path(&packages_dir, "example.com/deplib", "1.0.0");
    fs::create_dir_all(&package_dir).unwrap();
    fs::create_dir_all(&dep_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/facade
version: 1.0.0
packages:
    - id: example.com/deplib
      version: 1.0.0
      alias: deplib
"#,
    )
    .unwrap();
    write_api_yml(
        &package_dir,
        "facade: { describe: facade.describe, makeInfo: facade.makeInfo }\n",
    );
    fs::write(
        package_dir.join("facade.skiff"),
        r#"
            import deplib

            function describe(info: deplib.PlainInfo) -> string {
                return info.label
            }

            function makeInfo() -> deplib.PlainInfo {
                return deplib.makeInfo()
            }
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("facade.test.skiff"),
        r#"
            test "package test reads named dependency type field" {
                const info = root.facade.makeInfo()
                assert root.facade.describe(info) == "answer"
            }
"#,
    )
    .unwrap();
    fs::write(
        dep_dir.join("package.yml"),
        r#"
id: example.com/deplib
version: 1.0.0
"#,
    )
    .unwrap();
    write_api_yml(
        &dep_dir,
        "{ PlainInfo: deplib.PlainInfo, makeInfo: deplib.makeInfo }\n",
    );
    fs::write(
        dep_dir.join("deplib.skiff"),
        r#"
            type PlainInfo {
                label: string
            }

            function makeInfo() -> PlainInfo {
                return PlainInfo { label: "answer" }
            }
"#,
    )
    .unwrap();

    let summary = run_tests_with_package_dirs(&package_dir, vec![packages_dir]);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "package test reads named dependency type field");
}

// Regression for defect 1 (§9.2): a package has a `-> Stream<Chunk>` producer and the
// test iterates the stream with `for event in root.<mod>.produce(...)` reading a field of
// the same-package stream element type. The flattened service path must resolve the stream
// element type so `event.value` keeps its type.
#[test]
fn cli_test_package_test_reads_same_package_stream_element_field() {
    let project = PackageProjectBuilder::new("package-stream-element-field")
        .with_manifest(
            "example.com/streamer",
            "streamer: { Chunk: streamer.Chunk, produce: streamer.produce }\n",
        )
        .write_source(
            "streamer.skiff",
            r#"
            type Chunk {
                value: string
            }

            function produce() -> Stream<Chunk> {
                emit(Chunk { value: "hello" })
                emit(Chunk { value: "world" })
                return {}
            }
"#,
        )
        .write_source(
            "streamer.test.skiff",
            r#"
            test "package test reads same-package stream element field" {
                let text = ""
                for event in root.streamer.produce() {
                    text = text.concat(event.value)
                }
                assert text == "helloworld"
            }
"#,
        );

    let summary = run_tests(project.root());
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "package test reads same-package stream element field",
    );
}

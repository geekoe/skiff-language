mod common;
use common::test_runner::{
    assert_counts, assert_failed, assert_not_run, assert_passed, run_tests, run_tests_error,
};
use skiff_compiler::test_support::project_fixtures::ServiceProjectBuilder;
use std::{fs, path::Path};

#[test]
fn cli_test_directory_skips_default_run_false_files() {
    let project = ServiceProjectBuilder::new("test-default-run")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/fast.test.skiff",
            r#"
test "fast default test" {
                if true {
                    assert true
                }
                assert true
            }
        "#,
        )
        .write_source(
            "api/slow.test.skiff",
            r#"
            test defaultRun false

            test "explicit slow test" {
                assert false, "slow test should only run when explicitly selected"
            }
        "#,
        );
    let slow_test = project.path("api/slow.test.skiff");

    let directory_summary = run_tests(project.root());
    assert_counts(&directory_summary, 1, 0, 0);
    assert_passed(&directory_summary, "fast default test");
    assert_not_run(&directory_summary, "explicit slow test");

    let file_summary = run_tests(&slow_test);
    assert_counts(&file_summary, 0, 0, 1);
    assert_failed(
        &file_summary,
        "explicit slow test",
        "slow test should only run when explicitly selected",
    );
}

#[test]
fn cli_test_assert_rewrite_uses_public_std_decode_error() {
    let project = ServiceProjectBuilder::new("service-assert-rewrite")
        .with_default_manifest("example.com/assert")
        .write_source(
            "api/assert.test.skiff",
            r#"
            test "assert rewrite compiles" {
                assert true, "assertion should use a public std error type"
            }
        "#,
        );

    let summary = run_tests(project.root());
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "assert rewrite compiles");
}

#[test]
fn cli_test_service_directory_rejects_default_run_false_in_production_source() {
    let project = ServiceProjectBuilder::new("service-default-run-production-source")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/slow.skiff",
            r#"
            test defaultRun false

            function slowAnswer() -> number {
                return 42
            }
        "#,
        );

    let error = run_tests_error(project.root());
    assert!(error.contains("production"));
    assert!(error.contains("defaultRun"));
}

#[test]
fn cli_test_directory_discovers_only_test_files() {
    let project = ServiceProjectBuilder::new("service-directory-only-test-files")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/calc.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api/calc.test.skiff",
            r#"
            test "directory test file runs" {
                assert root.api.calc.publicAnswer() == 42
            }
        "#,
        );

    let summary = run_tests(project.root());
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "directory test file runs");
}

#[test]
fn cli_test_explicit_source_file_runs_friend_test_files() {
    let project = ServiceProjectBuilder::new("test-source-friends")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/calc.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api/calc.test.skiff",
            r#"
            test "sibling test file" {
                assert true
            }
        "#,
        )
        .write_source(
            "api/calc.live.test.skiff",
            r#"
            test "sibling live test file" {
                assert root.api.calc.publicAnswer() == 42
            }
        "#,
        );

    let summary = run_tests(&project.path("api/calc.skiff"));
    assert_counts(&summary, 2, 0, 0);
    assert_passed(&summary, "sibling test file");
    assert_passed(&summary, "sibling live test file");
}

#[test]
fn cli_test_explicit_source_file_skips_default_run_false_friend_by_default() {
    let project = ServiceProjectBuilder::new("test-source-friend-default-run")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/calc.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api/calc.test.skiff",
            r#"
            test "default friend test file" {
                assert root.api.calc.publicAnswer() == 42
            }
        "#,
        )
        .write_source(
            "api/calc.live.test.skiff",
            r#"
            test defaultRun false

            test "live friend test file" {
                assert false, "live friend should only run when explicitly selected"
            }
        "#,
        );

    let summary = run_tests(&project.path("api/calc.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "default friend test file");
    assert_not_run(&summary, "live friend test file");

    let explicit = run_tests(&project.path("api/calc.live.test.skiff"));
    assert_counts(&explicit, 0, 0, 1);
    assert_failed(
        &explicit,
        "live friend test file",
        "live friend should only run when explicitly selected",
    );
}

#[test]
fn cli_test_explicit_service_root_test_file_loads_whole_root() {
    let project = ServiceProjectBuilder::new("explicit-service-root-test")
        .with_default_manifest("example.com/calc")
        .write_root_file(
            "api.yml",
            "api: { example: { publicAnswer: api.example.publicAnswer } }\n",
        )
        .write_source(
            "api/example.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "internal.skiff",
            r#"
            function internalAnswer() -> number {
                return 7
            }
        "#,
        )
        .write_source(
            "api/example.live.test.skiff",
            r#"
            test defaultRun false

            test "explicit service root test sees all modules" {
                assert root.api.example.publicAnswer() == 42
                assert root.internal.internalAnswer() == 7
            }
        "#,
        )
        .write_source(
            "api/other.test.skiff",
            r#"
            test "unselected sibling test" {
                assert false, "explicit file run should not run sibling tests"
            }
        "#,
        );

    let summary = run_tests(&project.path("api/example.live.test.skiff"));
    assert_counts(&summary, 1, 0, 0);
    assert_passed(&summary, "explicit service root test sees all modules");
    assert_not_run(&summary, "unselected sibling test");
}

#[test]
fn cli_test_explicit_service_file_requires_service_root() {
    let project = ServiceProjectBuilder::new("service-file-requires-root").write_source(
        "api/calc.test.skiff",
        r#"
            test "standalone service file should not run" {
                assert true
            }
        "#,
    );

    let error = run_tests_error(&project.path("api/calc.test.skiff"));
    assert!(error.contains("service test file input"));
    assert!(error.contains("service.yml"));
}

#[test]
fn cli_test_service_directory_requires_service_root() {
    let project = ServiceProjectBuilder::new("service-directory-requires-root").write_source(
        "api/calc.test.skiff",
        r#"
            test "standalone service directory should not run" {
                assert true
            }
        "#,
    );

    let error = run_tests_error(project.root());
    assert!(error.contains("service test directory input"));
    assert!(error.contains("service.yml"));
}

#[test]
fn cli_test_service_friend_file_name_must_match_unique_production_file() {
    let project = ServiceProjectBuilder::new("test-source-friend-ambiguous")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/calc.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
        )
        .write_source(
            "api/calc.live.skiff",
            r#"
            function publicLiveAnswer() -> number {
                return 43
            }
        "#,
        )
        .write_source(
            "api/calc.live.test.skiff",
            r#"
            test "ambiguous friend test file" {
                assert true
            }
        "#,
        );

    let error = run_tests_error(project.root());
    assert!(error.contains("ambiguous friend test"));
    assert!(error.contains("calc.skiff"));
    assert!(error.contains("calc.live.skiff"));
}

#[test]
fn service_publish_uses_structured_service_unit_dispatch_shape() {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/service_publish.rs"))
            .expect("service_publish.rs should be readable");

    for forbidden in [
        "service_assembly",
        ".get(\"entrypoint\")",
        ".get(\"operationAbiId\")",
        "target.modulePath",
        "target.symbol",
    ] {
        assert!(
            !source.contains(forbidden),
            "service_publish.rs must not read legacy operation field {forbidden}"
        );
    }
    assert!(source.contains("serde_json::from_value(service_unit_value.clone())"));
    assert!(source.contains("source_call_operation_index"));
    assert!(source.contains("service_unit_operation_executable_target"));
    assert!(source.contains("throw std.json.DecodeError"));
    assert!(!source.contains("throw std.error"));
}

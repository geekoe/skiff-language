mod common;
use common::cli_command::{assert_failure, assert_success, stderr, CliCommand};
use serde_json::Value;
use skiff_compiler::{test_support::project_fixtures::ServiceProjectBuilder, PublicationId};

#[test]
fn cli_compile_rejects_production_file_with_embedded_test() {
    let project = ServiceProjectBuilder::new("compile-production-embedded-test")
        .with_default_manifest("calc")
        .write_source(
            "api/calc.skiff",
            r#"
            function publicAnswer() -> number {
                return 42
            }

            test "production file test is forbidden during compile" {
                assert true
            }
        "#,
        )
        .write_source(
            "api/calc.test.skiff",
            r#"
            function expectedSize() -> string {
                return "640x480"
            }

            test "test file uses test-only helper" {
                assert expectedSize() == "640x480", "test-only helper should be visible"
            }
        "#,
        );
    let artifact_path = project.temp_path().join("artifact.json");

    let output = CliCommand::compile(project.root())
        .arg("--out")
        .arg(&artifact_path)
        .output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(stderr.contains("production"));
    assert!(stderr.contains("test"));
    assert!(!stderr.contains("failed to parse"));
}

#[test]
fn cli_test_reports_runner_moved() {
    let project = ServiceProjectBuilder::new("test-production-default-run")
        .with_default_manifest("calc")
        .write_source(
            "api/calc.skiff",
            r#"
            test defaultRun false

            function publicAnswer() -> number {
                return 42
            }
        "#,
        );

    let output = CliCommand::test(project.root()).output();

    assert_failure(&output);
    let stderr = stderr(&output);
    assert!(stderr.contains("--test has moved out of skiff-compiler"));
    assert!(stderr.contains("skiff-test-runner"));
}

#[test]
fn cli_compile_accepts_url_like_service_id_override_with_safe_artifact_paths() {
    let project = ServiceProjectBuilder::new("compile-url-like-service-id")
        .with_default_manifest("example.com/calc")
        .write_source(
            "api/example.skiff",
            r#"
            type Input {}
            type Output {}
            interface ExampleService {
              function run(input: Input) -> Output
            }
        "#,
        )
        .write_source(
            "internal/example.skiff",
            r#"
            type ExampleService {}

            impl ExampleService {
              function run(self: ExampleService, input: root.api.example.Input) -> root.api.example.Output {
                return root.api.example.Output {}
              }
            }
        "#,
    );
    let artifact_path = project.temp_path().join("artifact.json");
    let artifact_root = project.temp_path().join("artifacts");

    let output = CliCommand::compile(project.root())
        .arg("--service-id")
        .arg("skiff.run/account")
        .arg("--out")
        .arg(&artifact_path)
        .arg("--artifact-root")
        .arg(&artifact_root)
        .output();

    assert_success(&output);
    let assembly: Value =
        serde_json::from_str(&std::fs::read_to_string(&artifact_path).unwrap()).unwrap();
    assert_eq!(assembly["service"]["id"], "skiff.run/account");
    let service_unit_dir = artifact_root.join("units/services/skiff~run~~account");
    assert!(service_unit_dir.is_dir());
    assert!(!artifact_root
        .join("units/services/skiff.run/account")
        .exists());
    let service_unit_paths = std::fs::read_dir(service_unit_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(service_unit_paths
        .iter()
        .any(|path| path.ends_with(".json") && !path.contains("%2F")));
    let service_target_component = PublicationId::parse("skiff.run/account")
        .unwrap()
        .runtime_target_component();
    let targets = assembly["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|operation| operation["entrypoint"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(targets
        .iter()
        .any(|target| target.starts_with(&format!("service.{service_target_component}."))));
    assert!(targets
        .iter()
        .all(|target| !target.contains("%2F") && !target.contains("skiff.run/account")));
}

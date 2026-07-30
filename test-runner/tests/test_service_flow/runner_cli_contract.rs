use std::{
    path::Path,
    process::{Command, Output},
};

use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_test_runner::{run_skiff_tests_with_options, SkiffTestError, SkiffTestOptions};

use super::*;

const RUNNER_ENVIRONMENT_KEYS: &[&str] = &[
    "SKIFF_TEST_ARTIFACT_ROOT",
    "SKIFF_TEST_RUNTIME_ARTIFACT_ROOT",
    "SKIFF_TEST_ACTIVATION_URL",
    "SKIFF_TEST_INGRESS_URL",
    "SKIFF_TEST_ENVIRONMENT",
    "SKIFF_TEST_EXPECTED_GENERATION",
];

#[test]
fn direct_runner_rejects_invalid_platform_context_and_redacts_urls() {
    let runner = env!("CARGO_BIN_EXE_skiff-test-runner");
    let platform_root = repository_root();
    let platform = platform_root.to_str().unwrap();
    let help = Command::new(runner).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).unwrap();
    for option in [
        "--artifact-root",
        "--platform-source-root",
        "--base-assembly",
        "--live",
        "--activation-url",
        "--ingress-url",
        "--environment",
        "--expected-generation",
        "--deny-skips",
        "--require-tests",
    ] {
        assert!(help.contains(option), "help omitted {option}");
    }
    for retired in [
        "--profile",
        "--test-config-literals",
        "--service-artifact-root",
        "--config",
        "--package-test-concurrency",
        "--router-reload-url",
        "--packages-dir",
        "--allow-network",
    ] {
        assert!(!help.contains(retired), "help retained {retired}");
        let rejected = run_runner(runner, ["input", retired], &[]);
        assert_failure_contains(&rejected, &format!("unknown option {retired}"));
    }
    let retired_artifact_root = run_runner(
        runner,
        ["input", "--platform-source-root", platform],
        &[("SKIFF_TEST_ARTIFACT_ROOT", "/retired/env-fallback")],
    );
    assert_failure_contains(&retired_artifact_root, "missing --artifact-root");

    let duplicate = run_runner(
        runner,
        [
            "missing-input",
            "--artifact-root",
            "/missing-artifacts",
            "--platform-source-root",
            platform,
            "--platform-source-root",
            platform,
        ],
        &[],
    );
    assert_failure_contains(
        &duplicate,
        "--platform-source-root was provided more than once",
    );

    let relative = run_runner(
        runner,
        [
            "missing-input",
            "--artifact-root",
            "/missing-artifacts",
            "--platform-source-root",
            "relative/platform",
        ],
        &[],
    );
    assert_failure_contains(&relative, "compiler platform source root must be absolute");

    let missing = run_runner(
        runner,
        ["missing-input", "--artifact-root", "/missing-artifacts"],
        &[],
    );
    assert_failure_contains(&missing, "missing --platform-source-root");

    let fixture = env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture");
    for (extra, expected) in [
        (
            vec![
                "--platform-source-root",
                platform,
                "--platform-source-root",
                platform,
            ],
            "--platform-source-root was provided more than once",
        ),
        (
            vec!["--platform-source-root", "relative/platform"],
            "compiler platform source root must be absolute",
        ),
        (Vec::new(), "missing --platform-source-root"),
    ] {
        let output = Command::new(fixture)
            .args([
                "--bootstrap-only",
                "--artifact-root",
                "/missing-artifacts",
                "--environment",
                "platform-context",
            ])
            .args(extra)
            .output()
            .unwrap();
        assert_failure_contains(&output, expected);
    }

    let sentinel = "runner-url-secret";
    for (option, value, expected) in [
        (
            "--activation-url",
            format!("http://user:{sentinel}@127.0.0.1:4001/__skiff/activate-assembly"),
            "activation URL must point exactly",
        ),
        (
            "--ingress-url",
            format!("http://127.0.0.1:4000/nested?token={sentinel}"),
            "ingress URL must be an http:// origin",
        ),
    ] {
        let output = run_runner(
            runner,
            [
                "missing-input",
                "--artifact-root",
                "/missing-artifacts",
                "--platform-source-root",
                platform,
                option,
                &value,
            ],
            &[],
        );
        let stderr = assert_failure_contains(&output, expected);
        assert!(!stderr.contains(sentinel), "{stderr}");
    }
}

#[test]
fn direct_runner_separates_live_cli_targets_from_non_live_harness_targets() {
    let runner = env!("CARGO_BIN_EXE_skiff-test-runner");
    let platform_root = repository_root();
    let platform = platform_root.to_str().unwrap();
    let base = [
        "missing-input",
        "--artifact-root",
        "/missing-artifacts",
        "--platform-source-root",
        platform,
    ];

    let incomplete_live = run_runner(runner, base.into_iter().chain(["--live"]), &[]);
    assert_failure_contains(
        &incomplete_live,
        "--live requires --activation-url, --ingress-url, --environment and --expected-generation",
    );

    let cli_target = run_runner(
        runner,
        base.into_iter().chain(["--environment", "dev"]),
        &[],
    );
    assert_failure_contains(
        &cli_target,
        "non-live targets are supplied only by the isolated runtime harness",
    );

    let harness_target = run_runner(
        runner,
        base,
        &[
            ("SKIFF_TEST_ENVIRONMENT", "dev"),
            (
                "SKIFF_TEST_ACTIVATION_URL",
                "http://127.0.0.1:9/__skiff/activate-assembly",
            ),
            ("SKIFF_TEST_INGRESS_URL", "http://127.0.0.1:9"),
            ("SKIFF_TEST_EXPECTED_GENERATION", "7"),
            ("SKIFF_TEST_RUNTIME_ARTIFACT_ROOT", "/missing-runtime"),
        ],
    );
    assert_failure_contains(&harness_target, "failed to inspect input");
}

#[test]
fn smoke_fixture_rejects_ambiguous_prepare_modes() {
    let fixture = env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture");
    for (args, expected) in [
        (
            vec![
                "consumer",
                "--prepare-host-base",
                "fixture",
                "--work-root",
                "work",
                "--receipt",
                "receipt.json",
            ],
            "--prepare-host-base is mutually exclusive",
        ),
        (
            vec!["--work-root", "work", "--receipt", "receipt.json"],
            "--work-root and --receipt require --prepare-host-base",
        ),
        (
            vec!["--prepare-host-base", "fixture", "--work-root", "work"],
            "--prepare-host-base requires --work-root and --receipt",
        ),
    ] {
        let output = Command::new(fixture)
            .args(args)
            .args([
                "--artifact-root",
                "artifacts",
                "--environment",
                "cli-contract",
            ])
            .output()
            .unwrap();
        assert_failure_contains(&output, expected);
    }
}

#[test]
fn canonical_execution_rejects_invalid_runtime_roots_and_ingress_before_network() {
    let root = TestRoot::new("runner-preflight");
    let artifacts = root.path().join("artifacts");
    let nested_runtime = artifacts.join("runtime-owned");
    let separate_runtime = root.path().join("runtime");
    let service = root.path().join("service");
    CanonicalArtifactStore::create(&artifacts).unwrap();
    fs::create_dir_all(&nested_runtime).unwrap();
    fs::create_dir_all(&separate_runtime).unwrap();
    write_minimal_test_service(&service);

    let options = |runtime_artifact_root, ingress_url| SkiffTestOptions {
        live: false,
        artifact_root: Some(artifacts.clone()),
        platform_sources: platform_sources(),
        runtime_artifact_root: Some(runtime_artifact_root),
        base_assembly: None,
        activation_url: Some("http://127.0.0.1:9/__skiff/activate-assembly".to_string()),
        ingress_url,
        target_environment: "runner-preflight".to_string(),
        expected_generation: 0,
    };

    let nested = run_skiff_tests_with_options(
        &service,
        &options(nested_runtime, Some("http://127.0.0.1:9".to_string())),
    )
    .unwrap_err();
    assert!(matches!(nested, SkiffTestError::MissingIsolatedRuntimeRoot));

    let missing = run_skiff_tests_with_options(&service, &options(separate_runtime.clone(), None))
        .unwrap_err();
    assert!(matches!(missing, SkiffTestError::MissingCanonicalRuntime));

    let invalid = run_skiff_tests_with_options(
        &service,
        &options(
            separate_runtime,
            Some("http://127.0.0.1:9/not-an-origin".to_string()),
        ),
    )
    .unwrap_err()
    .to_string();
    assert!(
        invalid.contains("ingress URL must be an http:// origin"),
        "{invalid}"
    );
}

fn run_runner<'a>(
    binary: &str,
    args: impl IntoIterator<Item = &'a str>,
    environment: &[(&str, &str)],
) -> Output {
    let mut command = Command::new(binary);
    command.args(args);
    for key in RUNNER_ENVIRONMENT_KEYS {
        command.env_remove(key);
    }
    for (key, value) in environment {
        command.env(key, value);
    }
    command.output().unwrap()
}

fn assert_failure_contains(output: &Output, expected: &str) -> String {
    assert!(!output.status.success(), "command unexpectedly succeeded");
    let stderr = String::from_utf8(output.stderr.clone()).unwrap();
    assert!(
        stderr.contains(expected),
        "expected {expected:?}, got {stderr}"
    );
    stderr
}

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
}

fn write_minimal_test_service(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("package.yml"),
        "id: test.skiff/runner-preflight\nversion: 1.0.0\n",
    )
    .unwrap();
    fs::write(root.join("api.yml"), "{}\n").unwrap();
    fs::write(
        root.join("service.yml"),
        "id: test.skiff/runner-preflight\nkind: test\n",
    )
    .unwrap();
    fs::write(root.join("config.skiff-test.yml"), "{}\n").unwrap();
    fs::write(
        root.join("main.test.skiff"),
        "test \"preflight\" { assert true }\n",
    )
    .unwrap();
}

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_test_runner::SkiffTestSummary;

use super::{parse_args, run, summary_policy_failures, CliError};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn parses_explicit_runtime_target_and_strict_result_flags() {
    let parsed = parse_args(
        [
            "fixture.skiff",
            "--router-reload-url=http://router.test:4101",
            "--artifact-root",
            "/tmp/artifacts",
            "--deny-skips",
            "--require-tests",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .expect("new runtime options should parse");

    assert_eq!(
        parsed.router_reload_url.as_deref(),
        Some("http://router.test:4101")
    );
    assert_eq!(parsed.artifact_root, Some(PathBuf::from("/tmp/artifacts")));
    assert!(parsed.deny_skips);
    assert!(parsed.require_tests);
}

#[test]
fn singleton_and_flag_duplicates_fail_closed_across_argument_forms() {
    for args in [
        vec![
            "fixture.skiff",
            "--router-reload-url",
            "http://router.test:4101",
            "--router-reload-url=http://other.test:4101",
        ],
        vec![
            "fixture.skiff",
            "--artifact-root=/tmp/one",
            "--artifact-root",
            "/tmp/two",
        ],
        vec!["fixture.skiff", "--config=one.json", "--config", "two.json"],
        vec!["fixture.skiff", "--deny-skips", "--deny-skips"],
        vec!["fixture.skiff", "--require-tests", "--require-tests"],
    ] {
        let error = parse_args(args.into_iter().map(str::to_string))
            .expect_err("duplicates must be rejected");
        assert!(matches!(error, CliError::Message(_)));
    }
}

#[test]
fn help_still_rejects_later_duplicate_and_unknown_options() {
    for args in [
        vec!["--help", "--help"],
        vec!["-h", "--help"],
        vec!["--help", "--unknown"],
    ] {
        let error = parse_args(args.into_iter().map(str::to_string))
            .expect_err("help must not bypass strict argument validation");
        assert!(matches!(error, CliError::Message(_)));
    }

    assert!(matches!(
        parse_args(["--help".to_string()]),
        Err(CliError::Help)
    ));
}

#[test]
fn explicit_artifact_root_must_be_a_directory_before_the_test_worker_starts() {
    let root = std::env::temp_dir().join(format!(
        "skiff-test-runner-cli-artifact-root-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let missing_root = root.join("missing-artifacts");
    let artifact_file = root.join("artifact-file");
    fs::write(&artifact_file, "not a directory\n").expect("artifact file should be written");

    for artifact_root in [missing_root, artifact_file] {
        let error = run([
            path_string(&root.join("input-that-must-not-be-opened.skiff")),
            "--artifact-root".to_string(),
            path_string(&artifact_root),
        ]
        .into_iter())
        .expect_err("invalid explicit artifact root must fail before the worker");
        match error {
            CliError::Message(message) => {
                assert!(message.contains("--artifact-root must be an existing directory"));
                assert!(message.contains(&path_string(&artifact_root)));
            }
            other => panic!("expected an artifact-root CLI error, got {other:?}"),
        }
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_reload_url_is_validated_and_redacted_before_the_test_worker_starts() {
    let root = std::env::temp_dir().join(format!(
        "skiff-test-runner-cli-reload-url-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).expect("fixture root should exist");
    let sentinel = "rust-cli-reload-secret-sentinel";
    let error = run([
        path_string(&root.join("input-that-must-not-be-opened.skiff")),
        "--router-reload-url".to_string(),
        format!("http://router.test:4101/?token={sentinel}"),
        "--artifact-root".to_string(),
        path_string(&root),
    ]
    .into_iter())
    .expect_err("invalid explicit reload URL must fail before the worker");
    match error {
        CliError::Message(message) => {
            assert!(message.contains("reload_url_query"));
            assert!(!message.contains(sentinel));
        }
        other => panic!("expected a reload URL CLI error, got {other:?}"),
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn strict_summary_policy_rejects_skips_and_empty_discovery() {
    let skipped = SkiffTestSummary {
        passed: 1,
        skipped: 1,
        failed: 0,
        results: Vec::new(),
    };
    assert!(summary_policy_failures(&skipped, false, false).is_empty());
    assert_eq!(summary_policy_failures(&skipped, true, true).len(), 1);

    let empty = SkiffTestSummary {
        passed: 0,
        skipped: 0,
        failed: 0,
        results: Vec::new(),
    };
    assert!(summary_policy_failures(&empty, false, false).is_empty());
    assert_eq!(summary_policy_failures(&empty, false, true).len(), 1);
}

#[test]
fn require_tests_rejects_a_live_named_fixture_without_test_cases() {
    let fixture = live_package_fixture("empty", "test defaultRun false\n");
    let result = run(live_cli_args(&fixture, ["--require-tests"]));
    assert!(matches!(result, Err(CliError::TestFailed)));
    let _ = fs::remove_dir_all(fixture.root);
}

#[test]
fn deny_skips_rejects_an_all_skip_live_summary() {
    let fixture = live_package_fixture(
        "skip",
        r#"
test defaultRun false

test "missing explicit live key" {
  const value = config.require<string>("provider.apiKey")
  assert value != ""
}
"#,
    );
    let result = run(live_cli_args(&fixture, ["--deny-skips"]));
    assert!(matches!(result, Err(CliError::TestFailed)));
    let _ = fs::remove_dir_all(fixture.root);
}

struct LivePackageFixture {
    root: PathBuf,
    test_file: PathBuf,
    config: PathBuf,
    artifact_root: PathBuf,
}

fn live_package_fixture(label: &str, test_source: &str) -> LivePackageFixture {
    let root = std::env::temp_dir().join(format!(
        "skiff-test-runner-cli-{label}-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let package = root.join("package");
    fs::create_dir_all(&package).expect("fixture package should exist");
    fs::write(
        package.join("package.yml"),
        "id: example.com/cli-policy\nversion: 1.0.0\n",
    )
    .expect("fixture package manifest should be written");
    fs::write(package.join("api.yml"), "api: { marker: api.marker }\n")
        .expect("fixture API manifest should be written");
    fs::write(
        package.join("api.skiff"),
        "function marker() -> bool { return true }\n",
    )
    .expect("fixture source should be written");
    let test_file = package.join("api.live.test.skiff");
    fs::write(&test_file, test_source).expect("fixture test should be written");
    let config = root.join("runtime-live.json");
    fs::write(&config, "{}\n").expect("fixture config should be written");
    let artifact_root = root.join("artifacts");
    fs::create_dir_all(&artifact_root).expect("fixture artifact root should exist");
    LivePackageFixture {
        root,
        test_file,
        config,
        artifact_root,
    }
}

fn live_cli_args<const N: usize>(
    fixture: &LivePackageFixture,
    policy: [&str; N],
) -> impl Iterator<Item = String> {
    let mut args = vec![
        path_string(&fixture.test_file),
        "--live".to_string(),
        "--allow-network".to_string(),
        "--config".to_string(),
        path_string(&fixture.config),
        "--router-reload-url".to_string(),
        "http://127.0.0.1:1".to_string(),
        "--artifact-root".to_string(),
        path_string(&fixture.artifact_root),
    ];
    args.extend(policy.into_iter().map(str::to_string));
    args.into_iter()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

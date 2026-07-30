use std::{
    env,
    io::{self, BufWriter},
    process::{self, Command},
};

use skiff_test_runner::{SkiffTestResult, SkiffTestSummary};

use super::{finish, report_summary, USAGE};

const PIPE_CHILD_MODE: &str = "SKIFF_TEST_RUNNER_PIPE_CHILD_MODE";

#[test]
fn failure_output_survives_a_pipe_and_exits_one() {
    let output = run_pipe_child("failure");

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("PASS first.test.skiff::passes\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("FAIL second.test.skiff::first failure\n  first detail\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("FAIL third.test.skiff::second failure\n  second detail\n"),
        "{stdout}"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(stderr, format!("error: 2 test(s) failed\n{USAGE}\n"));
}

#[test]
fn success_output_and_exit_code_are_unchanged_through_a_pipe() {
    let output = run_pipe_child("success");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("PASS first.test.skiff::passes\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("test result: ok. 1 passed; 0 failed\n"),
        "{stdout}"
    );
    assert!(output.stderr.is_empty());
}

fn run_pipe_child(mode: &str) -> process::Output {
    Command::new(env::current_exe().unwrap())
        .args(["--exact", "tests::pipe_child", "--nocapture"])
        .env(PIPE_CHILD_MODE, mode)
        .output()
        .unwrap()
}

#[test]
fn pipe_child() {
    let Ok(mode) = env::var(PIPE_CHILD_MODE) else {
        return;
    };
    let summary = match mode.as_str() {
        "failure" => failure_summary(),
        "success" => success_summary(),
        other => panic!("unknown pipe child mode {other}"),
    };
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut stdout = BufWriter::new(stdout.lock());
    let mut stderr = BufWriter::new(stderr.lock());
    let code = finish(
        report_summary(&summary, false, true, &mut stdout),
        &mut stdout,
        &mut stderr,
    );
    process::exit(i32::from(code));
}

fn success_summary() -> SkiffTestSummary {
    SkiffTestSummary {
        passed: 1,
        skipped: 0,
        failed: 0,
        results: vec![result("first.test.skiff", "passes", true, None)],
    }
}

fn failure_summary() -> SkiffTestSummary {
    SkiffTestSummary {
        passed: 1,
        skipped: 0,
        failed: 2,
        results: vec![
            result("first.test.skiff", "passes", true, None),
            result(
                "second.test.skiff",
                "first failure",
                false,
                Some("first detail"),
            ),
            result(
                "third.test.skiff",
                "second failure",
                false,
                Some("second detail"),
            ),
        ],
    }
}

fn result(module_path: &str, name: &str, passed: bool, message: Option<&str>) -> SkiffTestResult {
    SkiffTestResult {
        module_path: module_path.to_string(),
        name: name.to_string(),
        passed,
        skipped: false,
        message: message.map(str::to_string),
    }
}

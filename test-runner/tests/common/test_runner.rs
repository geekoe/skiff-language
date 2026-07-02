#![allow(dead_code)]

use std::path::{Path, PathBuf};

use skiff_test_runner::{
    run_skiff_tests, run_skiff_tests_with_options, SkiffTestOptions, SkiffTestResult,
    SkiffTestSummary,
};

pub fn run_tests(input: &Path) -> SkiffTestSummary {
    run_skiff_tests(input, None).unwrap_or_else(|error| {
        panic!("Skiff tests failed to run for {}: {error}", input.display())
    })
}

pub fn run_tests_with_package_dirs(input: &Path, package_dirs: Vec<PathBuf>) -> SkiffTestSummary {
    run_skiff_tests_with_options(
        input,
        None,
        &SkiffTestOptions {
            package_dirs,
            ..SkiffTestOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("Skiff tests failed to run for {}: {error}", input.display()))
}

pub fn run_tests_error(input: &Path) -> String {
    run_skiff_tests(input, None)
        .expect_err("Skiff tests should fail before producing a summary")
        .to_string()
}

pub fn assert_failed_result_or_error(input: &Path, name: &str, messages: &[&str]) {
    match run_skiff_tests(input, None) {
        Ok(summary) => {
            assert!(
                matching_results(&summary, name)
                    .iter()
                    .any(|result| !result.passed && !result.skipped),
                "expected failing test {name:?}\n{}",
                format_summary(&summary)
            );
            assert_any_message_contains(&summary, messages);
        }
        Err(error) => {
            let error = error.to_string();
            assert!(
                messages.is_empty() || messages.iter().any(|message| error.contains(message)),
                "expected error containing one of {messages:?}, got: {error}"
            );
        }
    }
}

pub fn assert_counts(summary: &SkiffTestSummary, passed: usize, skipped: usize, failed: usize) {
    assert_eq!(summary.passed, passed, "{}", format_summary(summary));
    assert_eq!(summary.skipped, skipped, "{}", format_summary(summary));
    assert_eq!(summary.failed, failed, "{}", format_summary(summary));
}

pub fn assert_passed(summary: &SkiffTestSummary, name: &str) {
    assert!(
        matching_results(summary, name)
            .iter()
            .any(|result| result.passed && !result.skipped),
        "expected passing test {name:?}\n{}",
        format_summary(summary)
    );
}

pub fn assert_failed(summary: &SkiffTestSummary, name: &str, message: &str) {
    assert!(
        matching_results(summary, name).iter().any(|result| {
            !result.passed
                && !result.skipped
                && result
                    .message
                    .as_deref()
                    .is_some_and(|actual| actual.contains(message))
        }),
        "expected failing test {name:?} containing {message:?}\n{}",
        format_summary(summary)
    );
}

pub fn assert_not_run(summary: &SkiffTestSummary, name: &str) {
    assert!(
        matching_results(summary, name).is_empty(),
        "expected test {name:?} not to run\n{}",
        format_summary(summary)
    );
}

pub fn assert_any_message_contains(summary: &SkiffTestSummary, messages: &[&str]) {
    if messages.is_empty() {
        return;
    }
    assert!(
        summary.results.iter().any(|result| {
            result
                .message
                .as_deref()
                .is_some_and(|actual| messages.iter().any(|message| actual.contains(message)))
        }),
        "expected any result message containing one of {messages:?}\n{}",
        format_summary(summary)
    );
}

pub fn matching_results<'a>(summary: &'a SkiffTestSummary, name: &str) -> Vec<&'a SkiffTestResult> {
    summary
        .results
        .iter()
        .filter(|result| result.name == name)
        .collect()
}

pub fn format_summary(summary: &SkiffTestSummary) -> String {
    summary
        .results
        .iter()
        .map(|result| {
            let status = if result.skipped {
                "SKIP"
            } else if result.passed {
                "PASS"
            } else {
                "FAIL"
            };
            let message = result.message.as_deref().unwrap_or("");
            format!("{status} {}::{} {message}", result.module_path, result.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

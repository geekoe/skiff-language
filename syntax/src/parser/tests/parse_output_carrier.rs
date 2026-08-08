//! Reproducible carrier for detecting byte changes in parsed output across all
//! committed `.skiff` fixtures.
//!
//! The carrier enumerates every committed `.skiff` file under the repository
//! root, computes the `parse_source` output (serialized `SourceFile` or error),
//! and compares it with the committed Phase 0 baseline data file. After the
//! A parser refactor that changes output yields the exact first difference;
//! regenerate the baseline explicitly with
//! `UPDATE_PARSER_PHASE0_BASELINE=1` only when the change is intentional.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::parser::parse_source;

pub(crate) fn repo_root() -> PathBuf {
    // `syntax` sits one level below the repository root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub(crate) fn skiff_fixture_paths(root: &Path) -> Vec<PathBuf> {
    let output = Command::new("git")
        .arg("ls-files")
        .arg("-z")
        .arg("--cached")
        .arg("--")
        .arg("*.skiff")
        .current_dir(root)
        .output()
        .unwrap_or_else(|error| panic!("failed to list tracked .skiff files: {error}"));
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut paths = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let relative = String::from_utf8_lossy(raw);
        paths.push(root.join(relative.as_ref()));
    }
    paths.sort();
    paths
}

pub(crate) fn canonical_parse_output(source: &str) -> String {
    match parse_source(source) {
        Ok(ast) => format!(
            "OK {}",
            serde_json::to_string(&ast).expect("SourceFile serialization should succeed")
        ),
        Err(error) => format!("ERR {error}"),
    }
}

pub(crate) fn fixture_baseline_entries(root: &Path) -> String {
    let mut entries = String::new();
    for path in skiff_fixture_paths(root) {
        let relative = path
            .strip_prefix(root)
            .unwrap_or_else(|_| panic!("fixture {} outside root", path.display()));
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        entries.push_str(&relative.to_string_lossy());
        entries.push('\t');
        entries.push_str(&canonical_parse_output(&source));
        entries.push('\n');
    }
    entries
}

fn baseline_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("parser")
        .join("tests")
        .join("data")
        .join("fixture-parse-output-baseline.txt")
}

#[test]
fn fixture_corpus_is_non_empty() {
    let paths = skiff_fixture_paths(&repo_root());
    assert!(
        paths.len() >= 70,
        "expected at least 70 committed .skiff fixtures, got {}",
        paths.len()
    );
}

#[test]
fn fixture_parse_output_is_deterministic() {
    for path in skiff_fixture_paths(&repo_root()) {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let first = canonical_parse_output(&source);
        let second = canonical_parse_output(&source);
        assert_eq!(
            first,
            second,
            "parse output is not deterministic for {}",
            path.display()
        );
    }
}

#[test]
fn fixture_parse_output_matches_phase0_baseline() {
    let actual = fixture_baseline_entries(&repo_root());
    if std::env::var_os("UPDATE_PARSER_PHASE0_BASELINE").is_some() {
        std::fs::write(baseline_path(), actual).expect("write parse output baseline");
        return;
    }

    let expected = include_str!("data/fixture-parse-output-baseline.txt");
    if actual == expected {
        return;
    }

    let mut message = String::from("fixture parse output differs from the Phase 0 baseline\n");
    for (index, (expected_line, actual_line)) in expected.lines().zip(actual.lines()).enumerate() {
        if expected_line != actual_line {
            message.push_str(&format!(
                "first difference at entry {index}:\n  baseline: {expected_line}\n  actual:   {actual_line}\n"
            ));
            break;
        }
    }
    if expected.lines().count() != actual.lines().count() {
        message.push_str(&format!(
            "entry count differs: baseline {} vs actual {}\n",
            expected.lines().count(),
            actual.lines().count()
        ));
    }
    message.push_str("regenerate with UPDATE_PARSER_PHASE0_BASELINE=1 after an intentional change");
    panic!("{message}");
}

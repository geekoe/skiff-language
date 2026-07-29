use std::{
    path::PathBuf,
    process::{Command, Output},
};

#[test]
fn explicit_test_http_entries_cross_the_real_isolated_router() {
    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner must live below the repository root")
        .to_path_buf();
    let runner = repository.join("test-runner/fixtures/http-entry-test-service/run.mjs");
    let output = Command::new("node")
        .arg(runner)
        .env("SKIFF_HTTP_ENTRY_PROBE_ROOT", &repository)
        .env(
            "SKIFF_HTTP_ENTRY_PROBE_TEST_RUNNER",
            env!("CARGO_BIN_EXE_skiff-test-runner"),
        )
        .env(
            "SKIFF_HTTP_ENTRY_PROBE_BOOTSTRAP",
            env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture"),
        )
        .output()
        .expect("launch HTTP entry probe orchestration");

    assert_success(&output);
    let stdout = String::from_utf8_lossy(&output.stdout);
    for marker in [
        "EXPECTED_CONCURRENCY_REJECTION",
        "HAPPY_HTTP_ENTRY_PASS",
        "ASSEMBLY_READY expected-rejection",
        "ASSEMBLY_READY happy",
        "ISOLATED_CLEANUP_PASS",
    ] {
        assert!(
            stdout.contains(marker),
            "probe output omitted {marker}\nstdout:\n{stdout}"
        );
    }
}

fn assert_success(output: &Output) {
    if output.status.success() {
        return;
    }
    panic!(
        "HTTP entry probe failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

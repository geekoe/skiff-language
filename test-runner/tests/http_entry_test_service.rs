use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

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

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("test-runner must live below the repository root")
        .to_path_buf()
}

fn probe_output(repository: &Path, mode: &str) -> Output {
    let runner = repository.join("test-runner/fixtures/http-entry-test-service/run.mjs");
    Command::new("node")
        .arg(runner)
        .env("SKIFF_HTTP_ENTRY_PROBE_MODE", mode)
        .env("SKIFF_HTTP_ENTRY_PROBE_ROOT", repository)
        .env(
            "SKIFF_HTTP_ENTRY_PROBE_TEST_RUNNER",
            env!("CARGO_BIN_EXE_skiff-test-runner"),
        )
        .env(
            "SKIFF_HTTP_ENTRY_PROBE_BOOTSTRAP",
            env!("CARGO_BIN_EXE_skiff-package-service-smoke-fixture"),
        )
        .output()
        .expect("launch HTTP entry probe orchestration")
}

fn assert_markers(output: &Output, markers: &[&str]) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    for marker in markers {
        assert!(
            stdout.contains(marker),
            "probe output omitted {marker}\nstdout:\n{stdout}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_http_entry_probe_crosses_the_real_isolated_router() {
        let output = probe_output(&repository_root(), "active");
        assert_success(&output);
        assert_markers(
            &output,
            &[
                "EXPECTED_CONCURRENCY_REJECTION",
                "ASSEMBLY_READY expected-rejection",
                "ISOLATED_CLEANUP_PASS",
            ],
        );
    }

    #[test]
    fn happy_http_entry_probe_crosses_the_real_isolated_router() {
        let output = probe_output(&repository_root(), "happy");
        assert_success(&output);
        assert_markers(
            &output,
            &[
                "HAPPY_HTTP_ENTRY_PASS",
                "ASSEMBLY_READY happy",
                "ISOLATED_CLEANUP_PASS",
            ],
        );
    }
}

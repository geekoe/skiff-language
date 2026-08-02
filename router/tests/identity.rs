use std::process::Command;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_reports_binary_sha256() {
        let output = Command::new(env!("CARGO_BIN_EXE_skiff-router"))
            .arg("--identity")
            .output()
            .expect("run skiff-router --identity");
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).expect("identity stdout must be UTF-8");
        let line = stdout.trim();
        let digest = line
            .strip_prefix("skiff-router ")
            .expect("identity line must start with the binary name");
        assert_eq!(digest.len(), 64, "sha256 digest must be 64 hex characters");
        assert!(
            digest.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "sha256 digest must be hexadecimal"
        );
    }

    #[test]
    fn bare_invocation_exits_without_listener() {
        let output = Command::new(env!("CARGO_BIN_EXE_skiff-router"))
            .output()
            .expect("run skiff-router");
        assert!(output.status.success());
        let stderr = String::from_utf8(output.stderr).expect("stderr must be UTF-8");
        assert!(
            stderr.contains("no listener bound"),
            "bare invocation must report the no-listener skeleton state"
        );
    }
}

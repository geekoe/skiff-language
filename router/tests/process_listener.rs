//! Binary lifecycle probe for the E-bootstrap wiring: without committed
//! activation state the epoch cannot be read, so `run_router` must fail
//! closed before binding any listener and exit non-zero. The real
//! success path (committed state -> published epoch -> listeners ->
//! `router.bootstrap` over a real socket) is covered by the ignored
//! `bootstrap_live_probe` driven by `scripts/run-router-bootstrap-live.mjs`.

use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(not(unix), ignore = "SIGTERM delivery is exercised on unix")]
    fn binary_fails_closed_without_bootstrap_state() {
        let http_port = reserve_port();
        let control_port = reserve_port();
        assert_ne!(http_port, control_port, "ports must not collide");
        let config_path = write_config(http_port, control_port);
        let mut child = task_router(config_path.to_str().expect("utf8 config path"));

        let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(15));
        assert!(
            !status.success(),
            "bootstrap without profile must exit non-zero, got {status}"
        );
        assert!(
            stderr.contains("bootstrap failed closed"),
            "stderr must report the E-bootstrap fail-closed reason, got: {stderr:?}"
        );
        assert!(
            ports_closed(http_port, control_port),
            "no listener may bind after a fail-closed bootstrap"
        );
        let _ = std::fs::remove_file(&config_path);
    }

    fn task_router(config_path: &str) -> Child {
        Command::new(env!("CARGO_BIN_EXE_skiff-router"))
            .arg(config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn skiff-router")
    }

    fn wait_for_exit(child: &mut Child, deadline: Duration) -> (std::process::ExitStatus, String) {
        let mut stderr = String::new();
        if let Some(mut handle) = child.stderr.take() {
            let _ = handle.read_to_string(&mut stderr);
        }
        let deadline = Instant::now() + deadline;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return (status, stderr),
                Ok(None) => {}
                Err(error) => panic!("wait for router failed: {error}"),
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("router did not exit after fail-closed bootstrap");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn ports_closed(http_port: u16, control_port: u16) -> bool {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if TcpStream::connect(("127.0.0.1", http_port)).is_err()
                && TcpStream::connect(("127.0.0.1", control_port)).is_err()
            {
                return true;
            }
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn reserve_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("reserve port");
        listener.local_addr().expect("local addr").port()
    }

    fn write_config(http_port: u16, control_port: u16) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "skiff-router-pr0b-{}-{http_port}.yml",
            std::process::id()
        ));
        let contents = format!(
            "profile: dev\nartifactsPath: ./artifacts\nserviceDb:\n  mongoUrl: mongodb://127.0.0.1:27017/skiff\nhttp:\n  port: {http_port}\n  maxRequestBytes: 1\n  maxResponseBytes: 1\nruntime:\n  port: {control_port}\n  maxConcurrency: 4\n"
        );
        std::fs::write(&path, contents).expect("write config fixture");
        path
    }
}

//! End-to-end binary lifecycle probe: spawn the real `skiff-router` binary
//! against a rendered-style config, wait for both listeners, probe the health
//! placeholder over a real socket, deliver SIGTERM and assert the graceful
//! shutdown exits 0.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(not(unix), ignore = "SIGTERM delivery is exercised on unix")]
    fn binary_parses_config_binds_listeners_and_shuts_down_gracefully() {
        let http_port = reserve_port();
        let control_port = reserve_port();
        assert_ne!(http_port, control_port, "ports must not collide");
        let config_path = write_config(http_port, control_port);
        let mut child = spawn_router(config_path.to_str().expect("utf8 config path"));
        let pid = child.id();

        try_wait_until(&mut child, || listeners_ready(http_port, control_port));

        let health = request(
            "GET /__router/health HTTP/1.1\r\nhost: probe\r\nconnection: close\r\n\r\n",
            control_port,
        );
        assert!(
            health.starts_with("HTTP/1.1 200"),
            "health placeholder must answer 200, got: {health:?}"
        );
        assert!(
            health.ends_with("\r\n\r\n"),
            "health placeholder body must be empty, got: {health:?}"
        );
        let public = request(
            "GET / HTTP/1.1\r\nhost: probe\r\nconnection: close\r\n\r\n",
            http_port,
        );
        assert!(
            public.starts_with("HTTP/1.1 200"),
            "public listener must answer 200, got: {public:?}"
        );

        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("deliver SIGTERM");
        assert!(signaled.success(), "kill -TERM must succeed");

        let deadline = Instant::now() + Duration::from_secs(10);
        let output = loop {
            match child.try_wait() {
                Ok(Some(status)) => break (status, String::new()),
                Ok(None) => {
                    if Instant::now() > deadline {
                        let _ = child.kill();
                        panic!("router did not exit after SIGTERM");
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(error) => panic!("wait for router failed: {error}"),
            }
        };
        assert!(
            output.0.success(),
            "graceful shutdown must exit 0, got {output:?}"
        );
        let _ = std::fs::remove_file(&config_path);
    }

    fn spawn_router(config_path: &str) -> Child {
        Command::new(env!("CARGO_BIN_EXE_skiff-router"))
            .arg(config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn skiff-router")
    }

    fn listeners_ready(http_port: u16, control_port: u16) -> bool {
        TcpStream::connect(("127.0.0.1", http_port)).is_ok()
            && TcpStream::connect(("127.0.0.1", control_port)).is_ok()
    }

    fn try_wait_until(child: &mut Child, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if ready() {
                return;
            }
            if let Ok(Some(status)) = child.try_wait() {
                panic!("router exited before listeners were ready: {status}");
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                panic!("router listeners did not become ready");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn request(request: &str, port: u16) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut response = Vec::new();
                    let _ = stream.read_to_end(&mut response);
                    return String::from_utf8_lossy(&response).into_owned();
                }
            }
            if Instant::now() > deadline {
                panic!("request to port {port} timed out");
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

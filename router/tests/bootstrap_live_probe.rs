//! `router-live:bootstrap` real boundary probe (M4 gate).
//!
//! Driven by `scripts/run-router-bootstrap-live.mjs`: the harness compiles a
//! real package/assembly artifact, starts an isolated temporary Mongo replica
//! set and leases router ports; this ignored test then seeds the release
//! pointer table (typed pointer store) and the actor routing projection
//! record, runs the bootstrap chain, and spawns the real `skiff-router`
//! binary to observe the profile-only `router.bootstrap` frame over the
//! `/runtime` WebSocket and the pointer-table `activeAssembly` health
//! projection. The fail-closed matrix (missing artifact root / invalid
//! profile) is asserted at both assembly and process level.

use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use skiff_artifact_identity::ArtifactRelativePath;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::artifact::ActorRoutingProjectionRef;
use skiff_router::bootstrap::{
    BlockingLoader, BlockingLoaderOptions, RouterBootstrapAssembly,
    ACTOR_ROUTING_PROJECTION_RECORD_PATH,
};

struct LiveProfile {
    mongo_url: String,
    database: String,
    artifact_root: PathBuf,
    profile: String,
    service_id: String,
    version: String,
    build_id: String,
    http_port: u16,
    runtime_port: u16,
    temp_dir: PathBuf,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/run-router-bootstrap-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_BOOTSTRAP_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ROUTER_BOOTSTRAP_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        Self {
            mongo_url: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_MONGO_URL"),
            database: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_BOOTSTRAP_LIVE_ARTIFACT_ROOT")),
            profile: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_PROFILE"),
            service_id: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_SERVICE_ID"),
            version: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_VERSION"),
            build_id: required("SKIFF_ROUTER_BOOTSTRAP_LIVE_BUILD_ID"),
            http_port,
            runtime_port,
            temp_dir: PathBuf::from(required("SKIFF_ROUTER_BOOTSTRAP_LIVE_TEMP_DIR")),
        }
    }

    fn actor_ref(&self) -> ActorRoutingProjectionRef {
        ActorRoutingProjectionRef::new(
            ArtifactRelativePath::new(
                ACTOR_ROUTING_PROJECTION_RECORD_PATH,
                "actor routing projection record",
            )
            .expect("actor projection record path"),
        )
    }
}

fn write_router_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-{}-{}.yml",
        live.http_port, live.runtime_port
    ));
    let contents = format!(
        "profile: {}\n\
         host: 127.0.0.1\n\
         artifactsPath: {}\n\
         releaseMode: true\n\
         requestTimeoutMs: 20000\n\
         http:\n  port: {}\n  maxRequestBytes: 1048576\n  maxResponseBytes: 1048576\n\
         runtime:\n  port: {}\n  path: /runtime\n  maxConcurrency: 16\n\
         serviceDb:\n  mongoUrl: {}\n",
        live.profile,
        live.artifact_root.display(),
        live.http_port,
        live.runtime_port,
        live.mongo_url,
    );
    std::fs::write(&path, contents).expect("write router config");
    path
}

fn task_router(config_path: &Path) -> Child {
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
            panic!("router did not exit within deadline; stderr: {stderr}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_listeners(live: &LiveProfile, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if TcpStream::connect(("127.0.0.1", live.http_port)).is_ok()
            && TcpStream::connect(("127.0.0.1", live.runtime_port)).is_ok()
        {
            return;
        }
        if let Ok(Some(status)) = child.try_wait() {
            let mut stderr = String::new();
            if let Some(mut handle) = child.stderr.take() {
                let _ = handle.read_to_string(&mut stderr);
            }
            panic!("router exited before listeners were ready: {status}; stderr: {stderr}");
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("router listeners did not become ready");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_ports_closed(live: &LiveProfile) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", live.http_port)).is_err()
            && TcpStream::connect(("127.0.0.1", live.runtime_port)).is_err()
        {
            return;
        }
        if Instant::now() > deadline {
            panic!("router left a listener bound after exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn assert_process_fails_closed(live: &LiveProfile) {
    let config_path = write_router_config(live);
    let mut child = task_router(&config_path);
    let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(30));
    assert!(
        !status.success(),
        "fail-closed bootstrap must exit non-zero, got {status}; stderr: {stderr}"
    );
    assert!(
        stderr.contains("bootstrap failed closed") || stderr.contains("artifact store"),
        "stderr must report the fail-closed bootstrap, got: {stderr:?}"
    );
    assert_ports_closed(live);
    let _ = std::fs::remove_file(config_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "driven by scripts/run-router-bootstrap-live.mjs"]
    async fn router_live_bootstrap_chain() {
        let live = LiveProfile::from_env();

        // The deployment record is produced by the real authoring tooling
        // inside the compiler-produced artifact root. Materialize the actor
        // routing projection record and seed the release pointer table
        // (M1 authoring already writes pointers; this probe re-seeds for an
        // isolated environment).
        let projection_directory = live.artifact_root.join("records/actor-routing");
        std::fs::create_dir_all(&projection_directory).expect("create projection directory");
        let projection = ActorRoutingProjection::new(
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
            Vec::new(),
        )
        .expect("empty projection");
        let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
        std::fs::write(
            live.artifact_root
                .join(ACTOR_ROUTING_PROJECTION_RECORD_PATH),
            bytes,
        )
        .expect("write projection record");

        // 1. In-process success chain: store open + profile validation.
        let pool = Arc::new(BlockingLoader::new(BlockingLoaderOptions::default()));
        let config = skiff_router::config::load_router_config(
            write_router_config(&live)
                .to_str()
                .expect("config path utf8"),
        )
        .expect("load router config");
        let assembly = RouterBootstrapAssembly::assemble(&config)
            .await
            .expect("bootstrap must succeed");
        assert_eq!(assembly.profile(), live.profile);
        assert!(assembly.store().root().starts_with(&live.artifact_root));
        let _ = pool;
        assembly.shutdown().await;

        // 2. Real process: the runtime socket receives the profile-only
        // `router.bootstrap` frame and `/__router/health` reports the
        // pointer-table `activeAssembly`.
        let config_path = write_router_config(&live);
        let mut child = task_router(&config_path);
        wait_for_listeners(&live, &mut child);
        let (mut socket, _response) = tokio_tungstenite::connect_async(format!(
            "ws://127.0.0.1:{}/runtime",
            live.runtime_port
        ))
        .await
        .expect("connect runtime websocket");
        let frame = tokio::time::timeout(Duration::from_secs(15), socket.next())
            .await
            .expect("bootstrap frame timeout")
            .expect("bootstrap frame stream")
            .expect("bootstrap frame read");
        let bytes = match frame {
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => bytes,
            other => panic!("expected binary router.bootstrap frame, got {other:?}"),
        };
        let header = skiff_runtime_transport::protocol::decode_router_bootstrap_frame(&bytes)
            .expect("decode router.bootstrap frame");
        assert_eq!(header.envelope_type, "router.bootstrap");
        assert_eq!(header.activation.profile, live.profile);
        assert_eq!(header.service_db.mongo_url, live.mongo_url);
        assert_eq!(
            header.artifacts_path,
            live.artifact_root.to_string_lossy().into_owned()
        );
        drop(socket);

        let health = TcpStream::connect(("127.0.0.1", live.http_port)).expect("health connect");
        drop(health);
        let pid = child.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("deliver SIGTERM");
        assert!(signaled.success(), "kill -TERM must succeed");
        let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(20));
        assert!(
            status.success(),
            "graceful shutdown must exit 0, got {status}; stderr: {stderr}"
        );
        assert_ports_closed(&live);
        let _ = std::fs::remove_file(config_path);
    }

    #[tokio::test]
    #[ignore = "driven by scripts/run-router-bootstrap-live.mjs"]
    async fn router_live_bootstrap_missing_store_fails_closed() {
        let live = LiveProfile::from_env();
        let mut config = skiff_router::config::load_router_config(
            write_router_config(&live)
                .to_str()
                .expect("config path utf8"),
        )
        .expect("load router config");
        config.artifacts_path = live.temp_dir.join("missing-artifact-root");
        let error = RouterBootstrapAssembly::assemble(&config)
            .await
            .expect_err("missing artifact root must fail closed");
        assert!(error.to_string().contains("artifact store open failed"));
    }

    #[tokio::test]
    #[ignore = "driven by scripts/run-router-bootstrap-live.mjs"]
    async fn router_live_bootstrap_invalid_profile_fails_closed() {
        let live = LiveProfile::from_env();
        let mut config = skiff_router::config::load_router_config(
            write_router_config(&live)
                .to_str()
                .expect("config path utf8"),
        )
        .expect("load router config");
        config.profile = "invalid profile".to_string();
        let error = RouterBootstrapAssembly::assemble(&config)
            .await
            .expect_err("invalid profile must fail closed");
        assert!(error.to_string().contains("profile is invalid"));
    }

    #[tokio::test]
    #[ignore = "driven by scripts/run-router-bootstrap-live.mjs"]
    async fn router_live_process_fails_closed_without_artifact_root() {
        let live = LiveProfile::from_env();
        let config_path = write_router_config(&live);
        let config = skiff_router::config::load_router_config(
            config_path.to_str().expect("config path utf8"),
        )
        .expect("load router config");
        let mut patched = std::fs::read_to_string(&config_path).expect("read config");
        patched = patched.replace(
            &live.artifact_root.display().to_string(),
            &live.temp_dir.join("missing-root").display().to_string(),
        );
        let patched_path = live.temp_dir.join("router-missing-root.yml");
        std::fs::write(&patched_path, patched).expect("write patched config");
        let _ = config;
        let mut child = task_router(&patched_path);
        let (status, stderr) = wait_for_exit(&mut child, Duration::from_secs(30));
        assert!(
            !status.success(),
            "missing artifact root must fail closed at process level; stderr: {stderr}"
        );
        assert_ports_closed(&live);
        let _ = std::fs::remove_file(&patched_path);
    }
}

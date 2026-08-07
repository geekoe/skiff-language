//! E-bootstrap production wiring tests (M4): `RouterSupervisor` assembles the
//! full composition over a real artifact root with the release pointer table
//! seeded (deployment record + pointer), starts the listeners and exercises
//! the fail-closed HTTP ingress path. The Mongo activation repository seam is
//! retired.

use skiff_router::config::RouterConfig;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::RouterSupervisor;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-w-composition-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&parent).expect("create temp parent");
        Self {
            parent: parent.clone(),
            root: parent.join("root"),
        }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.parent);
    }
}

fn materialize(profile: &str) -> TestRoot {
    let root = TestRoot::new();
    fs::create_dir_all(root.path()).expect("create artifact root");
    let store = skiff_deployment::storage::CanonicalArtifactStore::create(root.path())
        .expect("create artifact store");

    let mut deployment =
        skiff_deployment::fixtures::service_deployment_fixture().expect("deployment fixture");
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("assign deployment identity");
    store
        .write_service_deployment(&deployment)
        .expect("write deployment record");
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    store
        .write_release_pointer(
            &skiff_deployment::storage::ReleasePointer::new(profile, reference)
                .expect("release pointer"),
        )
        .expect("write release pointer");

    let directory = root.path().join("records/actor-routing");
    fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = skiff_deployment::projection::actor_routing::ActorRoutingProjection::new(
        skiff_deployment::projection::actor_routing::ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
            .to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = skiff_canonical_json::canonical_json_bytes(&projection)
        .expect("canonical projection bytes");
    fs::write(
        root.path().join("records/actor-routing/current.json"),
        bytes,
    )
    .expect("write projection record");
    root
}

fn config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: "prod".to_string(),
        release_mode: Some(true),
        request_timeout_ms: 1_000,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

async fn raw_get(addr: std::net::SocketAddr, path: &str) -> (String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to public http");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Skiff-Service: example.com/service-1\r\nX-Skiff-Version: 1.0.0\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    (status, text)
}

async fn raw_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> (String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to listener");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    if !body.is_empty() {
        stream.write_all(body).await.expect("write request body");
    }
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    (status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_assembles_full_composition_and_wires_public_http() {
        let root = materialize("prod");
        let config = config(root.path());

        let supervisor = RouterSupervisor::assemble(&config)
            .await
            .expect("production composition must assemble");

        let components = supervisor.components();
        // Static consumer manifest: exactly the installed session-keyed owners.
        assert_eq!(
            components.session.manifest_kinds(),
            vec![
                skiff_router::session::ConsumerKind::HealthLedger,
                skiff_router::session::ConsumerKind::RequestDispatcher,
                skiff_router::session::ConsumerKind::WebSocketRequestBroker,
                skiff_router::session::ConsumerKind::ActorSessionOwner,
            ]
        );
        // Installed sink bundle: request/connection/actor/task all wired.
        let sinks = components.session.inbound_sinks();
        assert!(sinks.request.is_some());
        assert!(sinks.connection.is_some());
        assert!(sinks.actor.is_some());
        assert!(sinks.task.is_some());

        // Components are live and empty.
        assert_eq!(components.dispatcher.pending_count(), 0);
        assert!(!components.surface_view.is_empty());
        assert_eq!(components.pending_http.pending_count(), 0);

        // Real-socket wiring: unknown service -> 404 ReleaseNotFound.
        let listeners = supervisor
            .start_listeners(&ListenerStartOptions::default())
            .await
            .expect("listeners start");
        let http_addr = listeners.public_http.addr();
        let (status, body) = raw_get(http_addr, "/items").await;
        assert!(
            status.contains("404"),
            "expected platform 404 for unknown release ingress, got {status:?}"
        );
        assert!(body.contains("ReleaseNotFound"));

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn supervisor_fails_closed_without_an_artifact_root() {
        let root = TestRoot::new();
        let config = config(root.path());
        let result = RouterSupervisor::assemble(&config).await;
        assert!(result.is_err(), "missing artifact root must fail closed");
    }

    #[tokio::test]
    async fn supervisor_fails_closed_on_invalid_profile() {
        let root = materialize("prod");
        let mut config = config(root.path());
        config.profile = "invalid profile".to_string();
        let result = RouterSupervisor::assemble(&config).await;
        assert!(result.is_err(), "invalid profile must fail closed");
    }

    #[tokio::test]
    async fn runtime_control_serves_health_and_test_dispatch_routes() {
        let root = materialize("prod");
        let config = config(root.path());
        let supervisor = RouterSupervisor::assemble(&config)
            .await
            .expect("production composition must assemble");
        let listeners = supervisor
            .start_listeners(&ListenerStartOptions::default())
            .await
            .expect("listeners start");
        let control_addr = listeners.runtime_control.addr();

        // Health route is served with a JSON body (GET-only).
        let (status, body) = raw_request(control_addr, "GET", "/__router/health", b"").await;
        assert!(status.contains("200"), "health must be 200, got {status:?}");
        assert!(
            body.contains("\"ok\":true"),
            "health must report ok, got {body:?}"
        );

        // The retired activation endpoint is no longer served (empty 200).
        let (status, _) = raw_request(
            control_addr,
            "POST",
            "/__skiff/activate-assembly",
            br#"{"profile":"prod"}"#,
        )
        .await;
        assert!(
            status.contains("200"),
            "retired activation endpoint must keep the empty 200, got {status:?}"
        );

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }
}

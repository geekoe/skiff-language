//! Router-owned half of the Phase 7 whole-system chain, proven from the Router
//! crate against the same real compiler-published artifact root the host-side
//! whole-system tests drive through the real RuntimeHost.
//!
//! The Router half cannot host a RuntimeHost (the host crate depends on the
//! Router, so a Router dev-dependency would be cyclic); this carrier therefore
//! proves the Router-owned fail-closed boundary deterministically:
//!
//! - the production `RouterSupervisor::assemble_with_task_store` composition
//!   assembles over the real compiler-published carrier;
//! - the real HTTP gateway resolves the exact compiler-published release and
//!   admission rejects with `ServiceUnavailable` when no runtime session is
//!   registered (no phantom dispatch, no hang, dispatcher balanced);
//! - unknown service/version and missing selector headers fail closed.
//!
//! The same production seam (`assemble_with_task_store` + real listeners +
//! real HTTP) is the exact seam the host-side whole-system assertions use, so
//! both crates prove complementary halves of one real chain.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use skiff_router::config::RouterConfig;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::{RouterSupervisor, SupervisorListeners};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::fixture::{published_unary, RouterFixture};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot {
    parent: std::path::PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-bcvm-p7-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&parent).expect("create temp parent");
        Self { parent }
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.parent);
    }
}

fn config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        run_dir: None,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: "skiff-test".to_string(),
        release_mode: Some(true),
        request_timeout_ms: 5_000,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff-p7-router-boundary".to_string(),
        },
        telemetry: None,
        profile_sampling: None,
        websocket_path: "/ws".to_string(),
    }
}

struct Boundary {
    _root: TestRoot,
    fixture: RouterFixture,
    supervisor: RouterSupervisor,
    listeners: SupervisorListeners,
    addr: std::net::SocketAddr,
}

impl Boundary {
    async fn start(prefix: &str) -> Self {
        let fixture = published_unary(prefix);
        let _root = TestRoot::new();
        let supervisor = RouterSupervisor::assemble_with_task_store(
            &config(&fixture.artifact_root),
            std::sync::Arc::new(skiff_task_control::MemoryTaskStore::new()),
        )
        .await
        .expect("production RouterSupervisor assembly over the real carrier");
        let listeners = supervisor
            .start_listeners(&ListenerStartOptions {
                public_bind: Some("127.0.0.1:0".parse().expect("bind address")),
                runtime_control_bind: Some("127.0.0.1:0".parse().expect("bind address")),
                drain_deadline: Duration::from_secs(10),
            })
            .await
            .expect("production listeners");
        let addr = listeners.public_http.addr();
        Self {
            _root,
            fixture,
            supervisor,
            listeners,
            addr,
        }
    }

    async fn shutdown(self) {
        self.listeners.shutdown().await.expect("listeners shutdown");
        self.supervisor.shutdown().await;
    }

    fn assert_balanced(&self) {
        assert!(
            self.supervisor.components().dispatcher.pending_count() == 0,
            "fail-closed boundary must leave the Router dispatcher empty"
        );
        let health = self.supervisor.components().dispatcher.health();
        assert_eq!(health.pending.unary, 0, "no pending unary");
        assert_eq!(health.pending.stream, 0, "no pending stream");
        assert_eq!(health.admission.permits_held, 0, "no held permits");
    }
}

async fn raw_post(
    addr: std::net::SocketAddr,
    path: &str,
    service: Option<&str>,
    version: Option<&str>,
    body: &[u8],
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to public http");
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(service) = service {
        request.push_str(&format!("X-Skiff-Service: {service}\r\n"));
    }
    if let Some(version) = version {
        request.push_str(&format!("X-Skiff-Version: {version}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request head");
    if !body.is_empty() {
        stream.write_all(body).await.expect("write request body");
    }
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    (status, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn real_release_resolution_without_runtime_fails_closed() {
        let boundary = Boundary::start("resolve").await;
        let (status, body) = raw_post(
            boundary.addr,
            "/phase-7/unary",
            Some(boundary.fixture.deployment.service_id.as_str()),
            Some(boundary.fixture.deployment.contract_version.as_str()),
            b"7",
        )
        .await;
        assert_eq!(
            status, 503,
            "known compiler-published release with no registered runtime must fail closed: {body}"
        );
        assert!(
            body.contains("ServiceUnavailable"),
            "fail-closed terminal must carry the exact ServiceUnavailable code: {body}"
        );
        assert!(
            body.contains("no eligible runtime"),
            "fail-closed terminal must name the no-candidate dispatch reason: {body}"
        );
        assert!(
            !body.contains("ReleaseNotFound"),
            "the compiler-published release must resolve before dispatch admission: {body}"
        );
        let health = boundary.supervisor.components().dispatcher.health();
        assert!(
            health.admission.no_candidate_rejects >= 1,
            "dispatcher must record the no-candidate admission rejection"
        );
        assert!(
            boundary
                .supervisor
                .components()
                .session
                .health_snapshot()
                .registered_sessions
                == 0,
            "no runtime session may be fabricated"
        );
        boundary.assert_balanced();
        boundary.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_service_fails_closed_404() {
        let boundary = Boundary::start("unknown-service").await;
        let (status, body) = raw_post(
            boundary.addr,
            "/phase-7/unary",
            Some("example.com/nope"),
            Some("1.0.0"),
            b"7",
        )
        .await;
        assert_eq!(status, 404, "unknown service must fail closed: {body}");
        assert!(body.contains("ReleaseNotFound"), "body: {body}");
        boundary.assert_balanced();
        boundary.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_version_fails_closed_404() {
        let boundary = Boundary::start("unknown-version").await;
        let (status, body) = raw_post(
            boundary.addr,
            "/phase-7/unary",
            Some(boundary.fixture.deployment.service_id.as_str()),
            Some("9.9.9"),
            b"7",
        )
        .await;
        assert_eq!(status, 404, "unknown version must fail closed: {body}");
        assert!(body.contains("ReleaseNotFound"), "body: {body}");
        boundary.assert_balanced();
        boundary.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn missing_selector_headers_fail_closed() {
        let boundary = Boundary::start("missing-selector").await;
        let (status, body) = raw_post(boundary.addr, "/phase-7/unary", None, None, b"7").await;
        assert_eq!(
            status, 400,
            "missing selector headers must fail closed: {body}"
        );
        assert!(
            body.contains("ServiceSelectorRequired"),
            "missing selector must carry the exact platform code: {body}"
        );
        boundary.assert_balanced();
        boundary.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn production_composition_installs_the_exact_consumer_manifest() {
        let boundary = Boundary::start("manifest").await;
        let components = boundary.supervisor.components();
        assert_eq!(
            components.session.manifest_kinds(),
            vec![
                skiff_router::session::ConsumerKind::HealthLedger,
                skiff_router::session::ConsumerKind::RequestDispatcher,
                skiff_router::session::ConsumerKind::WebSocketRequestBroker,
                skiff_router::session::ConsumerKind::ActorSessionOwner,
            ],
            "the whole-system chain must run on the exact production consumer manifest"
        );
        let sinks = components.session.inbound_sinks();
        assert!(sinks.request.is_some());
        assert!(sinks.connection.is_some());
        assert!(sinks.actor.is_some());
        assert!(sinks.task.is_some());
        boundary.shutdown().await;
    }
}

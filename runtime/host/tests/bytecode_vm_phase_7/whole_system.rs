//! Real whole-system composition harness: client HTTP -> Router gateway /
//! dispatcher -> runtime WebSocket session -> RuntimeHost -> atomic image /
//! scheduler / provider -> response / task / Actor terminal.
//!
//! No fake dispatcher frame, hand-built artifact/image/fiber/owner token or
//! test-side projection is used: the Router is the production
//! `RouterSupervisor::assemble_with_task_store` composition, the Runtime is the
//! production `RuntimeHost` over a real WebSocket session, and every fixture is
//! a real compiler/linker publication.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use skiff_router::config::RouterConfig;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::{RouterSupervisor, SupervisorListeners};
use skiff_runtime_capability_context::DbProviderSource;
use skiff_runtime_host::{RuntimeConfig, RuntimeHost};
use skiff_runtime_service_db::InMemoryDbProviderFactory;
use skiff_task_control::MemoryTaskStore;
use tokio::task::JoinHandle;

use super::fixture::{Capability, PublishedFixture};
use super::stages::published_positive;

const PROFILE: &str = "skiff-test";

pub struct WholeSystem {
    pub fixture: PublishedFixture,
    pub store: Arc<MemoryTaskStore>,
    pub supervisor: RouterSupervisor,
    listeners: SupervisorListeners,
    _host: AbortOnDrop,
    _runtime_home: RuntimeHome,
    pub public_addr: std::net::SocketAddr,
}

impl WholeSystem {
    pub async fn start(capability: Capability, prefix: &str) -> Self {
        Self::start_with_request_timeout(capability, prefix, 30_000).await
    }

    pub async fn start_with_request_timeout(
        capability: Capability,
        prefix: &str,
        request_timeout_ms: u64,
    ) -> Self {
        let fixture = published_positive(capability, prefix);
        let store = Arc::new(MemoryTaskStore::new());
        let supervisor = RouterSupervisor::assemble_with_task_store(
            &router_config(&fixture, request_timeout_ms),
            Arc::clone(&store) as Arc<dyn skiff_task_control::store::TaskStore>,
        )
        .await
        .expect("production RouterSupervisor assembly");
        let listeners = supervisor
            .start_listeners(&ListenerStartOptions {
                public_bind: Some("127.0.0.1:0".parse().expect("bind address")),
                runtime_control_bind: Some("127.0.0.1:0".parse().expect("bind address")),
                drain_deadline: Duration::from_secs(10),
            })
            .await
            .expect("production listeners");
        let control_addr = listeners.runtime_control.addr();
        let public_addr = listeners.public_http.addr();
        let (_host, _runtime_home) = start_host(control_addr.port(), prefix);
        wait_for_registered(&supervisor).await;
        Self {
            fixture,
            store,
            supervisor,
            listeners,
            _host,
            _runtime_home,
            public_addr,
        }
    }

    pub async fn post(&self, path: &str, body: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{}{path}", self.public_addr))
            .header(
                "X-Skiff-Service",
                self.fixture.deployment.service_id.as_str(),
            )
            .header(
                "X-Skiff-Version",
                self.fixture.deployment.contract_version.as_str(),
            )
            .body(body.to_vec())
            .send()
            .await
            .expect("HTTP request through production Router");
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.to_string(),
                    value.to_str().expect("HTTP header is UTF-8").to_string(),
                )
            })
            .collect();
        let bytes = response.bytes().await.expect("HTTP response body").to_vec();
        (status, headers, bytes)
    }

    pub async fn shutdown(self) {
        self.listeners.shutdown().await.expect("listeners shutdown");
        self.supervisor.shutdown().await;
    }

    /// Waits for the durable TaskStore to observe the accepted task record.
    pub async fn wait_for_records(&self) -> Vec<skiff_task_control::model::TaskRecord> {
        for _ in 0..200 {
            let records = self.store.records().await;
            if !records.is_empty() {
                return records;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("Router accepted no durable task record");
    }

    /// Waits for the task to reach the exact durable terminal kind.
    pub async fn wait_for_status(
        &self,
        task_id: &str,
        expected: skiff_task_control::model::TaskStatusKind,
    ) {
        use skiff_task_control::model::TaskId;
        use skiff_task_control::store::{StatusInput, TaskStore};
        for _ in 0..600 {
            let status = self
                .store
                .status(StatusInput {
                    task_id: TaskId::new(task_id),
                    retention: skiff_task_control::model::DurableDuration::from_millis(120_000),
                })
                .await
                .expect("task status");
            if status.kind == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "task {task_id} did not reach {expected:?}; records={:?}",
            self.store.records().await,
        );
    }
}

/// Drives one whole-system request to its terminal and returns the raw
/// response, then verifies the composition returned to a quiescent state.
pub async fn drive_terminal(capability: Capability, prefix: &str) -> WholeSystemResponse {
    let system = WholeSystem::start(capability, prefix).await;
    let (status, headers, body) = system.post(capability.ingress_path(), b"7").await;
    assert_eq!(status, 200, "whole-system HTTP terminal: {body:?}");
    assert!(
        !body.is_empty(),
        "whole-system terminal must expose an observable body"
    );
    assert!(
        headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("content-type") && value == "application/json; charset=utf-8"
        }),
        "typedJson whole-system response must carry canonical JSON content-type: {headers:?}"
    );
    assert_balanced(&system);
    let response = WholeSystemResponse {
        status,
        headers,
        body,
    };
    system.shutdown().await;
    response
}

/// Asserts the whole-system composition returned to a quiescent state after a
/// terminal: no pending request, no held permit and no leak into the session.
pub fn assert_balanced(system: &WholeSystem) {
    assert!(
        system.supervisor.components().dispatcher.pending_count() == 0,
        "whole-system request must leave the Router dispatcher empty"
    );
    let health = system.supervisor.components().dispatcher.health();
    assert_eq!(health.pending.unary, 0, "no pending unary after terminal");
    assert_eq!(health.pending.stream, 0, "no pending stream after terminal");
    assert_eq!(
        health.admission.permits_held, 0,
        "no held dispatch permits after terminal"
    );
    assert!(
        system
            .supervisor
            .components()
            .session
            .health_snapshot()
            .registered_sessions
            >= 1,
        "the runtime session must remain registered after the terminal"
    );
}

/// Drives an accepted-capability whole-system row that currently terminates
/// fail-closed with a deterministic protocol error (expected-red evidence for
/// the exact original owner). The assertion documents the genuine seam break:
/// the request terminates exactly once, the Router dispatcher is balanced and
/// the runtime session stays registered — no hang, leak or partial response.
pub async fn drive_fail_closed(capability: Capability, prefix: &str, surface: &str) {
    let system = WholeSystem::start(capability, prefix).await;
    let (status, _, body) = system.post(capability.ingress_path(), b"7").await;
    assert_eq!(
        status, 502,
        "{surface} whole-system row must currently terminate with the deterministic \
         protocol-error terminal: status={status} body={body:?}"
    );
    let json: serde_json::Value = serde_json::from_slice(&body)
        .unwrap_or_else(|_| panic!("{surface} fail-closed body must be platform JSON: {body:?}"));
    assert_eq!(
        json["error"]["code"], "InvalidHttpResponse",
        "{surface} fail-closed terminal must carry the exact Router protocol-error code: {json}"
    );
    assert_balanced(&system);
    system.shutdown().await;
}

pub struct WholeSystemResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

fn router_config(fixture: &PublishedFixture, request_timeout_ms: u64) -> RouterConfig {
    RouterConfig {
        run_dir: None,
        artifacts_path: fixture.artifact_root.clone(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: PROFILE.to_string(),
        release_mode: Some(true),
        request_timeout_ms,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 16,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff-p7-whole-system".to_string(),
        },
        telemetry: None,
        profile_sampling: None,
        websocket_path: "/ws".to_string(),
    }
}

fn start_host(control_port: u16, prefix: &str) -> (AbortOnDrop, RuntimeHome) {
    let home = RuntimeHome::new(prefix);
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(InMemoryDbProviderFactory::new()),
        router_url: format!("ws://127.0.0.1:{control_port}/runtime"),
        base_runtime_id: format!("runtime-phase-7-{prefix}"),
        runtime_home: home.path.clone(),
        profile: PROFILE.to_string(),
        bytecode_only: true,
        http_response_max_bytes: 64 * 1024,
        http_egress_proxy: None,
    })
    .expect("construct production RuntimeHost");
    let task = AbortOnDrop(Some(tokio::spawn(async move {
        host.run_forever()
            .await
            .expect("production RuntimeHost run loop")
    })));
    (task, home)
}

async fn wait_for_registered(supervisor: &RouterSupervisor) {
    for _ in 0..200 {
        if supervisor
            .components()
            .session
            .health_snapshot()
            .registered_sessions
            >= 1
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("RuntimeHost did not register with the Router");
}

struct RuntimeHome {
    path: PathBuf,
}

impl RuntimeHome {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p7-whole-system-{prefix}-{}-{id}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create runtime home");
        Self { path }
    }
}

impl Drop for RuntimeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

struct AbortOnDrop(Option<JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

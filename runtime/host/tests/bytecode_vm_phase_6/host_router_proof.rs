//! Real host -> Router vertical seam probes for the Phase 6 task/actor path.
//!
//! These tests build the production compiler fixture, assemble the production
//! RouterSupervisor in-process with an injected MemoryTaskStore, start the
//! production RuntimeHost against the Router listener, and drive HTTP through
//! the Router into the Runtime. They are expected-red on the current
//! candidate until the T6F/A6/X6 seams below are closed:
//!
//! - the Router sends ordinary and task-attempt requests with
//!   `assembly_identity` / `assembly_generation` unset, while the RuntimeHost
//!   task child requires exact activation identity, caller request id and
//!   runtime id;
//! - actor-method task emission is rejected by Phase 1 bytecode admission as
//!   a non-scalar callable effect inside an actor method.
//!
//! Once those seams land, the same probes prove host-produced accepted
//! records flow into TaskStore claim/lease/fresh attempt and Actor
//! get-or-activate / method execution.

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
use skiff_task_control::model::{DurableDuration, TaskId, TaskStatusKind};
use skiff_task_control::store::{StatusInput, TaskStore};
use skiff_task_control::MemoryTaskStore;
use tokio::task::JoinHandle;

use super::fixture::{build_actor_method_task_fixture, BuildOutcome, Capability, PublishedFixture};
use super::stages::published_positive;

const PROFILE: &str = "skiff-test";
const RUNTIME_ID: &str = "runtime-host-router-proof";

struct RuntimeHome {
    path: PathBuf,
}

impl RuntimeHome {
    fn new(prefix: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p6-host-router-{prefix}-{}-{id}",
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

fn router_config(fixture: &PublishedFixture) -> RouterConfig {
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
        request_timeout_ms: 30_000,
        rewrite: Vec::new(),
        runtime_path: "/runtime".to_string(),
        runtime_port: 0,
        runtime_max_concurrency: 16,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/skiff-host-router-proof".to_string(),
        },
        telemetry: None,
        profile_sampling: None,
        websocket_path: "/ws".to_string(),
    }
}

async fn start_router(
    fixture: &PublishedFixture,
    store: Arc<MemoryTaskStore>,
) -> (RouterSupervisor, SupervisorListeners) {
    let supervisor = RouterSupervisor::assemble_with_task_store(
        &router_config(fixture),
        Arc::clone(&store) as Arc<dyn TaskStore>,
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
    (supervisor, listeners)
}

fn start_host(control_port: u16, prefix: &str) -> (AbortOnDrop, RuntimeHome) {
    let home = RuntimeHome::new(prefix);
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: DbProviderSource::new(InMemoryDbProviderFactory::new()),
        router_url: format!("ws://127.0.0.1:{control_port}/runtime"),
        base_runtime_id: format!("{RUNTIME_ID}-{prefix}"),
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

async fn post(
    addr: std::net::SocketAddr,
    path: &str,
    service_id: &str,
    version: &str,
    body: &[u8],
) -> (u16, Vec<u8>) {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}{path}"))
        .header("X-Skiff-Service", service_id)
        .header("X-Skiff-Version", version)
        .body(body.to_vec())
        .send()
        .await
        .expect("HTTP request through production Router");
    let status = response.status().as_u16();
    let bytes = response.bytes().await.expect("HTTP response body").to_vec();
    (status, bytes)
}

async fn post_with_headers(
    addr: std::net::SocketAddr,
    path: &str,
    service_id: &str,
    version: &str,
    body: &[u8],
) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}{path}"))
        .header("X-Skiff-Service", service_id)
        .header("X-Skiff-Version", version)
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

async fn wait_for_records(store: &MemoryTaskStore) -> Vec<skiff_task_control::model::TaskRecord> {
    for _ in 0..200 {
        let records = store.records().await;
        if !records.is_empty() {
            return records;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Router accepted no durable task record");
}

async fn wait_for_status(
    supervisor: &RouterSupervisor,
    store: &MemoryTaskStore,
    task_id: &str,
    expected: TaskStatusKind,
) {
    for _ in 0..600 {
        let status = store
            .status(StatusInput {
                task_id: TaskId::new(task_id),
                retention: DurableDuration::from_millis(120_000),
            })
            .await
            .expect("task status");
        if status.kind == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "task {task_id} did not reach {expected:?}; records={:?}; dispatcher={:?}; counters={:?}; backlog={:?}",
        store.records().await,
        supervisor.components().dispatcher.health(),
        supervisor.components().task_control.counters(),
        supervisor.components().task_control.backlog().await,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_task_control::model::TaskState;

    #[tokio::test(flavor = "multi_thread")]
    async fn task_typed_json_unary_http_metadata_maps_router_response() {
        let fixture = published_positive(Capability::Task, "host-router-metadata");
        let store = Arc::new(MemoryTaskStore::new());
        let (supervisor, listeners) = start_router(&fixture, Arc::clone(&store)).await;
        let control_addr = listeners.runtime_control.addr();
        let public_addr = listeners.public_http.addr();
        let (_host, _home) = start_host(control_addr.port(), "metadata");
        wait_for_registered(&supervisor).await;

        let (status, headers, body) = post_with_headers(
            public_addr,
            "/phase-6/task",
            fixture.deployment.service_id.as_str(),
            fixture.deployment.contract_version.as_str(),
            b"7",
        )
        .await;
        assert_eq!(status, 200, "host HTTP terminal: {body:?}");
        assert!(!body.is_empty());
        assert!(
            headers.iter().any(|(name, value)| {
                name.eq_ignore_ascii_case("content-type")
                    && value == "application/json; charset=utf-8"
            }),
            "typedJson unary HTTP metadata must carry canonical JSON content-type: {headers:?}"
        );

        listeners.shutdown().await.expect("listeners shutdown");
        supervisor.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn task_host_router_proof_accepted_record_claim_fresh_request_and_settlement() {
        let fixture = published_positive(Capability::Task, "host-router-task");
        let store = Arc::new(MemoryTaskStore::new());
        let (supervisor, listeners) = start_router(&fixture, Arc::clone(&store)).await;
        let control_addr = listeners.runtime_control.addr();
        let public_addr = listeners.public_http.addr();
        let (_host, _home) = start_host(control_addr.port(), "task");
        wait_for_registered(&supervisor).await;

        let (status, body) = post(
            public_addr,
            "/phase-6/task",
            fixture.deployment.service_id.as_str(),
            fixture.deployment.contract_version.as_str(),
            b"7",
        )
        .await;
        assert_eq!(status, 200, "host HTTP terminal: {body:?}");
        assert!(!body.is_empty());

        let records = wait_for_records(&store).await;
        assert_eq!(records.len(), 1);
        let accepted = records[0].clone();
        assert_eq!(
            accepted.execution.deployment.deployment_artifact_identity,
            fixture.deployment.deployment_artifact_identity
        );
        assert_eq!(
            accepted.owner.as_str(),
            fixture.deployment.service_id.as_str()
        );
        assert!(!accepted.payload.as_bytes().is_empty());
        assert!(matches!(
            accepted.target,
            skiff_task_control::model::DetachedCallTarget::Function { .. }
        ));

        wait_for_status(
            &supervisor,
            &store,
            accepted.task_id.as_str(),
            TaskStatusKind::Succeeded,
        )
        .await;
        let terminal = store.records().await[0].clone();
        assert_eq!(terminal.state, TaskState::Succeeded);
        assert_eq!(terminal.active_lease, None);
        assert_eq!(terminal.attempt_generation, 1, "one production attempt");
        assert_eq!(
            terminal.execution.deployment.deployment_artifact_identity,
            fixture.deployment.deployment_artifact_identity
        );

        listeners.shutdown().await.expect("listeners shutdown");
        supervisor.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn actor_host_router_proof_accepted_actor_method_task_get_or_activate_and_settlement() {
        let fixture = match build_actor_method_task_fixture("host-router-actor") {
            BuildOutcome::Published(fixture) => fixture,
            BuildOutcome::Rejected { error_chain, .. } => {
                panic!("actor-method task fixture did not publish: {error_chain}")
            }
        };
        let store = Arc::new(MemoryTaskStore::new());
        let (supervisor, listeners) = start_router(&fixture, Arc::clone(&store)).await;
        let control_addr = listeners.runtime_control.addr();
        let public_addr = listeners.public_http.addr();
        let (_host, _home) = start_host(control_addr.port(), "actor");
        wait_for_registered(&supervisor).await;

        let (status, body) = post(
            public_addr,
            "/phase-6/task-actor-method",
            fixture.deployment.service_id.as_str(),
            fixture.deployment.contract_version.as_str(),
            b"7",
        )
        .await;
        assert_eq!(status, 200, "host HTTP terminal: {body:?}");

        let records = wait_for_records(&store).await;
        assert_eq!(records.len(), 1);
        let accepted = records[0].clone();
        assert!(matches!(
            accepted.target,
            skiff_task_control::model::DetachedCallTarget::ActorMethod { .. }
        ));
        assert_eq!(
            accepted.execution.deployment.deployment_artifact_identity,
            fixture.deployment.deployment_artifact_identity
        );
        assert!(!accepted.payload.as_bytes().is_empty());

        wait_for_status(
            &supervisor,
            &store,
            accepted.task_id.as_str(),
            TaskStatusKind::Succeeded,
        )
        .await;
        let terminal = store.records().await[0].clone();
        assert_eq!(terminal.state, TaskState::Succeeded);
        assert_eq!(terminal.active_lease, None);
        assert_eq!(terminal.attempt_generation, 1, "one actor task attempt");
        assert!(
            !supervisor
                .components()
                .actor
                .registry
                .owned_keys()
                .is_empty(),
            "Actor get-or-activate committed a real owner fence"
        );

        listeners.shutdown().await.expect("listeners shutdown");
        supervisor.shutdown().await;
    }
}

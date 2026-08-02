//! W-composition public test: `RouterSupervisor` full production assembly
//! over a real artifact root and the memory repository, the static consumer
//! manifest, the installed sink bundle, and a real-socket HTTP listener
//! wiring check.

use skiff_artifact_identity::runtime_assembly_ref;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::EnvironmentActivationState;
use skiff_deployment::fixtures::empty_runtime_assembly_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::activation::memory::MemoryActivationStateRepository;
use skiff_router::activation::ActivationStateRepository;
use skiff_router::config::RouterConfig;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::RouterSupervisor;
use skiff_runtime_config_snapshot::{RuntimeConfigSnapshot, RuntimeConfigSnapshotStore};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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

fn snapshot_ref() -> skiff_artifact_model::RuntimeConfigSnapshotRef {
    skiff_artifact_model::RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

struct RealChain {
    _root: TestRoot,
    assembly_ref: skiff_artifact_model::RuntimeAssemblyRef,
}

fn materialize(environment: &str) -> RealChain {
    let root = TestRoot::new();
    fs::create_dir_all(root.path()).expect("create artifact root");
    let snapshot_store = RuntimeConfigSnapshotStore::create(root.path().join("runtime-config"))
        .expect("create snapshot store");
    let snapshot = RuntimeConfigSnapshot::new(environment, snapshot_ref(), Vec::new())
        .expect("snapshot fixture");
    snapshot_store.publish(&snapshot).expect("publish snapshot");
    let artifact_store =
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    let assembly = empty_runtime_assembly_fixture().expect("assembly fixture");
    artifact_store
        .write_runtime_assembly(&assembly)
        .expect("write assembly");
    let assembly_ref = runtime_assembly_ref(&assembly).expect("assembly ref");

    let directory = root.path().join("records/actor-routing");
    fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
    fs::write(
        root.path().join("records/actor-routing/current.json"),
        bytes,
    )
    .expect("write projection record");
    RealChain {
        _root: root,
        assembly_ref,
    }
}

fn config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        activation_prepare_timeout_ms: 1_000,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        environment: Some("prod".to_string()),
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1_048_576,
        http_max_response_bytes: 1_048_576,
        http_port: 0,
        manifests: Vec::new(),
        profile: "dev".to_string(),
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

fn committed_state(chain: &RealChain) -> EnvironmentActivationState {
    EnvironmentActivationState::initial("prod", 7, chain.assembly_ref.clone(), snapshot_ref())
}

async fn raw_get(addr: std::net::SocketAddr, path: &str) -> (String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to public http");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Skiff-Service: example.com/docs\r\nX-Skiff-Version: example.com/docs@1\r\nConnection: close\r\n\r\n"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_assembles_full_composition_and_wires_public_http() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        repository
            .initialize(&committed_state(&chain))
            .await
            .expect("seed committed state");
        let config = config(chain._root.path());

        let supervisor = RouterSupervisor::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await
        .expect("production composition must assemble");

        let components = supervisor.components();
        // Static consumer manifest: exactly the installed session-keyed owners.
        assert_eq!(
            components.session.manifest_kinds(),
            vec![
                skiff_router::session::ConsumerKind::HealthLedger,
                skiff_router::session::ConsumerKind::RequestDispatcher,
                skiff_router::session::ConsumerKind::RuntimeGenerationPinLedger,
                skiff_router::session::ConsumerKind::WebSocketRequestBroker,
                skiff_router::session::ConsumerKind::ActorSessionOwner,
                skiff_router::session::ConsumerKind::ActivationCoordinator,
            ]
        );
        // Installed sink bundle: request/connection/activation/actor/spawn
        // all wired (E-actor-rust installed the real spawn inbound sink).
        let sinks = components.session.inbound_sinks();
        assert!(sinks.request.is_some());
        assert!(sinks.connection.is_some());
        assert!(sinks.activation_transaction.is_some());
        assert!(sinks.actor.is_some());
        assert!(sinks.spawn.is_some());

        // Components are live and empty.
        assert_eq!(components.dispatcher.pending_count(), 0);
        assert_eq!(
            components.coordinator.phase(),
            skiff_router::activation::ActivationPhase::Idle
        );
        assert_eq!(
            components.actor.catalog_view.schema_version(),
            ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
        );
        assert!(components.surface_view.is_empty());
        assert_eq!(components.pending_http.pending_count(), 0);

        // Real-socket wiring: public HTTP is served by the production adapter
        // through the ingress resolver (empty fixture -> 404 platform error).
        let listeners = supervisor
            .start_listeners(&ListenerStartOptions::default())
            .await
            .expect("listeners start");
        let http_addr = listeners.public_http.addr();
        let (status, body) = raw_get(http_addr, "/docs").await;
        assert!(
            status.contains("404"),
            "expected platform 404 for missing assembly ingress, got {status:?}"
        );
        assert!(body.contains("AssemblyIngressNotFound"));

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn supervisor_fails_closed_without_committed_epoch() {
        let chain = materialize("prod");
        let repository = Arc::new(MemoryActivationStateRepository::new());
        let config = config(chain._root.path());
        let result = RouterSupervisor::assemble_with(
            &config,
            "prod",
            Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
        )
        .await;
        assert!(
            result.is_err(),
            "missing durable committed state must fail closed"
        );
    }
}

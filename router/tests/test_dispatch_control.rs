//! Real-socket probe for the `POST /__skiff/test-dispatch` control endpoint
//! (plan §7 E-http test-dispatch isolation).
//!
//! The probe assembles the full production `RouterSupervisor` over the
//! memory activation repository and a real artifact root containing a
//! deployment with one exact HTTP gateway binding, then drives the
//! runtime/control listener over a real socket. No Runtime is registered in
//! this probe, so a valid exact test dispatch reaches the production
//! dispatcher and fails closed with the TS-parity `AssemblyParticipantsUnavailable`
//! control error; decode/binding/method failures and the public-gateway
//! isolation are asserted on the same stack.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use skiff_artifact_identity::{assign_runtime_assembly_identity, runtime_assembly_ref};
use skiff_artifact_model::{
    GatewayEntryKey, GatewayIngressBinding, IngressProtocol, IngressSelector,
    RuntimeAssemblyRef, RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};
use skiff_deployment::activation_state::EnvironmentActivationState;
use skiff_deployment::fixtures::{runtime_assembly_fixture, service_deployment_fixture};
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const ENVIRONMENT: &str = "prod";
const GENERATION: u64 = 7;
const GATEWAY_KEY: &str = "echo";
const ECHO_PATH: &str = "/echo";

struct TestRoot {
    parent: PathBuf,
    root: PathBuf,
}

impl TestRoot {
    fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-test-dispatch-control-{}-{sequence}",
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

fn snapshot_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect("snapshot id"),
    }
}

struct RealChain {
    _root: TestRoot,
    assembly_ref: RuntimeAssemblyRef,
    deployment: ServiceDeploymentRef,
    gateway_entry_identity: String,
}

fn materialize() -> RealChain {
    let root = TestRoot::new();
    fs::create_dir_all(root.path()).expect("create artifact root");
    let snapshot_store = RuntimeConfigSnapshotStore::create(root.path().join("runtime-config"))
        .expect("create snapshot store");
    let snapshot = RuntimeConfigSnapshot::new(ENVIRONMENT, snapshot_ref(), Vec::new())
        .expect("snapshot fixture");
    snapshot_store.publish(&snapshot).expect("publish snapshot");
    let artifact_store =
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    let deployment = service_deployment_fixture().expect("deployment fixture");
    let deployment_ref = ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    };
    let gateway_entry = deployment
        .gateway_entries
        .get(&GatewayEntryKey::parse(GATEWAY_KEY).expect("gateway key"))
        .expect("fixture gateway entry");
    let mut assembly = runtime_assembly_fixture().expect("assembly fixture");
    assembly.gateway_ingress = vec![GatewayIngressBinding {
        selector: IngressSelector {
            protocol: IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: ECHO_PATH.to_string(),
        },
        deployment: deployment_ref.clone(),
        gateway_entry_key: GatewayEntryKey::parse(GATEWAY_KEY).expect("gateway key"),
        gateway_entry_identity: gateway_entry.gateway_entry_identity.clone(),
    }];
    assign_runtime_assembly_identity(&mut assembly).expect("assign assembly identity");
    artifact_store
        .write_runtime_assembly(&assembly)
        .expect("write assembly");
    artifact_store
        .write_service_deployment(&deployment)
        .expect("write deployment");
    let assembly_ref = runtime_assembly_ref(&assembly).expect("assembly ref");

    let directory = root.path().join("records/actor-routing");
    fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
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
    RealChain {
        _root: root,
        assembly_ref,
        deployment: deployment_ref,
        gateway_entry_identity: gateway_entry.gateway_entry_identity.as_str().to_string(),
    }
}

fn config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        activation_prepare_timeout_ms: 1_000,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
        environment: Some(ENVIRONMENT.to_string()),
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
    EnvironmentActivationState::initial(
        ENVIRONMENT,
        GENERATION,
        chain.assembly_ref.clone(),
        snapshot_ref(),
    )
}

fn test_dispatch_body(chain: &RealChain) -> serde_json::Value {
    serde_json::json!({
        "kind": "test",
        "routing": {
            "kind": "runtimeAssembly",
            "assemblyIdentity": chain.assembly_ref.assembly_identity.clone(),
            "assemblyGeneration": GENERATION,
            "deployment": {
                "serviceId": chain.deployment.service_id.clone(),
                "contractVersion": chain.deployment.contract_version.clone(),
                "deploymentRevision": chain.deployment.deployment_revision.clone(),
                "deploymentArtifactIdentity": chain.deployment.deployment_artifact_identity.clone(),
            },
            "gatewayEntryIdentity": chain.gateway_entry_identity.clone(),
            "ingress": {
                "protocol": "http",
                "method": "POST",
                "path": ECHO_PATH,
            },
        },
        "mode": "unary",
        "httpRequest": {
            "method": "POST",
            "url": "http://127.0.0.1/echo",
            "path": ECHO_PATH,
            "query": [],
            "headers": [{
                "name": "content-type",
                "value": "application/json",
            }],
        },
        "payloadBase64": "bnVsbA==",
        "timeoutMs": 30_000,
    })
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

async fn start_stack() -> (
    RealChain,
    RouterSupervisor,
    skiff_router::supervisor::SupervisorListeners,
) {
    let chain = materialize();
    let repository = Arc::new(MemoryActivationStateRepository::new());
    repository
        .initialize(&committed_state(&chain))
        .await
        .expect("seed committed state");
    let config = config(chain._root.path());
    let supervisor = RouterSupervisor::assemble_with(
        &config,
        ENVIRONMENT,
        Arc::clone(&repository) as Arc<dyn ActivationStateRepository>,
    )
    .await
    .expect("production composition must assemble");
    let listeners = supervisor
        .start_listeners(&ListenerStartOptions::default())
        .await
        .expect("listeners start");
    (chain, supervisor, listeners)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_listener_serves_test_dispatch_over_a_real_socket() {
        let (chain, supervisor, listeners) = start_stack().await;
        let control_addr = listeners.runtime_control.addr();

        // Non-POST -> 405 with allow: POST (TS parity).
        let (status, body) =
            raw_request(control_addr, "GET", "/__skiff/test-dispatch", b"").await;
        assert!(status.contains("405"), "expected 405, got {status:?}");
        assert!(
            body.to_ascii_lowercase().contains("allow: post"),
            "405 must advertise POST, got {body:?}"
        );
        assert!(body.contains("MethodNotAllowed"));

        // Malformed JSON -> 400 AssemblyActivationRejected (TS classify).
        let (status, body) =
            raw_request(control_addr, "POST", "/__skiff/test-dispatch", b"not json").await;
        assert!(status.contains("400"), "expected 400, got {status:?}");
        assert!(body.contains("AssemblyActivationRejected"));

        // Unknown field -> 409 (TS classify default).
        let mut invalid = test_dispatch_body(&chain);
        invalid["extra"] = serde_json::json!(1);
        let bytes = serde_json::to_vec(&invalid).expect("body serializes");
        let (status, body) =
            raw_request(control_addr, "POST", "/__skiff/test-dispatch", &bytes).await;
        assert!(status.contains("409"), "expected 409, got {status:?}");
        assert!(body.contains("AssemblyActivationRejected"));

        // Generation mismatch -> 409.
        let mut stale = test_dispatch_body(&chain);
        stale["routing"]["assemblyGeneration"] = serde_json::json!(GENERATION + 1);
        let bytes = serde_json::to_vec(&stale).expect("body serializes");
        let (status, body) =
            raw_request(control_addr, "POST", "/__skiff/test-dispatch", &bytes).await;
        assert!(status.contains("409"), "expected 409, got {status:?}");
        assert!(body.contains("does not match the exact active assembly generation"));

        // Exact valid dispatch reaches the production dispatcher; with no
        // registered Runtime it fails closed as 503 AssemblyParticipantsUnavailable.
        let bytes = serde_json::to_vec(&test_dispatch_body(&chain)).expect("body serializes");
        let (status, body) =
            raw_request(control_addr, "POST", "/__skiff/test-dispatch", &bytes).await;
        assert!(status.contains("503"), "expected 503, got {status:?}");
        assert!(body.contains("AssemblyParticipantsUnavailable"));

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }

    #[tokio::test]
    async fn public_gateway_isolates_test_dispatch_from_business_ingress() {
        let (_chain, supervisor, listeners) = start_stack().await;
        let public_addr = listeners.public_http.addr();
        let bytes = b"{}";
        let (status, body) =
            raw_request(public_addr, "POST", "/__skiff/test-dispatch", bytes).await;
        assert!(status.contains("404"), "expected 404, got {status:?}");
        assert!(body.contains("ControlEndpointNotFound"));

        listeners
            .shutdown()
            .await
            .expect("listeners shut down cleanly");
        supervisor.shutdown().await;
    }
}

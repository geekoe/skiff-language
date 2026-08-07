//! Shared helpers for the batch 12 health real-socket probes.
//!
//! The production supervisor is assembled over a real artifact root and the
//! memory activation repository; `/__router/health` is exercised over a real
//! socket, and a fake Runtime peer drives the `/runtime` handshake
//! (bootstrap/capabilities/Register/ACK/health) like the session probes.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::fixtures::service_deployment_fixture;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_router::config::RouterConfig;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::RouterSupervisor;
use skiff_runtime_transport::protocol::{
    encode_binary_frame, RuntimeCapabilitiesFrameHeader, RuntimeCapabilitiesFrameHeaderMetadata,
    RuntimeDispatchModeCapability, RuntimeHealthCountersFrameHeader, RuntimeHealthFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::timeout;

pub const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
pub const PROFILE: &str = "prod";
pub const REPLICA_A: &str = "runtime-health-a";
pub const REPLICA_B: &str = "runtime-health-b";

static TEMP_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct TestRoot {
    parent: PathBuf,
    pub root: PathBuf,
}

impl TestRoot {
    pub fn new() -> Self {
        let sequence = TEMP_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let parent = std::env::temp_dir().join(format!(
            "skiff-router-health-{}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&parent).expect("create temp parent");
        Self {
            parent: parent.clone(),
            root: parent.join("root"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.parent);
    }
}

pub struct RealChain {
    pub _root: TestRoot,
    pub build_id: String,
}

pub fn materialize(profile: &str) -> RealChain {
    let root = TestRoot::new();
    std::fs::create_dir_all(root.path()).expect("create artifact root");
    let artifact_store =
        CanonicalArtifactStore::create(root.path()).expect("create artifact store");
    let mut deployment = service_deployment_fixture().expect("deployment fixture");
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
        .expect("assign deployment identity");
    artifact_store
        .write_service_deployment(&deployment)
        .expect("write deployment record");
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let release_pointer =
        skiff_deployment::storage::ReleasePointer::new(profile, reference.clone())
            .expect("release pointer");
    artifact_store
        .write_release_pointer(&release_pointer)
        .expect("write release pointer");
    let build_id = reference.deployment_artifact_identity.to_string();
    let directory = root.path().join("records/actor-routing");
    std::fs::create_dir_all(&directory).expect("create actor routing records directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
    std::fs::write(
        root.path().join("records/actor-routing/current.json"),
        bytes,
    )
    .expect("write projection record");
    RealChain {
        _root: root,
        build_id,
    }
}

pub fn config(artifact_root: &Path) -> RouterConfig {
    RouterConfig {
        run_dir: None,
        artifacts_path: artifact_root.to_path_buf(),
        dev_reload: None,
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

pub async fn assemble(chain: &RealChain) -> RouterSupervisor {
    let config = config(chain._root.path());
    RouterSupervisor::assemble(&config)
        .await
        .expect("production composition must assemble")
}

pub async fn start_listeners(
    supervisor: &RouterSupervisor,
) -> skiff_router::supervisor::SupervisorListeners {
    supervisor
        .start_listeners(&ListenerStartOptions::default())
        .await
        .expect("listeners start")
}

pub async fn raw_request(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> (String, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect to control listener");
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

pub async fn health_json(addr: std::net::SocketAddr, path: &str) -> (String, Value) {
    let (status, body) = raw_request(addr, "GET", path, b"").await;
    let json = body
        .split_once("\r\n\r\n")
        .map(|(_, json)| json)
        .unwrap_or(&body);
    let value: Value = serde_json::from_str(json)
        .unwrap_or_else(|error| panic!("health response is not JSON: {error}: {json:?}"));
    (status, value)
}

pub type PeerSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub async fn connect_runtime(addr: std::net::SocketAddr) -> PeerSocket {
    let url = format!("ws://{addr}/runtime");
    let (socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .expect("WS handshake timed out")
        .expect("WS handshake failed");
    assert_eq!(response.status(), 101);
    socket
}

pub async fn recv_binary(socket: &mut PeerSocket) -> Vec<u8> {
    match timeout(CLIENT_TIMEOUT, socket.next()).await {
        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)))) => bytes.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

pub async fn send_binary(socket: &mut PeerSocket, bytes: Vec<u8>) {
    timeout(
        CLIENT_TIMEOUT,
        socket.send(tokio_tungstenite::tungstenite::Message::Binary(
            bytes.into(),
        )),
    )
    .await
    .expect("send timed out")
    .expect("send failed");
}

pub fn capabilities_bytes(replica_id: &str) -> Vec<u8> {
    let header = RuntimeCapabilitiesFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.capabilities".to_string(),
        runtime_id: replica_id.to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            dispatch_modes: vec![RuntimeDispatchModeCapability::Unary],
            package_test_dispatch: false,
            request_cancel: true,
            runtime_program: false,
            artifact_root: None,
            lazy_load: false,
            loaded_build_ids: Vec::new(),
        },
    };
    encode_binary_frame(&header, &[]).expect("capabilities encodes")
}

pub fn health_bytes(replica_id: &str) -> Vec<u8> {
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let observed_at = format_iso(now);
    let header = RuntimeHealthFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.health".to_string(),
        runtime_id: replica_id.to_string(),
        observed_at,
        counters: RuntimeHealthCountersFrameHeader {
            outbound_requests_pending: 0,
            outbound_stream_leases_active: 0,
            stream_runtime_streams_active: 0,
            flag_backed_cancel_waiters_active: 0,
            task_requests_active: 0,
        },
    };
    encode_binary_frame(&header, &[]).expect("health encodes")
}

/// Completes the runtime handshake (M4): reads `router.bootstrap`, sends
/// capabilities (the registration), and reads the `runtime.registered` ACK.
pub async fn complete_handshake(socket: &mut PeerSocket, replica_id: &str, _chain: &RealChain) {
    let bootstrap = recv_binary(socket).await;
    assert!(
        bootstrap
            .windows(b"router.bootstrap".len())
            .any(|window| window == b"router.bootstrap"),
        "expected router.bootstrap frame"
    );
    send_binary(socket, capabilities_bytes(replica_id)).await;
    let ack = recv_binary(socket).await;
    assert!(
        ack.windows(b"runtime.registered".len())
            .any(|window| window == b"runtime.registered"),
        "expected runtime.registered ACK"
    );
}

/// Polls the health endpoint until the predicate accepts the payload.
pub async fn wait_until_health(
    addr: std::net::SocketAddr,
    path: &str,
    mut predicate: impl FnMut(&Value) -> bool,
) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let (_, health) = health_json(addr, path).await;
        if predicate(&health) {
            return health;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "wait_until_health deadline exceeded; last payload: {health}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Minimal UTC ISO formatting for the fake runtime health frame (mirrors the
/// router projection formatter shape; no crate dependency).
fn format_iso(millis: u64) -> String {
    let days = millis / 86_400_000;
    let (year, month, day) = civil_from_days(days as i64);
    let remainder = millis % 86_400_000;
    let hour = remainder / 3_600_000;
    let minute = (remainder % 3_600_000) / 60_000;
    let second = (remainder % 60_000) / 1_000;
    let ms = remainder % 1_000;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{ms:03}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

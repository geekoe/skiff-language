//! `router-live:actor` real-boundary probe (E-actor-rust gate, plan §7/§8).
//!
//! Driven by `scripts/check-router-actor-live.mjs`: the harness authors a
//! real compiler artifact (actor-full-chain-acceptance fixture), starts an
//! isolated temporary Mongo replica set, leases router + two relay ports and
//! builds explicit Rust router/runtime binaries. This ignored test then:
//!   - seeds the actor routing projection record and spawns the real
//!     `skiff-router` binary (M4: the release pointer table comes from
//!     authoring, no activation state is seeded);
//!   - tasks two real `runtime` binaries with independent runtime homes,
//!     each connected through a test-only WS relay to the real Router;
//!   - drives HTTP unary probes through the real Router into the fixture:
//!     ownership claim token / activation broker (get-or-create dedup),
//!     invocation relay, owner control, lease scheduler;
//!   - proves function task and actor-method task parent authority and that
//!     accepted tasks outlive the parent lifecycle;
//!   - exercises disconnect/replacement/concurrent claim/task mismatch fail
//!     closed and asserts frame pairing (invocation/control/lease/timer zero
//!     residue) plus graceful shutdown.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH;
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_response_end_frame, decode_response_error_frame,
    decode_task_submit_request_frame, decode_typed_binary_frame, encode_binary_frame,
    encode_task_submit_request_frame, RuntimeCapabilitiesFrameHeader, RuntimeHealthFrameHeader,
    RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
    RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestNameValueFrameHeader,
    RuntimeAssemblyRequestRoutingFrameHeader, RuntimeAssemblyRequestStartFrameHeader,
    RuntimeAssemblyRequestTraceFrameHeader,
};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

const REPLICA_ONE_ID: &str = "actor-live-replica-1";
const REPLICA_TWO_ID: &str = "actor-live-replica-2";
const LIVE_TIMEOUT: Duration = Duration::from_secs(180);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENCY: u64 = 16;

const HANDSHAKE_SEQUENCE: [&str; 4] = [
    "router.bootstrap",
    "runtime.capabilities",
    "runtime.registered",
    "runtime.health",
];

struct LiveProfile {
    mongo_url: String,
    database: String,
    artifact_root: PathBuf,
    profile: String,
    http_port: u16,
    runtime_port: u16,
    relay_one_port: u16,
    relay_two_port: u16,
    runtime_bin: PathBuf,
    runtime_one_home: PathBuf,
    runtime_two_home: PathBuf,
    temp_dir: PathBuf,
    entrypoints: BTreeMap<String, Entrypoint>,
    deployment: skiff_artifact_model::ServiceDeploymentRef,
}

#[derive(Debug, Clone)]
struct Entrypoint {
    path: String,
    method: String,
    gateway_entry_identity: String,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-router-actor-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_ACTOR_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let relay_one_port = required("SKIFF_ROUTER_ACTOR_LIVE_RELAY_ONE_PORT")
            .parse()
            .expect("relay one port");
        let relay_two_port = required("SKIFF_ROUTER_ACTOR_LIVE_RELAY_TWO_PORT")
            .parse()
            .expect("relay two port");
        let entrypoints_json = required("SKIFF_ROUTER_ACTOR_LIVE_ENTRYPOINTS");
        let deployment_json = required("SKIFF_ROUTER_ACTOR_LIVE_DEPLOYMENT");
        let deployment_raw: serde_json::Value =
            serde_json::from_str(&deployment_json).expect("deployment JSON");
        let deployment = skiff_artifact_model::ServiceDeploymentRef {
            service_id: deployment_raw["serviceId"]
                .as_str()
                .expect("deployment serviceId")
                .to_string(),
            contract_version: deployment_raw["contractVersion"]
                .as_str()
                .expect("deployment contractVersion")
                .to_string(),
            deployment_revision: skiff_artifact_model::DeploymentRevision::new(
                deployment_raw["deploymentRevision"]
                    .as_str()
                    .expect("deployment revision"),
            ),
            deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
                deployment_raw["deploymentArtifactIdentity"]
                    .as_str()
                    .expect("deployment artifact identity"),
            ),
        };
        let raw_entrypoints: Vec<serde_json::Value> =
            serde_json::from_str(&entrypoints_json).expect("entrypoints JSON");
        let mut entrypoints = BTreeMap::new();
        for raw in raw_entrypoints {
            let key = raw["gatewayEntryKey"]
                .as_str()
                .expect("entrypoint gatewayEntryKey")
                .to_string();
            let selector = &raw["selector"];
            let gateway_entry_identity = raw["gatewayEntryIdentity"]
                .as_str()
                .expect("entrypoint gatewayEntryIdentity")
                .to_string();
            entrypoints.insert(
                key,
                Entrypoint {
                    path: selector["path"]
                        .as_str()
                        .expect("entrypoint path")
                        .to_string(),
                    method: selector["method"].as_str().unwrap_or("POST").to_string(),
                    gateway_entry_identity,
                },
            );
        }
        Self {
            mongo_url: required("SKIFF_ROUTER_ACTOR_LIVE_MONGO_URL"),
            database: required("SKIFF_ROUTER_ACTOR_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_ACTOR_LIVE_ARTIFACT_ROOT")),
            profile: required("SKIFF_ROUTER_ACTOR_LIVE_PROFILE"),
            http_port,
            runtime_port,
            relay_one_port,
            relay_two_port,
            runtime_bin: PathBuf::from(required("SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_BIN")),
            runtime_one_home: PathBuf::from(required("SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_ONE_HOME")),
            runtime_two_home: PathBuf::from(required("SKIFF_ROUTER_ACTOR_LIVE_RUNTIME_TWO_HOME")),
            temp_dir: PathBuf::from(required("SKIFF_ROUTER_ACTOR_LIVE_TEMP_DIR")),
            entrypoints,
            deployment,
        }
    }

    fn entrypoint(&self, key: &str) -> Entrypoint {
        self.entrypoints
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("missing fixture entrypoint {key}"))
    }

    fn router_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.runtime_port)
    }

    fn relay_runtime_url(&self, port: u16) -> String {
        format!("ws://127.0.0.1:{port}/runtime")
    }

    fn deployment_ref(&self) -> skiff_artifact_model::ServiceDeploymentRef {
        self.deployment.clone()
    }
}

fn seed_runtime_home(home: &Path, replica_id: &str) {
    std::fs::create_dir_all(home).expect("create runtime home");
    std::fs::write(home.join("runtime-id"), format!("{replica_id}\n")).expect("seed runtime-id");
}

fn write_router_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-actor-{}-{}.yml",
        live.http_port, live.runtime_port
    ));
    let contents = format!(
        "profile: {}\n\
         host: 127.0.0.1\n\
         artifactsPath: {}\n\
         releaseMode: true\n\
         requestTimeoutMs: 30000\n\
         http:\n  port: {}\n  maxRequestBytes: 1048576\n  maxResponseBytes: 1048576\n\
         runtime:\n  port: {}\n  path: /runtime\n  maxConcurrency: {MAX_CONCURRENCY}\n\
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

fn write_runtime_config(live: &LiveProfile, relay_port: u16, home: &Path) -> PathBuf {
    let path = live.temp_dir.join(format!("runtime-{relay_port}.yml"));
    let contents = format!(
        "router: {}\n\
         runtime-home: {}\n",
        live.relay_runtime_url(relay_port),
        home.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
}

fn task_router(config_path: &Path) -> Child {
    let stderr_path = config_path.with_extension("router.stderr.log");
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stderr_path)
        .expect("open router stderr log");
    Command::new(env!("CARGO_BIN_EXE_skiff-router"))
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .expect("spawn skiff-router")
}

fn spawn_runtime(live: &LiveProfile, config_path: &Path, label: &str) -> Child {
    let stdout_path = live.temp_dir.join(format!("{label}.stdout.log"));
    let stderr_path = live.temp_dir.join(format!("{label}.stderr.log"));
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stdout_path)
        .expect("open runtime stdout log");
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stderr_path)
        .expect("open runtime stderr log");
    Command::new(&live.runtime_bin)
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("spawn runtime")
}

fn wait_for_exit(
    child: &mut Child,
    deadline: Duration,
    label: &str,
) -> (std::process::ExitStatus, String) {
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    let deadline = Instant::now() + deadline;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return (status, stderr),
            Ok(None) => {}
            Err(error) => panic!("wait for {label} failed: {error}"),
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{label} did not exit within deadline; stderr: {stderr}");
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

fn materialize_projection(live: &LiveProfile) {
    // E-actor-parity: the harness writes the canonical actor-routing
    // projection record (test-side A1 producer) before invoking this probe.
    // The probe must consume that exact record rather than overwrite it with
    // an empty projection, otherwise the Rust router would no longer match
    // the TS A2 hard-cut admission semantics.
    let path = live
        .artifact_root
        .join(ACTOR_ROUTING_PROJECTION_RECORD_PATH);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        panic!("actor routing projection record missing at {path:?}: {error}")
    });
    let projection: ActorRoutingProjection =
        serde_json::from_slice(&bytes).expect("projection record must decode");
    assert_eq!(
        projection.schema_version,
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION
    );
    assert!(
        !projection.methods.is_empty(),
        "actor routing projection must carry the fixture actor methods"
    );
}

fn frame_type(bytes: &[u8]) -> String {
    decode_binary_frame(bytes)
        .ok()
        .and_then(|frame| {
            frame
                .header
                .get("type")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "undecodable".to_string())
}

// ---------------------------------------------------------------------------
// Test-only WS relay hub: real Runtime process <-> relay <-> real Router
// binary, one relay listener per replica, with test-origin injection and
// pair drop for disconnect/replacement scenarios.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    ToRouter,
    ToRuntime,
}

#[derive(Debug, Clone)]
enum RecordKind {
    Frame {
        direction: Direction,
        frame_type: String,
        bytes: Vec<u8>,
    },
    PairClosed,
}

#[derive(Debug, Clone)]
struct RelayRecord {
    connection: u64,
    kind: RecordKind,
}

#[derive(Debug, Clone)]
struct PairHandle {
    connection: u64,
    tasks: Vec<tokio::task::AbortHandle>,
}

type RelayInjectors = BTreeMap<
    u64,
    (
        mpsc::UnboundedSender<Message>,
        mpsc::UnboundedSender<Message>,
    ),
>;

struct RelayState {
    records: Mutex<Vec<RelayRecord>>,
    pairs: Mutex<Vec<PairHandle>>,
    injectors: Mutex<RelayInjectors>,
    next_connection: AtomicU64,
}

impl RelayState {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            pairs: Mutex::new(Vec::new()),
            injectors: Mutex::new(BTreeMap::new()),
            next_connection: AtomicU64::new(0),
        }
    }

    fn record_frame(&self, connection: u64, direction: Direction, bytes: Vec<u8>) {
        let frame_type = frame_type(&bytes);
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RelayRecord {
                connection,
                kind: RecordKind::Frame {
                    direction,
                    frame_type,
                    bytes,
                },
            });
    }

    fn register_injector(
        &self,
        connection: u64,
        to_router: mpsc::UnboundedSender<Message>,
        to_runtime: mpsc::UnboundedSender<Message>,
    ) {
        self.injectors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(connection, (to_router, to_runtime));
    }

    fn inject(&self, connection: u64, direction: Direction, bytes: Vec<u8>) {
        let injectors = self
            .injectors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&connection)
            .cloned();
        if let Some((to_router, to_runtime)) = injectors {
            let injector = match direction {
                Direction::ToRouter => to_router,
                Direction::ToRuntime => to_runtime,
            };
            let _ = injector.send(Message::Binary(bytes.into()));
        }
    }

    fn register_pair(&self, connection: u64, tasks: &[tokio::task::JoinHandle<()>]) {
        self.pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(PairHandle {
                connection,
                tasks: tasks.iter().map(|task| task.abort_handle()).collect(),
            });
    }

    fn drop_pair(&self, connection: u64) {
        let handle = self
            .pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|handle| handle.connection == connection)
            .cloned();
        if let Some(handle) = handle {
            for task in handle.tasks {
                task.abort();
            }
        }
    }

    fn record_pair_closed(&self, connection: u64) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RelayRecord {
                connection,
                kind: RecordKind::PairClosed,
            });
        self.pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|handle| handle.connection != connection);
        self.injectors
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&connection);
    }
}

async fn relay_listen(port: u16, router_url: String, state: Arc<RelayState>) {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind relay listener");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let state = Arc::clone(&state);
        let router_url = router_url.clone();
        tokio::spawn(async move {
            relay_pair(stream, router_url, state).await;
        });
    }
}

async fn relay_pair(
    server_socket: tokio::net::TcpStream,
    router_url: String,
    state: Arc<RelayState>,
) {
    let connection = state.next_connection.fetch_add(1, Ordering::Relaxed) + 1;
    let server = match tokio_tungstenite::accept_async(server_socket).await {
        Ok(server) => server,
        Err(_) => return,
    };
    let (client, _response) = match tokio_tungstenite::connect_async(&router_url).await {
        Ok(connected) => connected,
        Err(_) => return,
    };
    let (mut server_writer, mut server_reader) = server.split();
    let (mut client_writer, mut client_reader) = client.split();
    let (to_router_inject_tx, mut to_router_inject_rx) = mpsc::unbounded_channel::<Message>();
    let (to_runtime_inject_tx, mut to_runtime_inject_rx) = mpsc::unbounded_channel::<Message>();
    state.register_injector(connection, to_router_inject_tx, to_runtime_inject_tx);

    let to_router_state = Arc::clone(&state);
    let to_runtime_state = Arc::clone(&state);
    let to_router = tokio::spawn(async move {
        // Runtime -> Router frames plus test-origin injections share the
        // single Router-bound writer (one writer per socket direction).
        loop {
            tokio::select! {
                item = server_reader.next() => {
                    let Some(item) = item else { break };
                    let message = match item {
                        Ok(message) => message,
                        Err(_) => break,
                    };
                    if let Message::Binary(bytes) = &message {
                        to_router_state.record_frame(connection, Direction::ToRouter, bytes.to_vec());
                    }
                    if matches!(message, Message::Close(_)) {
                        let _ = client_writer.send(Message::Close(None)).await;
                        break;
                    }
                    if client_writer.send(message).await.is_err() {
                        break;
                    }
                }
                message = to_router_inject_rx.recv() => {
                    let Some(message) = message else { break };
                    if let Message::Binary(bytes) = &message {
                        to_router_state.record_frame(connection, Direction::ToRouter, bytes.to_vec());
                    }
                    if client_writer.send(message).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = client_writer.send(Message::Close(None)).await;
    });
    let to_runtime = tokio::spawn(async move {
        loop {
            tokio::select! {
                item = client_reader.next() => {
                    let Some(item) = item else { break };
                    let message = match item {
                        Ok(message) => message,
                        Err(_) => break,
                    };
                    if let Message::Binary(bytes) = &message {
                        to_runtime_state.record_frame(connection, Direction::ToRuntime, bytes.to_vec());
                    }
                    if matches!(message, Message::Close(_)) {
                        let _ = server_writer.send(Message::Close(None)).await;
                        break;
                    }
                    if server_writer.send(message).await.is_err() {
                        break;
                    }
                }
                message = to_runtime_inject_rx.recv() => {
                    let Some(message) = message else { break };
                    if let Message::Binary(bytes) = &message {
                        to_runtime_state.record_frame(connection, Direction::ToRuntime, bytes.to_vec());
                    }
                    if server_writer.send(message).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = server_writer.send(Message::Close(None)).await;
    });
    let tasks = vec![to_router, to_runtime];
    state.register_pair(connection, &tasks);
    let mut tasks = tasks.into_iter();
    let mut to_router = tasks.next().expect("relay pump task");
    let mut to_runtime = tasks.next().expect("relay pump task");
    let mut to_router_done = false;
    let mut to_runtime_done = false;
    tokio::select! {
        _ = &mut to_router => {
            to_router_done = true;
            to_runtime.abort();
        }
        _ = &mut to_runtime => {
            to_runtime_done = true;
            to_router.abort();
        }
    }
    if !to_router_done {
        let _ = to_router.await;
    }
    if !to_runtime_done {
        let _ = to_runtime.await;
    }
    state.record_pair_closed(connection);
}

fn relay_records(state: &Arc<RelayState>) -> Vec<RelayRecord> {
    state
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn contains_sequence(types: &[&str], sequence: &[&str]) -> bool {
    if sequence.is_empty() {
        return true;
    }
    let mut index = 0;
    for observed in types {
        if *observed == sequence[index] {
            index += 1;
            if index == sequence.len() {
                return true;
            }
        }
    }
    false
}

async fn wait_for_pair_closed(state: &Arc<RelayState>, connection: u64) {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = relay_records(state);
        if records.iter().any(|record| {
            record.connection == connection && matches!(record.kind, RecordKind::PairClosed)
        }) {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("relay pair {connection} did not close");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_frame_type(
    state: &Arc<RelayState>,
    frame_type: &str,
    after_records: usize,
) -> Vec<u8> {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = relay_records(state);
        for record in records.iter().skip(after_records) {
            if let RecordKind::Frame {
                direction: Direction::ToRouter,
                frame_type: observed,
                bytes,
            } = &record.kind
            {
                if observed == frame_type {
                    return bytes.clone();
                }
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("no {frame_type} frame observed after record {after_records}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits until a full handshake exists for `expected_replica` on any
/// connection newer than `after_records`; returns that connection id.
async fn wait_for_replica_handshake(
    _live: &LiveProfile,
    state: &Arc<RelayState>,
    expected_replica: &str,
    after_records: usize,
) -> u64 {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = relay_records(state);
        let mut connections = BTreeMap::new();
        for record in records.iter().skip(after_records) {
            let RelayRecord { connection, kind } = record;
            if let RecordKind::Frame {
                direction: Direction::ToRouter,
                frame_type: observed,
                bytes,
            } = kind
            {
                if observed != "runtime.capabilities" {
                    continue;
                }
                let Ok((header, _)) =
                    decode_typed_binary_frame::<RuntimeCapabilitiesFrameHeader>(bytes)
                else {
                    continue;
                };
                connections.insert(header.runtime_id, *connection);
            }
        }
        if let Some(connection) = connections.get(expected_replica) {
            let types: Vec<&str> = records
                .iter()
                .filter(|record| {
                    record.connection == *connection
                        && matches!(record.kind, RecordKind::Frame { .. })
                })
                .filter_map(|record| match &record.kind {
                    RecordKind::Frame { frame_type, .. } => Some(frame_type.as_str()),
                    RecordKind::PairClosed => None,
                })
                .collect();
            if contains_sequence(&types, &HANDSHAKE_SEQUENCE) {
                return *connection;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("replica {expected_replica} handshake was not observed");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_two_handshakes(
    live: &LiveProfile,
    state: &Arc<RelayState>,
) -> BTreeMap<String, u64> {
    let one = wait_for_replica_handshake(live, state, REPLICA_ONE_ID, 0).await;
    let two = wait_for_replica_handshake(live, state, REPLICA_TWO_ID, 0).await;
    let mut map = BTreeMap::new();
    map.insert(REPLICA_ONE_ID.to_string(), one);
    map.insert(REPLICA_TWO_ID.to_string(), two);
    map
}

async fn wait_for_two_replicas(state: &Arc<RelayState>, expected: &[&str], after_records: usize) {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = relay_records(state);
        let mut replicas = Vec::new();
        for record in records.iter().skip(after_records) {
            if let RecordKind::Frame {
                direction: Direction::ToRouter,
                frame_type: observed,
                bytes,
            } = &record.kind
            {
                if observed != "runtime.health" {
                    continue;
                }
                let (header, _) = decode_typed_binary_frame::<RuntimeHealthFrameHeader>(bytes)
                    .expect("decode runtime.health");
                if !replicas.contains(&header.runtime_id) {
                    replicas.push(header.runtime_id);
                }
            }
        }
        if expected
            .iter()
            .all(|replica| replicas.iter().any(|observed| observed == replica))
        {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("did not observe all replicas {expected:?}; got {replicas:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

// ---------------------------------------------------------------------------
// Fake ingress dispatcher: real `runtimeAssembly request.start` frames
// injected into the real Runtime through the test-only relay (E-dispatch
// style). The actor control plane (get-or-create / claim / invocation /
// owner control / lease / task) still flows through the real Router; the
// business request itself is delivered directly to the Runtime because the
// real Runtime at this baseline advertises no dispatch modes over the wire
// (runtime capability seam owned by the E-http lane).
// ---------------------------------------------------------------------------

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn now_nanos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}

#[derive(Debug, Clone)]
struct DispatchResult {
    body: Option<String>,
    error: Option<String>,
}

async fn dispatch_unary(
    live: &LiveProfile,
    state: &Arc<RelayState>,
    connection: u64,
    entrypoint: &Entrypoint,
    body_value: Option<&str>,
) -> DispatchResult {
    let seq = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let request_id = format!("req-{}-{seq}", now_nanos());
    let body = body_value.unwrap_or("null");
    let now_ms = now_nanos() / 1_000_000;
    let header = RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.clone(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: live.deployment_ref(),
            build_id: Some(live.deployment_ref().deployment_artifact_identity.to_string()),
            gateway_entry_identity: skiff_artifact_model::GatewayEntryIdentity::parse(
                entrypoint.gateway_entry_identity.clone(),
            )
            .expect("gateway entry identity"),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: entrypoint.method.clone(),
                path: entrypoint.path.clone(),
            },
        },
        client_session: None,
        deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms: 30_000,
            expires_at: iso_timestamp(now_ms.saturating_add(30_000)),
        }),
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{seq}"),
            span_id: format!("span-{seq}"),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method: entrypoint.method.clone(),
            url: format!("http://127.0.0.1:{}{}", live.http_port, entrypoint.path),
            path: entrypoint.path.clone(),
            query: Vec::new(),
            headers: vec![RuntimeAssemblyRequestNameValueFrameHeader {
                name: "content-type".to_string(),
                value: "application/json".to_string(),
            }],
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    };
    let bytes = encode_binary_frame(&header, body.as_bytes()).expect("encode request.start");
    state.inject(connection, Direction::ToRuntime, bytes);
    let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
    loop {
        let records = relay_records(state);
        for record in records.iter().rev() {
            let RecordKind::Frame {
                direction: Direction::ToRouter,
                frame_type: observed,
                bytes,
            } = &record.kind
            else {
                continue;
            };
            if observed == "response.end" {
                if let Ok((header, payload)) = decode_response_end_frame(bytes) {
                    if header.request_id == request_id {
                        return DispatchResult {
                            body: Some(String::from_utf8_lossy(&payload).to_string()),
                            error: None,
                        };
                    }
                }
            }
            if observed == "response.error" {
                if let Ok((header, error)) = decode_response_error_frame(bytes) {
                    if header.request_id() == request_id {
                        return DispatchResult {
                            body: None,
                            error: Some(format!("{error:?}")),
                        };
                    }
                }
            }
        }
        if tokio::time::Instant::now() > deadline {
            let recent: Vec<String> = relay_records(state)
                .iter()
                .rev()
                .filter(|record| match &record.kind {
                    RecordKind::Frame { frame_type, .. } => frame_type != "runtime.health",
                    RecordKind::PairClosed => true,
                })
                .take(40)
                .map(|record| match &record.kind {
                    RecordKind::Frame {
                        direction,
                        frame_type,
                        bytes,
                    } => format!(
                        "{:?} {} {:?}",
                        direction,
                        frame_type,
                        decode_binary_frame(bytes).ok().map(|frame| frame.header)
                    ),
                    RecordKind::PairClosed => "closed".to_string(),
                })
                .collect();
            panic!("no terminal response for {request_id}; recent frames: {recent:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn dispatch_unary_ok(
    live: &LiveProfile,
    state: &Arc<RelayState>,
    connection: u64,
    entrypoint: &Entrypoint,
) -> serde_json::Value {
    let result = dispatch_unary(live, state, connection, entrypoint, None).await;
    let body = result
        .body
        .unwrap_or_else(|| panic!("unary failed: {}", result.error.unwrap_or_default()));
    serde_json::from_str(&body).unwrap_or_else(|error| {
        panic!("unary body is not JSON: {body}; {error}");
    })
}

async fn wait_for_actor_value(
    live: &LiveProfile,
    state: &Arc<RelayState>,
    connection: u64,
    entrypoint: &Entrypoint,
    expected: serde_json::Value,
    timeout: Duration,
) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let value = dispatch_unary_ok(live, state, connection, entrypoint).await;
        if value == expected {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("actor value {expected} was not observed within timeout");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn iso_timestamp(epoch_ms: u64) -> String {
    let seconds = (epoch_ms / 1000) as i64;
    let millis = epoch_ms % 1000;
    let days = seconds.div_euclid(86_400);
    let secs_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

// ---------------------------------------------------------------------------
// Frame pairing / zero-residue assertions.
// ---------------------------------------------------------------------------

fn frame_pairs_report(records: &[RelayRecord]) -> String {
    let mut get_or_create: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut task_submit: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut invocations: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut controls: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for record in records {
        let RecordKind::Frame { bytes, .. } = &record.kind else {
            continue;
        };
        let Ok(frame) = decode_binary_frame(bytes) else {
            continue;
        };
        let Some(frame_type) = frame.header.get("type").and_then(|v| v.as_str()) else {
            continue;
        };
        let header = &frame.header;
        let text = |key: &str| {
            header
                .get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        if text("rpcId") == "direction-violation-rpc" {
            // Test-origin direction-violation probe: the Router terminates
            // the exact session by contract; the injected response frame has
            // no matching request and is excluded from pairing.
            continue;
        }
        match frame_type {
            "actor.getOrCreate.request" => {
                get_or_create.entry(text("rpcId")).or_insert((0, 0)).0 += 1;
            }
            "actor.getOrCreate.response" | "actor.getOrCreate.error" => {
                get_or_create.entry(text("rpcId")).or_insert((0, 0)).1 += 1;
            }
            "task.submit.request" => {
                task_submit.entry(text("rpcId")).or_insert((0, 0)).0 += 1;
            }
            "task.submit.response" | "task.submit.error" => {
                task_submit.entry(text("rpcId")).or_insert((0, 0)).1 += 1;
            }
            "actor.method.invoke" => {
                invocations.entry(text("invocationId")).or_insert((0, 0)).0 += 1;
            }
            "actor.owner.invoke" => {
                // The invocation id lives inside the nested `invoke` header.
                let id = header
                    .get("invoke")
                    .and_then(|value| value.get("invocationId"))
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                invocations.entry(id).or_insert((0, 0)).0 += 1;
            }
            "actor.method.return"
            | "actor.method.error"
            | "actor.method.cancel"
            | "actor.owner.failure" => {
                invocations.entry(text("invocationId")).or_insert((0, 0)).1 += 1;
            }
            "actor.owner.control" => {
                controls.entry(text("requestId")).or_insert((0, 0)).0 += 1;
            }
            "actor.owner.control.ack" => {
                controls.entry(text("requestId")).or_insert((0, 0)).1 += 1;
            }
            _ => {}
        }
    }
    let mut problems = Vec::new();
    let balanced = |problems: &mut Vec<String>, label: &str, map: &BTreeMap<String, (i64, i64)>| {
        for (key, entry) in map {
            if entry.0 < 0 || entry.1 > entry.0 {
                problems.push(format!(
                    "{label} {key}: admits={} settles={}",
                    entry.0, entry.1
                ));
            }
        }
    };
    balanced(&mut problems, "getOrCreate", &get_or_create);
    balanced(&mut problems, "task", &task_submit);
    balanced(&mut problems, "invoke", &invocations);
    balanced(&mut problems, "control", &controls);
    problems.join("; ")
}

fn frame_pairs_zero(records: &[RelayRecord]) -> bool {
    frame_pairs_report(records).is_empty()
}

// ---------------------------------------------------------------------------
// Main probe.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "driven by scripts/check-router-actor-live.mjs"]
    async fn router_live_actor_two_replica_roundtrip() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live.runtime_one_home, REPLICA_ONE_ID);
        seed_runtime_home(&live.runtime_two_home, REPLICA_TWO_ID);
        materialize_projection(&live);

        let config_path = write_router_config(&live);
        let mut router = task_router(&config_path);
        wait_for_listeners(&live, &mut router);

        let state = Arc::new(RelayState::new());
        let relay_one_task = tokio::spawn(relay_listen(
            live.relay_one_port,
            live.router_runtime_url(),
            Arc::clone(&state),
        ));
        let relay_two_task = tokio::spawn(relay_listen(
            live.relay_two_port,
            live.router_runtime_url(),
            Arc::clone(&state),
        ));

        let runtime_one_config =
            write_runtime_config(&live, live.relay_one_port, &live.runtime_one_home);
        let runtime_two_config =
            write_runtime_config(&live, live.relay_two_port, &live.runtime_two_home);
        let mut runtime_one = spawn_runtime(&live, &runtime_one_config, "runtime-one");
        let mut runtime_two = spawn_runtime(&live, &runtime_two_config, "runtime-two");

        // Both real Runtimes complete the frozen handshake through their own
        // relay into the real Router.
        let mut replicas = wait_for_two_handshakes(&live, &state).await;
        let replica_one_connection = replicas[REPLICA_ONE_ID];
        let replica_two_connection = replicas[REPLICA_TWO_ID];
        assert_ne!(replica_one_connection, replica_two_connection);
        wait_for_two_replicas(&state, &[REPLICA_ONE_ID, REPLICA_TWO_ID], 0).await;

        // 1. Ownership claim token + activation broker + owner control +
        //    invocation: marker get-or-create and increment.
        let probe = live.entrypoint("probe");
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &probe).await,
            serde_json::json!("actor-count-1")
        );
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &probe).await,
            serde_json::json!("actor-count-next")
        );

        // 2. get waits for create; concurrent claims dedup onto one create.
        let slow_get = live.entrypoint("slowGet");
        let slow_increment = live.entrypoint("slowIncrement");
        let slow_dedup = live.entrypoint("slowDedup");
        let started = tokio::time::Instant::now();
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &slow_get).await,
            serde_json::json!("slow-get-ok")
        );
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "get returned before create completed"
        );
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &slow_increment).await,
            serde_json::json!("slow-ok")
        );
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &slow_increment).await,
            serde_json::json!("slow-ok")
        );

        let dedup_started = tokio::time::Instant::now();
        let (left, right) = tokio::join!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &slow_dedup),
            dispatch_unary_ok(&live, &state, replica_two_connection, &slow_dedup)
        );
        assert_eq!(left, serde_json::json!("slow-get-ok"));
        assert_eq!(right, serde_json::json!("slow-get-ok"));
        assert!(
            dedup_started.elapsed() >= Duration::from_millis(200),
            "concurrent gets did not wait for one create"
        );

        // 3. Create failure surfaces and the retained entry keeps failing.
        let flaky = live.entrypoint("flakyGet");
        let result = dispatch_unary(&live, &state, replica_one_connection, &flaky, None).await;
        assert!(
            result.error.is_some(),
            "flaky get must fail: {:?}",
            result.body
        );
        let body = result.error.unwrap_or_default();
        assert!(
            body.contains("UnhandledServiceError")
                || body.contains("InternalError")
                || body.contains("ProviderUnavailable"),
            "flaky get failure must be a platform error: {body}"
        );
        let retried = dispatch_unary(&live, &state, replica_one_connection, &flaky, None).await;
        assert!(
            retried.error.is_some(),
            "retained flaky entry must keep failing: {:?}",
            retried.body
        );

        // 4. Synchronous self-call through the actor control plane.
        let self_call = live.entrypoint("synchronousSelfCall");
        let self_count = live.entrypoint("synchronousSelfCount");
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &self_call).await,
            serde_json::json!(105)
        );
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &self_count).await,
            serde_json::json!(105)
        );

        // 5. Function-parent actor-method task authority (fail closed): the
        //    fixture function tasks `target.record("x")` with
        //    `callerKind=request`. This harness delivers the business request
        //    directly to the Runtime (fake ingress), so the Router's
        //    dispatcher has no request parent; the task must be rejected
        //    with `task.submit.error` (ParentNotFound) and the request must
        //    fail closed. No task.submit.response may appear for that rpc.
        let task_external = live.entrypoint("spawnExternal");
        let baseline_records = relay_records(&state).len();
        let rejected =
            dispatch_unary(&live, &state, replica_one_connection, &task_external, None).await;
        assert!(
            rejected.error.is_some(),
            "request-parent task without a dispatcher parent must fail closed: {:?}",
            rejected.body
        );
        let task_error_observed =
            relay_records(&state)
                .iter()
                .skip(baseline_records)
                .any(|record| {
                    matches!(
                        &record.kind,
                        RecordKind::Frame {
                            direction: Direction::ToRuntime,
                            frame_type,
                            ..
                        } if frame_type == "task.submit.error"
                    )
                });
        let task_response_observed =
            relay_records(&state)
                .iter()
                .skip(baseline_records)
                .any(|record| {
                    matches!(
                        &record.kind,
                        RecordKind::Frame {
                            direction: Direction::ToRuntime,
                            frame_type,
                            ..
                        } if frame_type == "task.submit.response"
                    )
                });
        assert!(task_error_observed, "request-parent task must be rejected");
        assert!(
            !task_response_observed,
            "request-parent task must not be accepted without a parent"
        );

        // 6. Actor-method parent task (self message) advances the instance.
        let task_self = live.entrypoint("spawnSelfKick");
        let self_kick_count = live.entrypoint("selfKickCount");
        let self_kick_history = live.entrypoint("selfKickHistory");
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &task_self).await,
            serde_json::json!("kicked")
        );
        wait_for_actor_value(
            &live,
            &state,
            replica_one_connection,
            &self_kick_count,
            serde_json::json!(1),
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &self_kick_history).await,
            serde_json::json!("s")
        );

        // 7. Multiple task self messages queue serially.
        let task_fanout = live.entrypoint("spawnFanout");
        let fanout_count = live.entrypoint("fanoutCount");
        let fanout_history = live.entrypoint("fanoutHistory");
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &task_fanout).await,
            serde_json::json!("fanned")
        );
        wait_for_actor_value(
            &live,
            &state,
            replica_one_connection,
            &fanout_count,
            serde_json::json!(3),
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &fanout_history).await,
            serde_json::json!("abc")
        );

        // 8. Chained self-spawn: 160 steps serially without nesting.
        let chain_kick = live.entrypoint("chainKick");
        let chain_steps = live.entrypoint("chainSteps");
        let chain_history = live.entrypoint("chainHistory");
        let started = tokio::time::Instant::now();
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &chain_kick).await,
            serde_json::json!("chain-kicked")
        );
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "chain task submit waited for the chained target"
        );
        wait_for_actor_value(
            &live,
            &state,
            replica_one_connection,
            &chain_steps,
            serde_json::json!(160),
            Duration::from_secs(90),
        )
        .await;
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &chain_history).await,
            serde_json::json!("c".repeat(160))
        );

        // 9. Task mismatch fail closed: an actor-invocation task with an
        //    unknown parent is rejected and the session survives.
        let baseline_records = relay_records(&state).len();
        let captured_task = wait_for_frame_type(&state, "task.submit.request", 0).await;
        let (mut mutated, _payload) =
            decode_task_submit_request_frame(&captured_task).expect("decode captured task");
        mutated.caller_request_id = "no-such-actor-invocation-parent".to_string();
        mutated.rpc_id = "task-mismatch-probe-rpc".to_string();
        mutated.task_id = Some("task-mismatch-probe".to_string());
        let injected =
            encode_task_submit_request_frame(&mutated, &[]).expect("encode mismatch task");
        state.inject(replica_one_connection, Direction::ToRouter, injected);
        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        loop {
            let records = relay_records(&state);
            let error = records.iter().skip(baseline_records).any(|record| {
                matches!(
                    &record.kind,
                    RecordKind::Frame {
                        direction: Direction::ToRuntime,
                        frame_type,
                        bytes,
                    } if frame_type == "task.submit.error"
                        && decode_binary_frame(bytes)
                            .ok()
                            .and_then(|frame| frame.header.get("rpcId").cloned())
                            .and_then(|value| value.as_str().map(str::to_string))
                            .as_deref()
                            == Some("task-mismatch-probe-rpc")
                )
            });
            if error {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("task mismatch error frame was not observed");
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The exact runtime session survived the rejection.
        assert_eq!(
            dispatch_unary_ok(&live, &state, replica_one_connection, &probe).await,
            serde_json::json!("actor-count-next")
        );

        // 10. Replacement: inject a direction-violation task response on the
        //     relay; the Router terminates the exact session and the real
        //     Runtime reconnects through a fresh pair (connection 3).
        let direction_violation = {
            let header = skiff_runtime_transport::protocol::TaskSubmitResponseFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "task.submit.response".to_string(),
                rpc_id: "direction-violation-rpc".to_string(),
                task_ref: skiff_runtime_transport::protocol::TaskRef::new(
                    "task-direction",
                    "example.com/docs",
                )
                .expect("task ref"),
                task_id: "task-direction".to_string(),
                request_id: "request-direction".to_string(),
                status: "submitted".to_string(),
            };
            skiff_runtime_transport::protocol::encode_task_submit_response_frame(&header)
                .expect("encode direction violation")
        };
        state.inject(
            replica_one_connection,
            Direction::ToRouter,
            direction_violation,
        );
        wait_for_pair_closed(&state, replica_one_connection).await;
        let one_baseline = relay_records(&state).len();
        let reconnected_one =
            wait_for_replica_handshake(&live, &state, REPLICA_ONE_ID, one_baseline).await;
        replicas.insert(REPLICA_ONE_ID.to_string(), reconnected_one);

        // 11. Disconnect: drop the second replica's relay pair; the Router
        //     releases its owners and the Runtime reconnects; actors
        //     re-activate through a fresh claim.
        let current_two = replicas[REPLICA_TWO_ID];
        state.drop_pair(current_two);
        wait_for_pair_closed(&state, current_two).await;
        let two_baseline = relay_records(&state).len();
        let reconnected_two =
            wait_for_replica_handshake(&live, &state, REPLICA_TWO_ID, two_baseline).await;
        replicas.insert(REPLICA_TWO_ID.to_string(), reconnected_two);
        wait_for_two_replicas(&state, &[REPLICA_ONE_ID, REPLICA_TWO_ID], 0).await;
        let current_one = replicas[REPLICA_ONE_ID];
        assert_eq!(
            dispatch_unary_ok(&live, &state, current_one, &probe).await,
            // Replacement released the owner fence; the actor re-activates
            // through a fresh claim (create runs again), proving replacement
            // fail-closed re-activation.
            serde_json::json!("actor-count-1")
        );

        // 12. Zero residue: every request/response pair across the actor and
        //     task families is balanced (invocation/control/lease/timer).
        let records = relay_records(&state);
        assert!(
            frame_pairs_zero(&records),
            "actor/task frame pairs are not balanced: {}",
            frame_pairs_report(&records)
        );

        // 13. Graceful shutdown: Router barrier completes and exits 0 with
        //     listeners released; both Runtimes exit 0 on SIGINT.
        let router_pid = router.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &router_pid.to_string()])
            .status()
            .expect("deliver SIGTERM to router");
        assert!(signaled.success(), "kill -TERM router must succeed");
        let (router_status, router_stderr) =
            wait_for_exit(&mut router, Duration::from_secs(30), "router");
        assert!(
            router_status.success(),
            "graceful router shutdown must exit 0, got {router_status}; stderr: {router_stderr}"
        );
        assert_ports_closed(&live);

        for (label, runtime) in [
            ("runtime-one", &mut runtime_one),
            ("runtime-two", &mut runtime_two),
        ] {
            let pid = runtime.id();
            let signaled = Command::new("kill")
                .args(["-INT", &pid.to_string()])
                .status()
                .expect("deliver SIGINT to runtime");
            assert!(signaled.success(), "kill -INT {label} must succeed");
            let (status, _) = wait_for_exit(runtime, Duration::from_secs(30), label);
            assert!(
                status.success(),
                "graceful {label} shutdown must exit 0, got {status}"
            );
        }

        relay_one_task.abort();
        relay_two_task.abort();
        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&runtime_one_config);
        let _ = std::fs::remove_file(&runtime_two_config);
        eprintln!("router-live:actor probe: PASS");
    }
}

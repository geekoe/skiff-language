//! `durable-task-e2e-live` real-boundary vertical probe (dispatch E3a
//! completion evidence).
//!
//! Driven by `scripts/check-durable-task-e2e-live.mjs`: the harness authors a
//! real compiler artifact from the `durable-task-e2e-live` fixture (dispatch
//! statements/expressions, after/at timing, function and actor-method targets,
//! `std.task.status`/`std.task.cancel`, TaskRef stored DB field), prepares a
//! probe-owned database on the local Mongo replica set (27017, isolated from
//! the stable instance databases), leases ports in the 4100-4102 style and
//! builds the explicit Rust `skiff-router` and `runtime` binaries. This
//! ignored test then:
//!
//!   - assembles the production `RouterSupervisor` in-process with the real
//!     Mongo activation repository and Mongo TaskStore pointing at the
//!     probe-owned database (the only difference from the router binary: the
//!     database is injected so the shared local Mongo is never touched);
//!   - starts the production HTTP/control listeners on the leased ports;
//!   - spawns real `runtime` processes whose WebSocket connections are
//!     observed through a test-only relay (real Router control listener <->
//!     relay <-> real Runtime process);
//!   - drives real HTTP unary requests (X-Skiff-Service / X-Skiff-Version)
//!     through the production listener into the fixture;
//!   - asserts the full vertical chain: source -> compiler -> artifact ->
//!     runtime -> router -> Mongo TaskStore durable create -> scheduler claim
//!     -> attempt ordinary request execution -> settlement ->
//!     `std.task.status`/`std.task.cancel`;
//!   - proves immediate success, delayed after/at not-before semantics,
//!     before-start cancellation, the cancel/claim race (`alreadyStarted`),
//!     runtime kill -> lease-expiry recovery -> new attempt with repeated
//!     effect, router restart -> accepted tasks survive (Mongo persistence),
//!     actor-method tasks live / entry cold-activation / snapshot-restore,
//!     and TaskRef recovery across requests.
//!
//! The probe never touches the stable instance (4000-4007), PM2, the stable
//! Mongo databases, or the fixed 4004-4007 ports. Probe-owned databases are
//! dropped before and after the run.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use mongodb::bson::{doc, Document};
use mongodb::options::ClientOptions;
use mongodb::{Client, Collection, Database};
use serde_json::Value;
use skiff_artifact_model::{
    AssemblyActivationControl, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
};
use skiff_deployment::activation_state::ProfileActivationState;
use skiff_router::activation::{
    ActivationStateRepository, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, SystemClock,
};
use skiff_router::config::load_router_config;
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::{RouterSupervisor, SupervisorListeners};
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame, decode_typed_binary_frame,
    RuntimeCapabilitiesFrameHeader, RuntimeDispatchModeCapability, RuntimeHealthFrameHeader,
    RuntimeRegisteredFrameHeader,
};
use skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyTaskRequestStartFrameHeader;
use skiff_task_control::store::TaskStore;
use skiff_task_control::{MongoTaskStore, MongoTaskStoreOptions};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;

const REPLICA_ID: &str = "durable-task-e2e-replica";
const LIVE_TIMEOUT: Duration = Duration::from_secs(240);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CONCURRENCY: u64 = 16;
const SERVICE_ID: &str = "test.skiff/durable-task-e2e-live";
const VERSION: &str = "1.0.0";

const HANDSHAKE_SEQUENCE: [&str; 5] = [
    "router.bootstrap",
    "runtime.capabilities",
    "assembly.activation",
    "runtime.registered",
    "runtime.health",
];

struct LiveProfile {
    mongo_url: String,
    database: String,
    service_database: String,
    artifact_root: PathBuf,
    profile: String,
    assembly_identity: String,
    config_snapshot_id: String,
    generation: u64,
    http_port: u16,
    runtime_port: u16,
    relay_port: u16,
    runtime_bin: PathBuf,
    runtime_home: PathBuf,
    keyring_file: PathBuf,
    temp_dir: PathBuf,
    entrypoints: BTreeMap<String, Entrypoint>,
}

#[derive(Debug, Clone)]
struct Entrypoint {
    path: String,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-durable-task-e2e-live.mjs")
            })
        }
        let http_port = required("SKIFF_DURABLE_TASK_E2E_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_DURABLE_TASK_E2E_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let relay_port = required("SKIFF_DURABLE_TASK_E2E_RELAY_PORT")
            .parse()
            .expect("relay port");
        let generation = required("SKIFF_DURABLE_TASK_E2E_GENERATION")
            .parse()
            .expect("generation");
        let entrypoints_json = required("SKIFF_DURABLE_TASK_E2E_ENTRYPOINTS");
        let deployment_json = required("SKIFF_DURABLE_TASK_E2E_DEPLOYMENT");
        let deployment_raw: Value =
            serde_json::from_str(&deployment_json).expect("deployment JSON");
        assert_eq!(
            deployment_raw["serviceId"].as_str(),
            Some(SERVICE_ID),
            "probe deployment service id must match fixture"
        );
        assert_eq!(
            deployment_raw["contractVersion"].as_str(),
            Some(VERSION),
            "probe deployment version must match fixture"
        );
        let raw_entrypoints: Vec<Value> =
            serde_json::from_str(&entrypoints_json).expect("entrypoints JSON");
        let mut entrypoints = BTreeMap::new();
        for raw in raw_entrypoints {
            let key = raw["gatewayEntryKey"]
                .as_str()
                .expect("entrypoint gatewayEntryKey")
                .to_string();
            let selector = &raw["selector"];
            entrypoints.insert(
                key,
                Entrypoint {
                    path: selector["path"]
                        .as_str()
                        .expect("entrypoint path")
                        .to_string(),
                },
            );
        }
        Self {
            mongo_url: required("SKIFF_DURABLE_TASK_E2E_MONGO_URL"),
            database: required("SKIFF_DURABLE_TASK_E2E_DB"),
            service_database: required("SKIFF_DURABLE_TASK_E2E_SERVICE_DATABASE"),
            artifact_root: PathBuf::from(required("SKIFF_DURABLE_TASK_E2E_ARTIFACT_ROOT")),
            profile: required("SKIFF_DURABLE_TASK_E2E_PROFILE"),
            assembly_identity: required("SKIFF_DURABLE_TASK_E2E_ASSEMBLY_IDENTITY"),
            config_snapshot_id: required("SKIFF_DURABLE_TASK_E2E_CONFIG_SNAPSHOT_ID"),
            generation,
            http_port,
            runtime_port,
            relay_port,
            runtime_bin: PathBuf::from(required("SKIFF_DURABLE_TASK_E2E_RUNTIME_BIN")),
            runtime_home: PathBuf::from(required("SKIFF_DURABLE_TASK_E2E_RUNTIME_HOME")),
            keyring_file: PathBuf::from(required("SKIFF_DURABLE_TASK_E2E_KEYRING_FILE")),
            temp_dir: PathBuf::from(required("SKIFF_DURABLE_TASK_E2E_TEMP_DIR")),
            entrypoints,
        }
    }

    fn entrypoint(&self, key: &str) -> Entrypoint {
        self.entrypoints
            .get(key)
            .cloned()
            .unwrap_or_else(|| panic!("missing fixture entrypoint {key}"))
    }

    fn assembly_ref(&self) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                self.assembly_identity.clone(),
            ),
        }
    }

    fn snapshot_ref(&self) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                &self.config_snapshot_id,
            )
            .expect("config snapshot id"),
        }
    }

    fn router_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.runtime_port)
    }

    fn relay_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.relay_port)
    }

    fn control_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.runtime_port)
            .parse()
            .expect("control addr")
    }

    fn public_http_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.http_port)
            .parse()
            .expect("http addr")
    }

    /// Per-run suffix for task ids / effect tags, so repeated harness runs
    /// never collide even if a previous run left probe-owned state behind.
    fn run_suffix(&self) -> String {
        let name = self
            .temp_dir
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("run");
        name.chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    }

    fn task_id(&self, name: &str) -> String {
        format!("{name}-{}", self.run_suffix())
    }

    fn effect_tag(&self, name: &str) -> String {
        format!("{name}-{}", self.run_suffix())
    }

    async fn mongo_client(&self) -> Client {
        let options = ClientOptions::parse(&self.mongo_url)
            .await
            .expect("parse mongo options");
        Client::with_options(options).expect("mongo client")
    }
}

async fn connect_repository(live: &LiveProfile) -> Arc<dyn ActivationStateRepository> {
    let options = MongoActivationStateRepositoryOptions {
        database: live.database.clone(),
        ..Default::default()
    };
    Arc::new(
        MongoActivationStateRepository::connect(&live.mongo_url, options, Arc::new(SystemClock))
            .await
            .expect("connect probe Mongo repository"),
    )
}

async fn connect_task_store(live: &LiveProfile) -> Arc<dyn TaskStore> {
    let options = MongoTaskStoreOptions {
        database: live.database.clone(),
        ..Default::default()
    };
    Arc::new(
        MongoTaskStore::connect(&live.mongo_url, options)
            .await
            .expect("connect probe Mongo task store"),
    )
}

fn seed_runtime_home(home: &Path) {
    std::fs::create_dir_all(home).expect("create runtime home");
    std::fs::write(home.join("runtime-id"), format!("{REPLICA_ID}\n")).expect("seed runtime-id");
}

async fn seed_committed(live: &LiveProfile, repository: &Arc<dyn ActivationStateRepository>) {
    let state = ProfileActivationState::initial(
        &live.profile,
        live.generation,
        live.assembly_ref(),
        live.snapshot_ref(),
    );
    repository
        .initialize(&state)
        .await
        .expect("seed committed activation state");
}

fn write_router_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-durable-task-{}-{}.yml",
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

fn write_runtime_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join("runtime.yml");
    let contents = format!(
        "router: {}\n\
         runtime-home: {}\n\
         serviceDb:\n  encryption:\n    keyringFile: {}\n",
        live.relay_runtime_url(),
        live.runtime_home.display(),
        live.keyring_file.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
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

fn kill_process(child: &mut Child, signal: &str, label: &str) -> u32 {
    let pid = child.id();
    let signaled = Command::new("kill")
        .args([signal, &pid.to_string()])
        .status()
        .unwrap_or_else(|error| panic!("kill {label} failed: {error}"));
    assert!(signaled.success(), "kill {label} must succeed");
    pid
}

// ---------------------------------------------------------------------------
// Frame type decoding (canonical transport codec, no private copy).
// ---------------------------------------------------------------------------

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
// Test-only WS relay: real Runtime process <-> relay <-> real Router.
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
}

struct RelayState {
    records: Mutex<Vec<RelayRecord>>,
    pairs: Mutex<Vec<PairHandle>>,
    next_connection: AtomicU64,
}

impl RelayState {
    fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            pairs: Mutex::new(Vec::new()),
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
    }

    fn register_pair(&self, connection: u64) {
        self.pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(PairHandle { connection });
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
    let to_router_state = Arc::clone(&state);
    let to_runtime_state = Arc::clone(&state);
    let to_router = tokio::spawn(async move {
        pump_one(
            &mut server_reader,
            &mut client_writer,
            connection,
            Direction::ToRouter,
            to_router_state,
        )
        .await;
    });
    let to_runtime = tokio::spawn(async move {
        pump_one(
            &mut client_reader,
            &mut server_writer,
            connection,
            Direction::ToRuntime,
            to_runtime_state,
        )
        .await;
    });
    let tasks = vec![to_router, to_runtime];
    state.register_pair(connection);
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

async fn pump_one<R, W>(
    reader: &mut R,
    writer: &mut W,
    connection: u64,
    direction: Direction,
    state: Arc<RelayState>,
) where
    R: Stream<Item = Result<Message, TungsteniteError>> + Unpin,
    W: Sink<Message, Error = TungsteniteError> + Unpin,
{
    while let Some(item) = reader.next().await {
        let message = match item {
            Ok(message) => message,
            Err(_) => break,
        };
        if let Message::Binary(bytes) = &message {
            state.record_frame(connection, direction, bytes.to_vec());
        }
        if matches!(message, Message::Close(_)) {
            let _ = writer.send(Message::Close(None)).await;
            break;
        }
        if writer.send(message).await.is_err() {
            break;
        }
    }
    let _ = writer.send(Message::Close(None)).await;
}

// ---------------------------------------------------------------------------
// Relay assertions.
// ---------------------------------------------------------------------------

fn relay_records(state: &Arc<RelayState>) -> Vec<RelayRecord> {
    state
        .records
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn count_frames(
    state: &Arc<RelayState>,
    connection: u64,
    direction: Direction,
    frame_type: &str,
) -> usize {
    relay_records(state)
        .iter()
        .filter(|record| {
            if record.connection != connection {
                return false;
            }
            match &record.kind {
                RecordKind::Frame {
                    direction: record_direction,
                    frame_type: recorded_type,
                    ..
                } => recorded_type == frame_type && *record_direction == direction,
                RecordKind::PairClosed => false,
            }
        })
        .count()
}

async fn wait_for_sequence(
    state: &Arc<RelayState>,
    connection: u64,
    sequence: &[&str],
) -> Vec<RelayRecord> {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = relay_records(state);
        let frames = records
            .iter()
            .filter(|record| {
                record.connection == connection && matches!(record.kind, RecordKind::Frame { .. })
            })
            .collect::<Vec<_>>();
        if frames.len() >= sequence.len() {
            for start in 0..=frames.len() - sequence.len() {
                let window = &frames[start..start + sequence.len()];
                let types = window
                    .iter()
                    .map(|record| match &record.kind {
                        RecordKind::Frame { frame_type, .. } => frame_type.as_str(),
                        RecordKind::PairClosed => "",
                    })
                    .collect::<Vec<_>>();
                if types == sequence.to_vec() {
                    return window.iter().map(|record| (*record).clone()).collect();
                }
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "relay connection {connection} did not observe sequence {sequence:?}; records: {:?}",
                records
                    .iter()
                    .filter(|record| record.connection == connection)
                    .map(|record| match &record.kind {
                        RecordKind::Frame { frame_type, .. } => frame_type.clone(),
                        RecordKind::PairClosed => "<closed>".to_string(),
                    })
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
            panic!("relay connection {connection} never closed");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn assert_handshake(live: &LiveProfile, records: &[RelayRecord]) {
    assert_eq!(records.len(), HANDSHAKE_SEQUENCE.len());
    for (index, expected) in HANDSHAKE_SEQUENCE.iter().enumerate() {
        let RecordKind::Frame {
            direction,
            frame_type,
            bytes,
        } = &records[index].kind
        else {
            panic!("handshake record {index} is not a frame");
        };
        assert_eq!(frame_type, expected);
        match *expected {
            "router.bootstrap" => {
                assert_eq!(*direction, Direction::ToRuntime);
                let header =
                    decode_router_bootstrap_frame(bytes).expect("decode router.bootstrap frame");
                assert_eq!(header.activation.profile, live.profile);
                assert_eq!(header.activation.generation, live.generation);
                assert_eq!(
                    header.activation.assembly.assembly_identity.as_str(),
                    live.assembly_identity
                );
                assert_eq!(
                    header.activation.config_snapshot.snapshot_id.to_string(),
                    live.config_snapshot_id
                );
            }
            "runtime.capabilities" => {
                assert_eq!(*direction, Direction::ToRouter);
                let (header, _) =
                    decode_typed_binary_frame::<RuntimeCapabilitiesFrameHeader>(bytes)
                        .expect("decode capabilities frame");
                assert!(
                    header
                        .capabilities
                        .dispatch_modes
                        .contains(&RuntimeDispatchModeCapability::Unary),
                    "real Runtime must advertise unary for the admitted HTTP gateway assembly"
                );
            }
            "assembly.activation" => {
                assert_eq!(*direction, Direction::ToRouter);
                assert_register_control(live, bytes);
            }
            "runtime.registered" => {
                assert_eq!(*direction, Direction::ToRuntime);
                let (header, _) = decode_typed_binary_frame::<RuntimeRegisteredFrameHeader>(bytes)
                    .expect("decode registered ACK");
                assert_eq!(header.runtime_id, REPLICA_ID);
            }
            "runtime.health" => {
                assert_eq!(*direction, Direction::ToRouter);
                let (header, _) = decode_typed_binary_frame::<RuntimeHealthFrameHeader>(bytes)
                    .expect("decode health frame");
                assert_eq!(header.runtime_id, REPLICA_ID);
            }
            other => panic!("unexpected handshake frame {other}"),
        }
    }
}

fn assert_register_control(live: &LiveProfile, bytes: &[u8]) {
    let control =
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, bytes)
            .expect("decode register frame");
    match control {
        AssemblyActivationControl::Register {
            replica_id,
            profile,
            generation,
            assembly,
            config_snapshot,
            ..
        } => {
            assert_eq!(replica_id, REPLICA_ID);
            assert_eq!(profile, live.profile);
            assert_eq!(generation, live.generation);
            assert_eq!(assembly.assembly_identity.as_str(), live.assembly_identity);
            assert_eq!(
                config_snapshot.snapshot_id.to_string(),
                live.config_snapshot_id
            );
        }
        other => panic!("unexpected assembly activation control {other:?}"),
    }
}

async fn wait_for_replica_handshake(
    _live: &LiveProfile,
    state: &Arc<RelayState>,
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
                if observed != "assembly.activation" {
                    continue;
                }
                let Ok(control) = decode_assembly_activation_frame(
                    AssemblyActivationFrameDirection::RuntimeToRouter,
                    bytes,
                ) else {
                    continue;
                };
                if let AssemblyActivationControl::Register { replica_id, .. } = control {
                    connections.insert(replica_id, *connection);
                }
            }
        }
        if let Some(connection) = connections.get(REPLICA_ID) {
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
            panic!("replica {REPLICA_ID} handshake was not observed");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn contains_sequence(haystack: &[&str], needle: &[&str]) -> bool {
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Real HTTP client (production listener, raw TCP).
// ---------------------------------------------------------------------------

async fn raw_http(addr: SocketAddr, method: &str, path: &str, body: &[u8]) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect http listener");
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nX-Skiff-Service: {SERVICE_ID}\r\nX-Skiff-Version: {VERSION}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(head.as_bytes())
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
    let text = String::from_utf8_lossy(&response);
    let mut lines = text.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let mut headers = Vec::new();
    let mut chunked = false;
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("content-length:") {
            headers.push(line.to_string());
        }
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        }
    }
    let body = lines.collect::<Vec<_>>().join("\r\n");
    if !chunked {
        let content_length = headers
            .iter()
            .find_map(|line| {
                line.split_once(':')
                    .map(|(_, value)| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(body.len());
        return (status, body.chars().take(content_length).collect());
    }
    let mut decoded = String::new();
    let mut rest = body.as_str();
    while !rest.is_empty() {
        let Some((size_line, remainder)) = rest.split_once("\r\n") else {
            break;
        };
        let size =
            usize::from_str_radix(size_line.split(';').next().unwrap_or_default().trim(), 16)
                .unwrap_or(0);
        if size == 0 {
            break;
        }
        if remainder.len() < size + 2 {
            break;
        }
        decoded.push_str(&remainder[..size]);
        rest = &remainder[size + 2..];
    }
    (status, decoded)
}

async fn http_post(live: &LiveProfile, entry: &str, body: &Value) -> (u16, Value) {
    let entrypoint = live.entrypoint(entry);
    let (status, text) = raw_http(
        live.public_http_addr(),
        "POST",
        &entrypoint.path,
        serde_json::to_vec(body).expect("body JSON").as_slice(),
    )
    .await;
    let value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{entry} returned non-JSON body {text:?}: {error}"))
    };
    (status, value)
}

async fn submit_task(live: &LiveProfile, entry: &str, id: &str, tag: &str) {
    let (status, body) = http_post(live, entry, &serde_json::json!({ "id": id, "tag": tag })).await;
    assert_eq!(status, 200, "{entry} submit {id} failed: {body}");
    assert_eq!(
        body,
        Value::String("submitted".to_string()),
        "{entry} submit {id} returned unexpected body: {body}"
    );
}

async fn status_kind(live: &LiveProfile, id: &str) -> String {
    let (status, body) = http_post(live, "status", &serde_json::json!({ "id": id })).await;
    assert_eq!(status, 200, "status {id} failed: {body}");
    body["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("status {id} has no kind: {body}"))
        .to_string()
}

async fn wait_status_kind(live: &LiveProfile, id: &str, expected: &str, deadline: Duration) {
    let deadline = tokio::time::Instant::now() + deadline;
    loop {
        let kind = status_kind(live, id).await;
        if kind == expected {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("task {id} did not reach {expected}; last kind {kind}");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn effect_count(live: &LiveProfile, tag: &str) -> i64 {
    let (status, body) = http_post(live, "effect", &serde_json::json!({ "id": tag })).await;
    assert_eq!(status, 200, "effect {tag} failed: {body}");
    body.as_i64()
        .unwrap_or_else(|| panic!("effect {tag} is not a number: {body}"))
}

async fn actor_count(live: &LiveProfile, id: &str) -> i64 {
    let (status, body) = http_post(live, "actor-count", &serde_json::json!({ "id": id })).await;
    assert_eq!(status, 200, "actor-count {id} failed: {body}");
    body.as_i64()
        .unwrap_or_else(|| panic!("actor-count {id} is not a number: {body}"))
}

// ---------------------------------------------------------------------------
// Direct Mongo evidence helpers (probe-owned database only).
// ---------------------------------------------------------------------------

async fn task_collection(live: &LiveProfile) -> (Client, Collection<Document>) {
    let client = live.mongo_client().await;
    let collection = client
        .database(&live.database)
        .collection::<Document>("tasks");
    (client, collection)
}

/// Reads the newest task record (each harness run starts with a fresh probe
/// store, so the newest record is the scenario task under assertion).
async fn read_task_facts(live: &LiveProfile) -> (String, i64) {
    let (_client, tasks) = task_collection(live).await;
    let document = tasks
        .find_one(doc! {})
        .sort(doc! { "createdAt": -1 })
        .await
        .expect("read task records")
        .unwrap_or_else(|| panic!("newest task record missing"));
    let state = document
        .get_str("state")
        .expect("task state missing")
        .to_string();
    let generation = document
        .get("attemptGeneration")
        .and_then(mongodb::bson::Bson::as_i64)
        .expect("task attemptGeneration missing");
    (state, generation)
}

async fn drop_probe_databases(live: &LiveProfile) {
    let client = live.mongo_client().await;
    for database in [&live.database, &live.service_database] {
        let db: Database = client.database(database);
        db.drop().await.expect("drop probe database");
        let remaining = db
            .list_collection_names()
            .await
            .expect("list probe database collections");
        assert!(
            remaining.is_empty(),
            "probe database {database} must be empty after drop, got {remaining:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Supervisor helpers.
// ---------------------------------------------------------------------------

async fn assemble_supervisor(
    live: &LiveProfile,
    config_path: &Path,
) -> (
    Arc<RouterSupervisor>,
    Arc<dyn TaskStore>,
    SupervisorListeners,
) {
    let repository = connect_repository(live).await;
    let task_store = connect_task_store(live).await;
    task_store
        .ensure_indexes()
        .await
        .expect("ensure task store indexes");
    let config = load_router_config(config_path.to_str().expect("config path utf8"))
        .expect("load router config");
    let supervisor = Arc::new(
        RouterSupervisor::assemble_with_task_store(
            &config,
            &live.profile,
            repository,
            Arc::clone(&task_store),
        )
        .await
        .expect("assemble production router composition"),
    );
    let listeners = supervisor
        .start_listeners(&ListenerStartOptions {
            public_bind: Some(
                format!("127.0.0.1:{}", live.http_port)
                    .parse()
                    .expect("public bind"),
            ),
            runtime_control_bind: Some(live.control_addr()),
            drain_deadline: Duration::from_secs(30),
        })
        .await
        .expect("start production listeners");
    (supervisor, task_store, listeners)
}

async fn shutdown_supervisor(listeners: SupervisorListeners, supervisor: Arc<RouterSupervisor>) {
    listeners.shutdown().await.expect("listeners shutdown");
    supervisor.shutdown().await;
}

// ---------------------------------------------------------------------------
// Task-attempt frame evidence.
// ---------------------------------------------------------------------------

fn task_attempt_starts(state: &Arc<RelayState>, connection: u64) -> Vec<(String, String)> {
    let mut attempts = Vec::new();
    for record in relay_records(state) {
        let RecordKind::Frame {
            direction: Direction::ToRuntime,
            frame_type,
            bytes,
        } = &record.kind
        else {
            continue;
        };
        if record.connection != connection || frame_type != "request.start" {
            continue;
        }
        let Ok((header, _)) =
            decode_typed_binary_frame::<RuntimeAssemblyTaskRequestStartFrameHeader>(bytes)
        else {
            continue;
        };
        let Some(attempt) = header.task_attempt else {
            continue;
        };
        attempts.push((attempt.attempt_id, attempt.lease_id));
    }
    attempts
}

// ---------------------------------------------------------------------------
// The vertical probe.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "driven by scripts/check-durable-task-e2e-live.mjs"]
    async fn durable_task_e2e_live_vertical_chain() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live.runtime_home);
        drop_probe_databases(&live).await;

        let repository = connect_repository(&live).await;
        repository.ensure_indexes().await.expect("ensure indexes");
        seed_committed(&live, &repository).await;

        let config_path = write_router_config(&live);
        let (supervisor, _task_store, listeners) = assemble_supervisor(&live, &config_path).await;

        let state = Arc::new(RelayState::new());
        let relay_task = tokio::spawn(relay_listen(
            live.relay_port,
            live.router_runtime_url(),
            Arc::clone(&state),
        ));

        let runtime_config = write_runtime_config(&live);
        let mut runtime = spawn_runtime(&live, &runtime_config, "runtime-one");
        let connection = wait_for_replica_handshake(&live, &state, 0).await;
        let first = wait_for_sequence(&state, connection, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &first);

        // ---------------------------------------------------------------------
        // Scenario 1: immediate function task executes and settles succeeded.
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-immediate",
            &live.task_id("task-immediate"),
            &live.effect_tag("effect-immediate"),
        )
        .await;
        wait_status_kind(
            &live,
            &live.task_id("task-immediate"),
            "succeeded",
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-immediate")).await,
            1,
            "immediate task effect must execute exactly once"
        );
        let (state_kind, generation) = read_task_facts(&live).await;
        assert_eq!(state_kind, "succeeded", "immediate task record state");
        assert_eq!(generation, 1, "immediate task must have one attempt");
        assert!(
            count_frames(
                &state,
                connection,
                Direction::ToRouter,
                "task.submit.request"
            ) >= 1,
            "runtime must submit task.submit.request to the router"
        );
        assert!(
            count_frames(&state, connection, Direction::ToRuntime, "request.start") >= 1,
            "router must admit a task attempt request.start"
        );
        assert!(
            count_frames(&state, connection, Direction::ToRouter, "response.end") >= 1,
            "runtime must settle the attempt with response.end"
        );
        let immediate_attempts = task_attempt_starts(&state, connection);
        assert_eq!(
            immediate_attempts.len(),
            1,
            "exactly one attempt for immediate task"
        );
        let attempts_after_immediate = immediate_attempts.len();

        // ---------------------------------------------------------------------
        // Scenario 2a: delayed after(3000ms) task is not executed before due and
        // before-start cancel converges to canceled with no attempt.
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-after",
            &live.task_id("task-cancel"),
            &live.effect_tag("effect-cancel"),
        )
        .await;
        let kind_before = status_kind(&live, &live.task_id("task-cancel")).await;
        assert!(
            kind_before == "scheduled" || kind_before == "ready",
            "delayed task must be scheduled/ready before due, got {kind_before}"
        );
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-cancel")).await,
            0
        );
        let (status, body) = http_post(
            &live,
            "cancel",
            &serde_json::json!({ "id": &live.task_id("task-cancel") }),
        )
        .await;
        assert_eq!(status, 200, "cancel failed: {body}");
        assert_eq!(body["kind"], "canceled", "before-start cancel must succeed");
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-cancel")).await,
            0,
            "canceled task must never execute"
        );
        let (state_kind, _) = read_task_facts(&live).await;
        assert_eq!(state_kind, "canceled", "canceled task record state");
        assert_eq!(
            task_attempt_starts(&state, connection).len(),
            attempts_after_immediate,
            "canceled task must have no attempt"
        );

        // ---------------------------------------------------------------------
        // Scenario 2b: delayed after(3000ms) task executes after due.
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-after",
            &live.task_id("task-delayed"),
            &live.effect_tag("effect-delayed"),
        )
        .await;
        let kind_before = status_kind(&live, &live.task_id("task-delayed")).await;
        assert!(
            kind_before == "scheduled" || kind_before == "ready",
            "delayed task must be scheduled/ready before due, got {kind_before}"
        );
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-delayed")).await,
            0
        );
        wait_status_kind(
            &live,
            &live.task_id("task-delayed"),
            "succeeded",
            Duration::from_secs(30),
        )
        .await;
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-delayed")).await,
            1
        );
        let (state_kind, generation) = read_task_facts(&live).await;
        assert_eq!(state_kind, "succeeded");
        assert_eq!(generation, 1);

        // ---------------------------------------------------------------------
        // Scenario 3: cancel/claim race — running task cancel -> alreadyStarted,
        // state unchanged, task still converges succeeded.
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-slow",
            &live.task_id("task-race"),
            &live.effect_tag("effect-race"),
        )
        .await;
        wait_status_kind(&live, &live.task_id("task-race"), "running", CLIENT_TIMEOUT).await;
        let (status, body) = http_post(
            &live,
            "cancel",
            &serde_json::json!({ "id": &live.task_id("task-race") }),
        )
        .await;
        assert_eq!(status, 200, "running cancel failed: {body}");
        assert_eq!(
            body["kind"], "alreadyStarted",
            "cancel after claim must be alreadyStarted, got {body}"
        );
        wait_status_kind(
            &live,
            &live.task_id("task-race"),
            "succeeded",
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-race")).await,
            1
        );
        let (state_kind, _) = read_task_facts(&live).await;
        assert_eq!(state_kind, "succeeded");

        // ---------------------------------------------------------------------
        // Scenario 4: runtime kill mid-attempt -> lease-expiry recovery -> new
        // attempt with the same TaskId (at-least-once, repeated effect allowed).
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-slow",
            &live.task_id("task-recovery"),
            &live.effect_tag("effect-recovery"),
        )
        .await;
        wait_status_kind(
            &live,
            &live.task_id("task-recovery"),
            "running",
            CLIENT_TIMEOUT,
        )
        .await;
        // Wait until the first attempt's effect is durably written (the target
        // writes the effect before sleeping), then kill the runtime mid-request.
        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        while effect_count(&live, &live.effect_tag("effect-recovery")).await < 1 {
            assert!(
                tokio::time::Instant::now() < deadline,
                "first recovery attempt effect was never written"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let (pre_kill_state, pre_kill_generation) = read_task_facts(&live).await;
        assert_eq!(
            pre_kill_state, "leased",
            "recovery task must be leased before kill"
        );
        assert_eq!(pre_kill_generation, 1);
        kill_process(&mut runtime, "-9", "runtime");
        wait_for_pair_closed(&state, connection).await;
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(10), "runtime");

        // Restart the runtime immediately; the scheduler stops renewing the
        // orphaned lease and lease-expiry recovery claims the same TaskId once
        // the store-authority lease expires (~60s).
        let mut runtime = spawn_runtime(&live, &runtime_config, "runtime-two");
        let recovery_connection =
            wait_for_replica_handshake(&live, &state, relay_records(&state).len()).await;
        let handshake = wait_for_sequence(&state, recovery_connection, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &handshake);
        wait_status_kind(
            &live,
            &live.task_id("task-recovery"),
            "succeeded",
            Duration::from_secs(150),
        )
        .await;
        let (state_kind, generation) = read_task_facts(&live).await;
        assert_eq!(state_kind, "succeeded");
        assert!(
            generation >= 2,
            "lease-loss recovery must create a new attempt, got generation {generation}"
        );
        assert!(
            effect_count(&live, &live.effect_tag("effect-recovery")).await >= 2,
            "at-least-once allows a repeated effect"
        );
        let recovery_attempts = task_attempt_starts(&state, recovery_connection);
        assert!(
            recovery_attempts.len() >= 1,
            "recovered task must be admitted as a new attempt on the restarted runtime"
        );
        let _ = connection;

        // ---------------------------------------------------------------------
        // Scenario 5: router restart -> accepted delayed task survives (Mongo
        // TaskStore persistence) and executes after due.
        // ---------------------------------------------------------------------
        submit_task(
            &live,
            "submit-after",
            &live.task_id("task-router-restart"),
            &live.effect_tag("effect-router-restart"),
        )
        .await;
        let kind = status_kind(&live, &live.task_id("task-router-restart")).await;
        assert!(
            kind == "scheduled" || kind == "ready",
            "restart task must be scheduled/ready, got {kind}"
        );
        kill_process(&mut runtime, "-9", "runtime");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(10), "runtime");
        shutdown_supervisor(listeners, Arc::clone(&supervisor)).await;
        drop(supervisor);

        let (supervisor, _task_store, listeners) = assemble_supervisor(&live, &config_path).await;
        let mut runtime = spawn_runtime(&live, &runtime_config, "runtime-three");
        let restart_connection =
            wait_for_replica_handshake(&live, &state, relay_records(&state).len()).await;
        let handshake = wait_for_sequence(&state, restart_connection, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &handshake);
        wait_status_kind(
            &live,
            &live.task_id("task-router-restart"),
            "succeeded",
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(
            effect_count(&live, &live.effect_tag("effect-router-restart")).await,
            1
        );
        let (state_kind, generation) = read_task_facts(&live).await;
        assert_eq!(state_kind, "succeeded");
        assert_eq!(
            generation, 1,
            "router restart must not duplicate the accepted task"
        );

        // ---------------------------------------------------------------------
        // Scenario 6: actor-method task, branch 1 (live same implementation).
        // ---------------------------------------------------------------------
        assert_eq!(actor_count(&live, &live.effect_tag("actor-live")).await, 0);
        submit_task(
            &live,
            "submit-actor",
            &live.task_id("task-actor-live"),
            &live.effect_tag("actor-live"),
        )
        .await;
        wait_status_kind(
            &live,
            &live.task_id("task-actor-live"),
            "succeeded",
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(actor_count(&live, &live.effect_tag("actor-live")).await, 1);
        assert!(
            count_frames(
                &state,
                restart_connection,
                Direction::ToRuntime,
                "actor.owner.invoke"
            ) >= 1,
            "actor task must invoke through the actor owner lane"
        );

        // ---------------------------------------------------------------------
        // Scenario 7: actor-method task, branch 2 (registry entry exists, no live
        // incarnation -> cold activation from the entry's create input).
        // ---------------------------------------------------------------------
        assert_eq!(actor_count(&live, &live.effect_tag("actor-entry")).await, 0);
        submit_task(
            &live,
            "submit-actor-after",
            &live.task_id("task-actor-entry"),
            &live.effect_tag("actor-entry"),
        )
        .await;
        let kind = status_kind(&live, &live.task_id("task-actor-entry")).await;
        assert!(
            kind == "scheduled" || kind == "ready",
            "branch-2 actor task must be scheduled/ready, got {kind}"
        );
        kill_process(&mut runtime, "-9", "runtime");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(10), "runtime");
        let mut runtime = spawn_runtime(&live, &runtime_config, "runtime-four");
        let branch2_connection =
            wait_for_replica_handshake(&live, &state, relay_records(&state).len()).await;
        let handshake = wait_for_sequence(&state, branch2_connection, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &handshake);
        wait_status_kind(
            &live,
            &live.task_id("task-actor-entry"),
            "succeeded",
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(actor_count(&live, &live.effect_tag("actor-entry")).await, 1);
        assert!(
            count_frames(
                &state,
                branch2_connection,
                Direction::ToRuntime,
                "actor.owner.control",
            ) >= 1,
            "branch 2 must cold-activate through actor.owner.control"
        );

        // ---------------------------------------------------------------------
        // Scenario 8: actor-method task, branch 3 (registry entry lost after
        // router restart -> snapshot restore of the minimal entry).
        // ---------------------------------------------------------------------
        assert_eq!(
            actor_count(&live, &live.effect_tag("actor-snapshot")).await,
            0
        );
        submit_task(
            &live,
            "submit-actor-after",
            &live.task_id("task-actor-snapshot"),
            &live.effect_tag("actor-snapshot"),
        )
        .await;
        let kind = status_kind(&live, &live.task_id("task-actor-snapshot")).await;
        assert!(
            kind == "scheduled" || kind == "ready",
            "branch-3 actor task must be scheduled/ready, got {kind}"
        );
        kill_process(&mut runtime, "-9", "runtime");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(10), "runtime");
        shutdown_supervisor(listeners, Arc::clone(&supervisor)).await;
        drop(supervisor);

        let (supervisor, _task_store, listeners) = assemble_supervisor(&live, &config_path).await;
        let mut runtime = spawn_runtime(&live, &runtime_config, "runtime-five");
        let branch3_connection =
            wait_for_replica_handshake(&live, &state, relay_records(&state).len()).await;
        let handshake = wait_for_sequence(&state, branch3_connection, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &handshake);
        wait_status_kind(
            &live,
            &live.task_id("task-actor-snapshot"),
            "succeeded",
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(
            actor_count(&live, &live.effect_tag("actor-snapshot")).await,
            1
        );
        assert!(
            count_frames(
                &state,
                branch3_connection,
                Direction::ToRuntime,
                "actor.owner.control",
            ) >= 1,
            "branch 3 must restore and activate from the task snapshot"
        );

        // ---------------------------------------------------------------------
        // Scenario 9 (F0b): an ordinary HTTP handler submits
        // `dispatch actor.method(...)` directly (no actor execution frame). The
        // Runtime freezes the ActorActivationSnapshot from the local live
        // incarnation resolved through the request context's actor instance
        // store; the task then executes through the actor owner lane.
        // ---------------------------------------------------------------------
        assert_eq!(
            actor_count(&live, &live.effect_tag("actor-direct")).await,
            0
        );
        let submits_before = count_frames(
            &state,
            branch3_connection,
            Direction::ToRouter,
            "task.submit.request",
        );
        let invokes_before = count_frames(
            &state,
            branch3_connection,
            Direction::ToRuntime,
            "actor.owner.invoke",
        );
        submit_task(
            &live,
            "submit-actor-direct",
            &live.task_id("task-actor-direct"),
            &live.effect_tag("actor-direct"),
        )
        .await;
        wait_status_kind(
            &live,
            &live.task_id("task-actor-direct"),
            "succeeded",
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            actor_count(&live, &live.effect_tag("actor-direct")).await,
            1
        );
        assert!(
            count_frames(
                &state,
                branch3_connection,
                Direction::ToRouter,
                "task.submit.request",
            ) > submits_before,
            "HTTP-context dispatch must emit task.submit.request from the handler"
        );
        assert!(
            count_frames(
                &state,
                branch3_connection,
                Direction::ToRuntime,
                "actor.owner.invoke",
            ) > invokes_before,
            "HTTP-context actor task must execute through the actor owner lane"
        );

        // ---------------------------------------------------------------------
        // TaskRef across requests: every status/cancel above recovered the TaskRef
        // from the DB stored field; the router-restart scenario additionally
        // proves the TaskRef remains usable after the router came back.
        // ---------------------------------------------------------------------
        let (status, body) = http_post(
            &live,
            "status",
            &serde_json::json!({ "id": &live.task_id("task-immediate") }),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["kind"], "succeeded");
        let (status, body) = http_post(
            &live,
            "cancel",
            &serde_json::json!({ "id": &live.task_id("task-immediate") }),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["kind"], "alreadyTerminal");

        // ---------------------------------------------------------------------
        // Cleanup.
        // ---------------------------------------------------------------------
        kill_process(&mut runtime, "-9", "runtime");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(10), "runtime");
        shutdown_supervisor(listeners, supervisor).await;
        relay_task.abort();
        drop_probe_databases(&live).await;

        eprintln!("durable-task-e2e-live probe: PASS");
    }
}

//! `router-live:activation-full-chain` real boundary probe (E-activation
//! gate, plan §4/§7/§8).
//!
//! Driven by `scripts/check-router-activation-live.mjs`: real compiler
//! artifact (three package versions), temporary Mongo replica set, explicit
//! Rust router + runtime binaries. This ignored test then seeds the
//! committed activation state, spawns the real Router, runs the real Runtime
//! process through a test-only WS relay, and proves the §8 chain:
//! activate HTTP → durable prepare → real Runtime prepared → durable commit
//! → epoch swap → Runtime commit → same-session re-register →
//! new-generation HTTP request, while an old captured-epoch request
//! completes under its original lease; live disconnect before the decision
//! durably aborts and after the decision durable outcome reconciles; cold
//! recovery publishes committed first, rebinds the expected replica, and
//! durably aborts on candidate load failure; audit never duplicates and CAS
//! revision rejects stale replays.

use std::fs::OpenOptions;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use mongodb::bson::doc;
use skiff_artifact_model::{
    AssemblyActivationControl, AssemblyActivationRequest, AssemblyIdentity, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef, ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION,
};
use skiff_deployment::activation_state::ProfileActivationState;
use skiff_router::activation::{
    repository::PrepareInput, ActivationStateRepository, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, SystemClock,
};
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{decode_binary_frame, decode_router_bootstrap_frame};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const LIVE_TIMEOUT: Duration = Duration::from_secs(120);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENCY: u64 = 4;

#[derive(Clone)]
struct LiveProfile {
    mongo_url: String,
    database: String,
    artifact_root: PathBuf,
    profile: String,
    generation: u64,
    assembly_identity: String,
    config_snapshot_id: String,
    candidate_assembly_identity: String,
    candidate_config_snapshot_id: String,
    third_assembly_identity: String,
    third_config_snapshot_id: String,
    http_port: u16,
    runtime_port: u16,
    relay_port: u16,
    runtime_bin: PathBuf,
    runtime_home: PathBuf,
    temp_dir: PathBuf,
    replica_id: String,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-router-activation-live.mjs")
            })
        }
        let http_port = required("SKIFF_ACTIVATION_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ACTIVATION_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let relay_port = required("SKIFF_ACTIVATION_LIVE_RELAY_PORT")
            .parse()
            .expect("relay port");
        let generation = required("SKIFF_ACTIVATION_LIVE_GENERATION")
            .parse()
            .expect("generation");
        Self {
            mongo_url: required("SKIFF_ACTIVATION_LIVE_MONGO_URL"),
            database: required("SKIFF_ACTIVATION_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ACTIVATION_LIVE_ARTIFACT_ROOT")),
            profile: required("SKIFF_ACTIVATION_LIVE_PROFILE"),
            generation,
            assembly_identity: required("SKIFF_ACTIVATION_LIVE_ASSEMBLY_IDENTITY"),
            config_snapshot_id: required("SKIFF_ACTIVATION_LIVE_CONFIG_SNAPSHOT_ID"),
            candidate_assembly_identity: required(
                "SKIFF_ACTIVATION_LIVE_CANDIDATE_ASSEMBLY_IDENTITY",
            ),
            candidate_config_snapshot_id: required(
                "SKIFF_ACTIVATION_LIVE_CANDIDATE_CONFIG_SNAPSHOT_ID",
            ),
            third_assembly_identity: required("SKIFF_ACTIVATION_LIVE_THIRD_ASSEMBLY_IDENTITY"),
            third_config_snapshot_id: required("SKIFF_ACTIVATION_LIVE_THIRD_CONFIG_SNAPSHOT_ID"),
            http_port,
            runtime_port,
            relay_port,
            runtime_bin: PathBuf::from(required("SKIFF_ACTIVATION_LIVE_RUNTIME_BIN")),
            runtime_home: PathBuf::from(required("SKIFF_ACTIVATION_LIVE_RUNTIME_HOME")),
            temp_dir: PathBuf::from(required("SKIFF_ACTIVATION_LIVE_TEMP_DIR")),
            replica_id: required("SKIFF_ACTIVATION_LIVE_REPLICA_ID"),
        }
    }

    fn assembly_ref(&self, identity: &str) -> RuntimeAssemblyRef {
        RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(identity),
        }
    }

    fn snapshot_ref(&self, id: &str) -> RuntimeConfigSnapshotRef {
        RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(id)
                .expect("config snapshot id"),
        }
    }

    fn committed_refs(&self) -> (RuntimeAssemblyRef, RuntimeConfigSnapshotRef) {
        (
            self.assembly_ref(&self.assembly_identity),
            self.snapshot_ref(&self.config_snapshot_id),
        )
    }

    fn candidate_refs(&self) -> (RuntimeAssemblyRef, RuntimeConfigSnapshotRef) {
        (
            self.assembly_ref(&self.candidate_assembly_identity),
            self.snapshot_ref(&self.candidate_config_snapshot_id),
        )
    }

    fn third_refs(&self) -> (RuntimeAssemblyRef, RuntimeConfigSnapshotRef) {
        (
            self.assembly_ref(&self.third_assembly_identity),
            self.snapshot_ref(&self.third_config_snapshot_id),
        )
    }

    fn router_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.runtime_port)
    }

    fn relay_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.relay_port)
    }

    fn runtime_control_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.runtime_port)
            .parse()
            .expect("runtime control addr")
    }

    fn public_http_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.http_port)
            .parse()
            .expect("public http addr")
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
            .expect("connect temporary Mongo repository"),
    )
}

async fn seed_committed(live: &LiveProfile, repository: &Arc<dyn ActivationStateRepository>) {
    let (assembly, config_snapshot) = live.committed_refs();
    let state =
        ProfileActivationState::initial(&live.profile, live.generation, assembly, config_snapshot);
    repository
        .initialize(&state)
        .await
        .expect("seed committed activation state");
}

async fn seed_pending(
    live: &LiveProfile,
    repository: &Arc<dyn ActivationStateRepository>,
    activation_id: &str,
    expected_generation: u64,
    assembly: RuntimeAssemblyRef,
    config_snapshot: RuntimeConfigSnapshotRef,
) {
    repository
        .prepare(PrepareInput {
            profile: live.profile.clone(),
            activation_id: activation_id.to_string(),
            expected_generation,
            candidate_generation: expected_generation + 1,
            assembly,
            config_snapshot,
            participant_replica_ids: vec![live.replica_id.clone()],
        })
        .await
        .expect("seed pending activation");
}

fn write_router_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-activation-{}-{}.yml",
        live.http_port, live.runtime_port
    ));
    let contents = format!(
        "profile: {}\n\
         host: 127.0.0.1\n\
         artifactsPath: {}\n\
         releaseMode: true\n\
         requestTimeoutMs: 20000\n\
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
    let path = live.temp_dir.join("runtime-activation.yml");
    let contents = format!(
        "router: {}\n\
         runtime-home: {}\n",
        live.relay_runtime_url(),
        live.runtime_home.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
}

fn seed_runtime_home(live: &LiveProfile) {
    std::fs::create_dir_all(&live.runtime_home).expect("create runtime home");
    std::fs::write(
        live.runtime_home.join("runtime-id"),
        format!("{}\n", live.replica_id),
    )
    .expect("seed runtime-id");
}

fn task_router(live: &LiveProfile, config_path: &Path) -> Child {
    let stdout_path = live.temp_dir.join("router-activation.stdout.log");
    let stderr_path = live.temp_dir.join("router-activation.stderr.log");
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stdout_path)
        .expect("open router stdout log");
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(stderr_path)
        .expect("open router stderr log");
    Command::new(env!("CARGO_BIN_EXE_skiff-router"))
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .expect("spawn skiff-router")
}

fn spawn_runtime(live: &LiveProfile, config_path: &Path) -> Child {
    let stdout_path = live.temp_dir.join("runtime-activation.stdout.log");
    let stderr_path = live.temp_dir.join("runtime-activation.stderr.log");
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
// Test-only WS relay: real Runtime process <-> relay <-> real Router binary.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    ToRouter,
    ToRuntime,
}

#[derive(Debug, Clone)]
struct RelayRecord {
    connection: u64,
    direction: Direction,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
struct PairHandle {
    tasks: Vec<tokio::task::AbortHandle>,
}

#[derive(Debug)]
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
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(RelayRecord {
                connection,
                direction,
                bytes,
            });
    }

    fn register_pair(&self, _connection: u64, tasks: &[tokio::task::JoinHandle<()>]) {
        self.pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(PairHandle {
                tasks: tasks.iter().map(|task| task.abort_handle()).collect(),
            });
    }

    fn drop_latest_pair(&self) {
        let handle = self
            .pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .last()
            .cloned();
        if let Some(handle) = handle {
            for task in handle.tasks {
                task.abort();
            }
        }
    }

    fn records(&self) -> Vec<RelayRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
    state.register_pair(connection, &tasks);
    let mut tasks = tasks.into_iter();
    let mut to_router = tasks.next().expect("relay pump task");
    let mut to_runtime = tasks.next().expect("relay pump task");
    tokio::select! {
        _ = &mut to_router => {
            to_runtime.abort();
        }
        _ = &mut to_runtime => {
            to_router.abort();
        }
    }
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

async fn wait_for_handshake(state: &Arc<RelayState>, connection: u64) -> Vec<RelayRecord> {
    // Registration prefix; the first runtime.health may be interleaved by a
    // recovery Prepare (cold recovery rebinds immediately after the ACK), so
    // the prefix window is the four registration frames and health is
    // required anywhere after it.
    const SEQUENCE: [&str; 4] = [
        "router.bootstrap",
        "runtime.capabilities",
        "assembly.activation",
        "runtime.registered",
    ];
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = state.records();
        let frames = records
            .iter()
            .filter(|record| record.connection == connection)
            .collect::<Vec<_>>();
        if frames.len() >= SEQUENCE.len() {
            for start in 0..=frames.len() - SEQUENCE.len() {
                let window = &frames[start..start + SEQUENCE.len()];
                let types = window
                    .iter()
                    .map(|record| frame_type(&record.bytes))
                    .collect::<Vec<_>>();
                if types == SEQUENCE {
                    let has_health = frames[start + SEQUENCE.len()..]
                        .iter()
                        .any(|record| frame_type(&record.bytes) == "runtime.health");
                    if has_health {
                        return window.iter().map(|record| (*record).clone()).collect();
                    }
                }
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "relay connection {connection} did not observe handshake; types: {:?}",
                frames
                    .iter()
                    .map(|record| frame_type(&record.bytes))
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Waits until the relay observed the full activation control sequence for
/// one activation: Router Prepare → Runtime Prepared → Router Commit →
/// Runtime Register (candidate generation). Returns the decoded controls.
async fn wait_for_activation_sequence(
    state: &Arc<RelayState>,
    connection: u64,
    activation_id: &str,
    candidate_generation: u64,
) -> Vec<AssemblyActivationControl> {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let records = state.records();
        let mut controls = Vec::new();
        for record in records
            .iter()
            .filter(|record| record.connection == connection)
        {
            let direction = if record.direction == Direction::ToRouter {
                AssemblyActivationFrameDirection::RuntimeToRouter
            } else {
                AssemblyActivationFrameDirection::RouterToRuntime
            };
            let Ok(control) = decode_assembly_activation_frame(direction, &record.bytes) else {
                continue;
            };
            let matches_activation = activation_matches(&control, activation_id);
            let is_candidate_register = matches!(
                &control,
                AssemblyActivationControl::Register { generation, .. }
                    if *generation == candidate_generation
            );
            if matches_activation || is_candidate_register {
                controls.push(control);
            }
        }
        if controls.len() >= 4 {
            let kinds = controls
                .iter()
                .map(|control| control_kind(control))
                .collect::<Vec<_>>();
            if kinds == vec!["prepare", "prepared", "commit", "register"] {
                return controls;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "activation sequence for {activation_id} not observed; controls: {:?}",
                controls.iter().map(control_kind).collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn activation_matches(control: &AssemblyActivationControl, activation_id: &str) -> bool {
    match control {
        AssemblyActivationControl::Prepare {
            activation_id: id, ..
        }
        | AssemblyActivationControl::Prepared {
            activation_id: id, ..
        }
        | AssemblyActivationControl::Commit {
            activation_id: id, ..
        } => id == activation_id,
        _ => false,
    }
}

fn control_kind(control: &AssemblyActivationControl) -> &'static str {
    match control {
        AssemblyActivationControl::Prepare { .. } => "prepare",
        AssemblyActivationControl::Prepared { .. } => "prepared",
        AssemblyActivationControl::Commit { .. } => "commit",
        AssemblyActivationControl::Abort { .. } => "abort",
        AssemblyActivationControl::Reject { .. } => "reject",
        AssemblyActivationControl::Register { .. } => "register",
    }
}

fn assert_register_control(
    live: &LiveProfile,
    control: &AssemblyActivationControl,
    expected_generation: u64,
) {
    match control {
        AssemblyActivationControl::Register {
            profile,
            generation,
            assembly,
            config_snapshot,
            replica_id,
        } => {
            assert_eq!(profile, &live.profile);
            assert_eq!(*generation, expected_generation);
            assert_eq!(replica_id, &live.replica_id);
            let _ = (assembly, config_snapshot);
        }
        other => panic!("expected Register control, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Raw HTTP helpers (control + public gateway).
// ---------------------------------------------------------------------------

async fn raw_http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    service: Option<(&str, &str)>,
    body: &[u8],
) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connect http listener");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some((service_id, version)) = service {
        head.push_str(&format!(
            "X-Skiff-Service: {service_id}\r\nX-Skiff-Version: {version}\r\n"
        ));
    }
    head.push_str("\r\n");
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(head.as_bytes())
        .await
        .expect("write request head");
    if !body.is_empty() {
        stream.write_all(body).await.expect("write request body");
    }
    let mut response = Vec::new();
    use tokio::io::AsyncReadExt;
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
        let Some(crlf) = rest.find("\r\n") else {
            break;
        };
        let Ok(size) = usize::from_str_radix(rest[..crlf].trim_end_matches(';').trim(), 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let payload_start = crlf + 2;
        let payload_end = payload_start + size;
        if payload_end > rest.len() {
            break;
        }
        decoded.push_str(&rest[payload_start..payload_end]);
        rest = &rest[payload_end..];
        if let Some(stripped) = rest.strip_prefix("\r\n") {
            rest = stripped;
        }
    }
    (status, decoded)
}

fn activation_request_json(
    live: &LiveProfile,
    activation_id: &str,
    expected_generation: u64,
    assembly_identity: &str,
    config_snapshot_id: &str,
) -> String {
    let request = AssemblyActivationRequest {
        schema_version: ASSEMBLY_ACTIVATION_REQUEST_SCHEMA_VERSION.to_string(),
        profile: live.profile.clone(),
        activation_id: activation_id.to_string(),
        expected_generation,
        assembly: live.assembly_ref(assembly_identity),
        config_snapshot: live.snapshot_ref(config_snapshot_id),
    };
    serde_json::to_string(&request).expect("activation request serializes")
}

async fn post_activation(
    live: &LiveProfile,
    activation_id: &str,
    expected_generation: u64,
    assembly_identity: &str,
    config_snapshot_id: &str,
) -> (u16, serde_json::Value) {
    let body = activation_request_json(
        live,
        activation_id,
        expected_generation,
        assembly_identity,
        config_snapshot_id,
    );
    let (status, text) = raw_http(
        live.runtime_control_addr(),
        "POST",
        "/__skiff/activate-assembly",
        None,
        body.as_bytes(),
    )
    .await;
    let value = if text.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

async fn http_service_request(live: &LiveProfile, path: &str, version: &str) -> (u16, String) {
    raw_http(
        live.public_http_addr(),
        "GET",
        path,
        Some(("test.skiff/router-rust-activation-live", version)),
        b"",
    )
    .await
}

// ---------------------------------------------------------------------------
// Direct WS client helpers (committed-published-first checks).
// ---------------------------------------------------------------------------

async fn connect_direct(
    addr: SocketAddr,
) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{addr}/runtime");
    let (socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .expect("WS handshake timed out")
        .expect("WS handshake failed");
    assert_eq!(response.status(), 101);
    socket
}

async fn recv_binary(
    socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) -> Vec<u8> {
    let message = timeout(CLIENT_TIMEOUT, socket.next())
        .await
        .expect("frame timed out")
        .expect("stream ended before a binary frame")
        .expect("frame error");
    match message {
        Message::Binary(bytes) => bytes.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

async fn assert_bootstrap_tuple(live: &LiveProfile, expected_generation: u64) {
    let mut socket = connect_direct(live.runtime_control_addr()).await;
    let bytes = recv_binary(&mut socket).await;
    assert_eq!(frame_type(&bytes), "router.bootstrap");
    let header = decode_router_bootstrap_frame(&bytes).expect("decode bootstrap frame");
    assert_eq!(header.activation.profile, live.profile);
    assert_eq!(header.activation.generation, expected_generation);
    drop(socket);
}

// ---------------------------------------------------------------------------
// Durable / audit helpers.
// ---------------------------------------------------------------------------

async fn count_audit_events(live: &LiveProfile, activation_id: &str) -> u64 {
    let client = mongodb::Client::with_uri_str(&live.mongo_url)
        .await
        .expect("connect mongodb client");
    let collection = client
        .database(&live.database)
        .collection::<mongodb::bson::Document>("activation_audit");
    collection
        .count_documents(doc! { "activationId": activation_id })
        .await
        .expect("count audit events")
}

async fn wait_for_durable(
    live: &LiveProfile,
    repository: &Arc<dyn ActivationStateRepository>,
    predicate: impl Fn(&ProfileActivationState) -> bool,
) -> ProfileActivationState {
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        let state = repository
            .read(&live.profile)
            .await
            .expect("read durable state");
        if predicate(&state) {
            return state;
        }
        if tokio::time::Instant::now() > deadline {
            panic!(
                "durable state did not converge: committed gen {}, pending {:?}",
                state.committed.generation,
                state
                    .pending
                    .as_ref()
                    .map(|pending| pending.activation_id.clone())
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ---------------------------------------------------------------------------
// Scenario helpers.
// ---------------------------------------------------------------------------

/// Tasks the real Runtime against the already-listening relay and waits for
/// the new connection's handshake. Returns the child and the relay
/// connection id.
async fn spawn_runtime_await_handshake(
    live: &LiveProfile,
    relay_state: &Arc<RelayState>,
    runtime_config: &Path,
) -> (Child, u64) {
    let connection = relay_state.next_connection.load(Ordering::Relaxed) + 1;
    let runtime = spawn_runtime(live, runtime_config);
    let _ = wait_for_handshake(relay_state, connection).await;
    (runtime, connection)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires SKIFF_ACTIVATION_LIVE_* temporary profile managed by the harness"]
    async fn activation_full_chain_live() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live);
        let repository = connect_repository(&live).await;
        repository.ensure_indexes().await.expect("ensure indexes");
        seed_committed(&live, &repository).await;
        repository
            .close()
            .await
            .expect("close repository before router");

        let router_config = write_router_config(&live);
        let runtime_config = write_runtime_config(&live);
        let mut router = task_router(&live, &router_config);
        wait_for_listeners(&live, &mut router);

        // Committed epoch published before the runtime connects.
        assert_bootstrap_tuple(&live, live.generation).await;

        let relay_state = Arc::new(RelayState::new());
        let relay_task = tokio::spawn(relay_listen(
            live.relay_port,
            live.router_runtime_url(),
            Arc::clone(&relay_state),
        ));
        let (mut runtime, connection_1) =
            spawn_runtime_await_handshake(&live, &relay_state, &runtime_config).await;
        let handshake = wait_for_handshake(&relay_state, connection_1).await;
        assert_eq!(handshake.len(), 4);
        for record in &handshake {
            let direction = if record.direction == Direction::ToRouter {
                AssemblyActivationFrameDirection::RuntimeToRouter
            } else {
                AssemblyActivationFrameDirection::RouterToRuntime
            };
            match frame_type(&record.bytes).as_str() {
                "router.bootstrap" => {
                    let header = decode_router_bootstrap_frame(&record.bytes)
                        .expect("decode bootstrap frame");
                    assert_eq!(header.activation.generation, live.generation);
                }
                "assembly.activation" => {
                    let control = decode_assembly_activation_frame(direction, &record.bytes)
                        .expect("decode activation control");
                    assert_register_control(&live, &control, live.generation);
                }
                _ => {}
            }
        }

        // PHASE 1: old captured-epoch request (slow unary, version 0.1.0)
        // completes under its original lease while the live activation swaps
        // the epoch to generation 2 (version 0.1.1).
        let old_request_live = live.clone();
        let old_request = tokio::spawn(async move {
            http_service_request(&old_request_live, "/slow-unary", "0.1.0").await
        });
        let (candidate_assembly, candidate_snapshot) = live.candidate_refs();
        let (status, body) = {
            let live_task = live.clone();
            let activation_id = "activation-live-2".to_string();
            let assembly_id = candidate_assembly.assembly_identity.as_str().to_string();
            let snapshot_id = candidate_snapshot.snapshot_id.to_string();
            post_activation(
                &live_task,
                &activation_id,
                live.generation,
                &assembly_id,
                &snapshot_id,
            )
            .await
        };
        assert_eq!(status, 200, "live activation must commit: {body}");
        assert_eq!(body["ok"], true);
        assert_eq!(body["committed"]["generation"], 2);
        assert_eq!(
            body["committed"]["assembly"]["assemblyIdentity"],
            live.candidate_assembly_identity
        );
        assert_eq!(
            body["activeAssembly"]["assemblyIdentity"],
            live.candidate_assembly_identity
        );

        let controls =
            wait_for_activation_sequence(&relay_state, connection_1, "activation-live-2", 2).await;
        assert_eq!(controls.len(), 4);
        assert_register_control(&live, &controls[3], 2);

        let (old_status, old_body) = old_request.await.expect("old epoch request task");
        assert_eq!(old_status, 200, "old captured-epoch request must complete");
        assert!(old_body.contains("late"), "old request body: {old_body:?}");

        let (new_status, new_body) = http_service_request(&live, "/unary-new", "0.1.1").await;
        assert_eq!(
            new_status, 200,
            "new-generation request must succeed: {new_body:?}"
        );
        assert!(
            new_body.contains("pong-new"),
            "new request body: {new_body:?}"
        );

        let repository = connect_repository(&live).await;
        let durable = repository.read(&live.profile).await.expect("durable");
        assert_eq!(durable.committed.generation, 2);
        assert!(durable.pending.is_none());
        assert_eq!(
            count_audit_events(&live, "activation-live-2").await,
            2,
            "prepare + commit audit must not duplicate"
        );

        // CAS replay: the stale expected generation is rejected and no new
        // audit is written.
        let (replay_status, replay_body) = {
            let live_task = live.clone();
            let activation_id = "activation-live-2".to_string();
            let assembly_id = candidate_assembly.assembly_identity.as_str().to_string();
            let snapshot_id = candidate_snapshot.snapshot_id.to_string();
            post_activation(
                &live_task,
                &activation_id,
                live.generation,
                &assembly_id,
                &snapshot_id,
            )
            .await
        };
        assert_eq!(
            replay_status, 409,
            "stale CAS replay must conflict: {replay_body}"
        );
        assert_eq!(
            count_audit_events(&live, "activation-live-2").await,
            2,
            "rejected CAS replay must not append audit"
        );

        // PHASE 2: pre-decision disconnect durably aborts. Start the
        // activation, wait until durable prepare exists, freeze the Runtime
        // (cannot ACK), drop the relay connection, expect the abort outcome.
        let (third_assembly, third_snapshot) = live.third_refs();
        let activation_task = {
            let live_task = live.clone();
            let activation_id = "activation-live-3".to_string();
            let assembly_id = third_assembly.assembly_identity.as_str().to_string();
            let snapshot_id = third_snapshot.snapshot_id.to_string();
            tokio::spawn(async move {
                post_activation(&live_task, &activation_id, 2, &assembly_id, &snapshot_id).await
            })
        };
        wait_for_durable(&live, &repository, |state| {
            state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.activation_id == "activation-live-3")
        })
        .await;
        let stopped = Command::new("kill")
            .args(["-STOP", &runtime.id().to_string()])
            .status()
            .expect("SIGSTOP runtime");
        assert!(stopped.success());
        relay_state.drop_latest_pair();
        let (abort_status, abort_body) = activation_task.await.expect("activation task");
        assert!(
            abort_status >= 400 && abort_status < 600,
            "pre-decision disconnect must abort, got {abort_status}: {abort_body}"
        );
        let durable = wait_for_durable(&live, &repository, |state| {
            state.pending.is_none() && state.committed.generation == 2
        })
        .await;
        assert_eq!(durable.committed.generation, 2);
        assert_eq!(
            count_audit_events(&live, "activation-live-3").await,
            2,
            "prepare + abort audit must be exactly two"
        );
        runtime.kill().expect("kill stopped runtime");
        let (status, stderr) = wait_for_exit(&mut runtime, Duration::from_secs(15), "runtime");
        let _ = (status, stderr);

        // PHASE 3: decision-后 disconnect — durable committed outcome
        // reconciles; the Runtime is killed after commit and the durable
        // state stays authoritative.
        let (mut runtime, connection_3) =
            spawn_runtime_await_handshake(&live, &relay_state, &runtime_config).await;
        let (status, body) = {
            let live_task = live.clone();
            let activation_id = "activation-live-4".to_string();
            let assembly_id = third_assembly.assembly_identity.as_str().to_string();
            let snapshot_id = third_snapshot.snapshot_id.to_string();
            post_activation(&live_task, &activation_id, 2, &assembly_id, &snapshot_id).await
        };
        assert_eq!(status, 200, "second live activation must commit: {body}");
        wait_for_activation_sequence(&relay_state, connection_3, "activation-live-4", 3).await;
        runtime.kill().expect("kill runtime after commit");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(15), "runtime");
        let durable = wait_for_durable(&live, &repository, |state| {
            state.pending.is_none() && state.committed.generation == 3
        })
        .await;
        assert_eq!(
            durable.committed.generation, 3,
            "decision-后 disconnect must keep the durable committed outcome"
        );
        let (mut runtime, connection_4) =
            spawn_runtime_await_handshake(&live, &relay_state, &runtime_config).await;
        let _ = wait_for_handshake(&relay_state, connection_4).await;
        let (status, body) = http_service_request(&live, "/unary-third", "0.1.2").await;
        assert_eq!(status, 200, "generation-3 request must succeed: {body}");
        assert!(body.contains("pong-third"), "generation-3 body: {body:?}");

        // PHASE 4: cold recovery — committed published first, pending rebind
        // through the registration observer, commit to generation 4.
        runtime.kill().expect("kill runtime before recovery");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(15), "runtime");
        let pid = router.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("SIGTERM router");
        assert!(signaled.success());
        let (status, stderr) = wait_for_exit(&mut router, Duration::from_secs(30), "router");
        assert!(status.success(), "router exit {status}; {stderr}");
        assert_ports_closed(&live);

        seed_pending(
            &live,
            &repository,
            "recovery-4",
            3,
            third_assembly.clone(),
            third_snapshot.clone(),
        )
        .await;
        let mut router = task_router(&live, &router_config);
        wait_for_listeners(&live, &mut router);
        assert_bootstrap_tuple(&live, 3).await;
        // The phase-1 relay listener is still bound on the leased port; the
        // restarted router accepts the Runtime's new connection through it.
        let (mut runtime, recovery_connection) =
            spawn_runtime_await_handshake(&live, &relay_state, &runtime_config).await;
        let controls =
            wait_for_activation_sequence(&relay_state, recovery_connection, "recovery-4", 4).await;
        assert_eq!(controls.len(), 4);
        assert_register_control(&live, &controls[3], 4);
        let durable = wait_for_durable(&live, &repository, |state| {
            state.pending.is_none() && state.committed.generation == 4
        })
        .await;
        assert_eq!(durable.committed.generation, 4);
        assert_eq!(
            count_audit_events(&live, "recovery-4").await,
            2,
            "recovery prepare + commit audit must be exactly two"
        );
        let (status, body) = http_service_request(&live, "/unary-third", "0.1.2").await;
        assert_eq!(status, 200, "post-recovery request must succeed: {body}");

        // PHASE 5: cold recovery candidate load failure durably aborts.
        runtime
            .kill()
            .expect("kill runtime before load-failure recovery");
        let _ = wait_for_exit(&mut runtime, Duration::from_secs(15), "runtime");
        let pid = router.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("SIGTERM router");
        assert!(signaled.success());
        let (status, stderr) = wait_for_exit(&mut router, Duration::from_secs(30), "router");
        assert!(status.success(), "router exit {status}; {stderr}");
        assert_ports_closed(&live);

        let bogus_assembly = live.assembly_ref(&format!(
            "skiff-runtime-assembly-v3:sha256:{}",
            "c".repeat(64)
        ));
        seed_pending(
            &live,
            &repository,
            "recovery-load-failure",
            4,
            bogus_assembly,
            live.snapshot_ref(&live.third_config_snapshot_id),
        )
        .await;
        let mut router = task_router(&live, &router_config);
        wait_for_listeners(&live, &mut router);
        assert_bootstrap_tuple(&live, 4).await;
        let durable = wait_for_durable(&live, &repository, |state| {
            state.pending.is_none() && state.committed.generation == 4
        })
        .await;
        assert_eq!(
            durable.committed.generation, 4,
            "candidate load failure must durably abort and keep committed"
        );

        repository.close().await.expect("close repository");
        let pid = router.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("SIGTERM router");
        assert!(signaled.success());
        let (status, stderr) = wait_for_exit(&mut router, Duration::from_secs(30), "router");
        assert!(status.success(), "router exit {status}; {stderr}");
        assert_ports_closed(&live);
        relay_task.abort();

        eprintln!("router-live:activation-full-chain probe: PASS");
    }
}

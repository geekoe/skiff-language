//! `router-live:dispatch` real boundary probe (E-dispatch gate, plan §7/§8).
//!
//! Driven by `scripts/check-router-dispatch-live.mjs`: the harness compiles a
//! real package/assembly artifact with HTTP gateway entries (typedJson unary
//! echo + slow handlers), starts an isolated temporary Mongo replica set,
//! leases router/control/relay ports and builds the explicit Rust `runtime`
//! binary. This ignored test then assembles the production Router
//! composition in-process (`RouterSupervisor`/`RouterComponents` with the
//! real Mongo repository and committed activation state), starts the
//! production listeners, and spawns real `runtime` processes whose WebSocket
//! connections are observed through a test-only relay (real Router control
//! listener <-> relay <-> real Runtime process).
//!
//! The gate drives the production `HttpDispatchPort` adapter
//! (`components.http_dispatcher`) directly with constructed
//! `DispatchRequest` envelopes — the fake ingress seam (Rust has no TS
//! `/__skiff/test-dispatch` control endpoint; the equivalent seam is the
//! contract `DispatchRequest { header, payload_bytes, timeout,
//! cancel_signal }`). Everything downstream is production: epoch capture,
//! exact `RuntimeCandidateQuery`, admission permit, revalidation, enqueue to
//! the real Runtime session, terminal, and pending/permit zeroing.
//!
//! Scenarios: successful unary roundtrip through the real Runtime,
//! missing/invalid selector, wrong deployment/entry, duplicate request id,
//! timeout with `request.cancel`, runtime disconnect, selection/replacement
//! races without double-counted capacity, and full pending/permit
//! zeroing. Real HTTP sockets are deliberately out of scope (E-http gate).

use std::fs::OpenOptions;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use serde_json::Value;
use skiff_artifact_model::{
    AssemblyActivationControl, GatewayEntryIdentity, IngressProtocol, RuntimeAssemblyRef,
    RuntimeConfigSnapshotRef,
};
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::activation_state::ProfileActivationState;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::activation::{
    ActivationStateRepository, MongoActivationStateRepository,
    MongoActivationStateRepositoryOptions, SystemClock,
};
use skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH;
use skiff_router::config::load_router_config;
use skiff_router::http::dispatch::cancel_channel;
use skiff_router::http::{CancelSignal, DispatchRequest, HttpDispatchError, UnaryHttpResponse};
use skiff_router::listener::ListenerStartOptions;
use skiff_router::supervisor::{RouterComponents, RouterSupervisor};
use skiff_router::HttpDispatchPort;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame, decode_typed_binary_frame,
    RuntimeCapabilitiesFrameHeader, RuntimeDispatchModeCapability, RuntimeHealthFrameHeader,
    RuntimeRegisteredFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::runtime_assembly_request::{
    RuntimeAssemblyHttpRequestFrameHeader, RuntimeAssemblyRequestCallerFrameHeader,
    RuntimeAssemblyRequestDeadlineFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
    RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestRoutingFrameHeader,
    RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;

const REPLICA_A: &str = "skiff-runtime-live-dispatch-replica-a";
const REPLICA_B: &str = "skiff-runtime-live-dispatch-replica-b";
const LIVE_TIMEOUT: Duration = Duration::from_secs(60);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONCURRENCY: u64 = 4;
const ECHO_PATH: &str = "/echo";
const SLOW_PATH: &str = "/slow";

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
    artifact_root: PathBuf,
    profile: String,
    assembly_identity: String,
    config_snapshot_id: String,
    generation: u64,
    http_port: u16,
    control_port: u16,
    relay_port: u16,
    runtime_bin: PathBuf,
    runtime_home_a: PathBuf,
    runtime_home_b: PathBuf,
    temp_dir: PathBuf,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-router-dispatch-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_DISPATCH_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let control_port = required("SKIFF_ROUTER_DISPATCH_LIVE_CONTROL_PORT")
            .parse()
            .expect("control port");
        let relay_port = required("SKIFF_ROUTER_DISPATCH_LIVE_RELAY_PORT")
            .parse()
            .expect("relay port");
        let generation = required("SKIFF_ROUTER_DISPATCH_LIVE_GENERATION")
            .parse()
            .expect("generation");
        let temp_dir = PathBuf::from(required("SKIFF_ROUTER_DISPATCH_LIVE_TEMP_DIR"));
        Self {
            mongo_url: required("SKIFF_ROUTER_DISPATCH_LIVE_MONGO_URL"),
            database: required("SKIFF_ROUTER_DISPATCH_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_DISPATCH_LIVE_ARTIFACT_ROOT")),
            profile: required("SKIFF_ROUTER_DISPATCH_LIVE_PROFILE"),
            assembly_identity: required("SKIFF_ROUTER_DISPATCH_LIVE_ASSEMBLY_IDENTITY"),
            config_snapshot_id: required("SKIFF_ROUTER_DISPATCH_LIVE_CONFIG_SNAPSHOT_ID"),
            generation,
            http_port,
            control_port,
            relay_port,
            runtime_bin: PathBuf::from(required("SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_BIN")),
            runtime_home_a: PathBuf::from(required("SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_HOME_A")),
            runtime_home_b: PathBuf::from(required("SKIFF_ROUTER_DISPATCH_LIVE_RUNTIME_HOME_B")),
            temp_dir,
        }
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
        format!("ws://127.0.0.1:{}/runtime", self.control_port)
    }

    fn relay_runtime_url(&self) -> String {
        format!("ws://127.0.0.1:{}/runtime", self.relay_port)
    }

    fn control_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.control_port)
            .parse()
            .expect("control addr")
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

fn seed_runtime_home(home: &Path, replica_id: &str) {
    std::fs::create_dir_all(home).expect("create runtime home");
    std::fs::write(home.join("runtime-id"), format!("{replica_id}\n")).expect("seed runtime-id");
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
        "router-dispatch-{}-{}.yml",
        live.http_port, live.control_port
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
        live.control_port,
        live.mongo_url,
    );
    std::fs::write(&path, contents).expect("write router config");
    path
}

fn write_runtime_config(live: &LiveProfile, name: &str, home: &Path) -> PathBuf {
    let path = live.temp_dir.join(name);
    let contents = format!(
        "router: {}\n\
         runtime-home: {}\n",
        live.relay_runtime_url(),
        home.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
}

fn spawn_runtime(live: &LiveProfile, config_path: &Path, log_suffix: &str) -> Child {
    let stdout_path = live
        .temp_dir
        .join(format!("runtime-{log_suffix}.stdout.log"));
    let stderr_path = live
        .temp_dir
        .join(format!("runtime-{log_suffix}.stderr.log"));
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

fn assert_control_port_closed(live: &LiveProfile) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(live.control_addr()).is_err() {
            return;
        }
        if Instant::now() > deadline {
            panic!("router left the control listener bound after shutdown");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn materialize_projection(live: &LiveProfile) {
    let projection_directory = live.artifact_root.join("records/actor-routing");
    std::fs::create_dir_all(&projection_directory).expect("create projection directory");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let bytes = canonical_json_bytes(&projection).expect("canonical projection bytes");
    std::fs::write(
        live.artifact_root
            .join(ACTOR_ROUTING_PROJECTION_RECORD_PATH),
        bytes,
    )
    .expect("write projection record");
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
// Test-only WS relay: real Runtime process <-> relay <-> real Router
// control listener. Pass-through only (no frame mutation).
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
                assert!(header
                    .runtime_id
                    .starts_with("skiff-runtime-live-dispatch-replica-"));
            }
            "runtime.health" => {
                assert_eq!(*direction, Direction::ToRouter);
                let (header, _) = decode_typed_binary_frame::<RuntimeHealthFrameHeader>(bytes)
                    .expect("decode health frame");
                assert!(header
                    .runtime_id
                    .starts_with("skiff-runtime-live-dispatch-replica-"));
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
            profile,
            generation,
            assembly,
            config_snapshot,
            replica_id,
        } => {
            assert_eq!(profile, live.profile);
            assert_eq!(generation, live.generation);
            assert_eq!(assembly.assembly_identity.as_str(), live.assembly_identity);
            assert_eq!(
                config_snapshot.snapshot_id.to_string(),
                live.config_snapshot_id
            );
            assert!(replica_id.starts_with("skiff-runtime-live-dispatch-replica-"));
        }
        other => panic!("expected Register control, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Fake ingress: production `HttpDispatchPort` adapter driven by the probe.
// ---------------------------------------------------------------------------

fn dispatch_request(
    components: &RouterComponents,
    request_id: &str,
    mode: &str,
    path: &str,
    payload: &[u8],
    timeout_ms: u64,
) -> (DispatchRequest, CancelSignal) {
    let epoch = Arc::clone(&components.epoch);
    let binding = epoch
        .ingress_projection()
        .iter()
        .find(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.path == path
                && binding.selector.method.as_deref() == Some("POST")
        })
        .unwrap_or_else(|| panic!("epoch must project HTTP ingress {path}"));
    let method = binding
        .selector
        .method
        .clone()
        .expect("HTTP ingress method");
    let header = RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: mode.to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: epoch.assembly().assembly_identity.clone(),
            assembly_generation: epoch.assembly_generation(),
            deployment: binding.deployment.clone(),
            build_id: Some(binding.deployment.deployment_artifact_identity.to_string()),
            gateway_entry_identity: binding.gateway_entry_identity.clone(),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                method: method.clone(),
                path: binding.selector.path.clone(),
            },
        },
        client_session: None,
        deadline: Some(RuntimeAssemblyRequestDeadlineFrameHeader {
            timeout_ms,
            expires_at: "2030-01-01T00:00:05Z".to_string(),
        }),
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-dispatch-live".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: RuntimeAssemblyHttpRequestFrameHeader {
            method,
            url: format!("http://127.0.0.1{path}"),
            path: path.to_string(),
            query: Vec::new(),
            headers: Vec::new(),
        },
        test_effects_enabled: false,
        test_case_capability: None,
        test_case_parent_request_id: None,
    };
    let (signal, client_disconnect) = cancel_channel();
    let request = DispatchRequest {
        header,
        payload_bytes: Bytes::copy_from_slice(payload),
        timeout: Duration::from_millis(timeout_ms),
        client_disconnect,
    };
    (request, signal)
}

fn wrong_deployment_request(
    components: &RouterComponents,
    request_id: &str,
) -> (DispatchRequest, CancelSignal) {
    let epoch = Arc::clone(&components.epoch);
    let binding = epoch
        .ingress_projection()
        .iter()
        .find(|binding| {
            binding.selector.protocol == IngressProtocol::Http && binding.selector.path == ECHO_PATH
        })
        .expect("echo ingress binding");
    let mut deployment = binding.deployment.clone();
    deployment.service_id = "test.skiff/not-in-epoch".to_string();
    let (mut request, signal) =
        dispatch_request(components, request_id, "unary", ECHO_PATH, b"{}", 5000);
    request.header.routing.deployment = deployment;
    (request, signal)
}

fn wrong_entry_request(
    components: &RouterComponents,
    request_id: &str,
) -> (DispatchRequest, CancelSignal) {
    let (mut request, signal) = dispatch_request(
        components,
        request_id,
        "unary",
        ECHO_PATH,
        b"{\"value\":\"wrong-entry\"}",
        5000,
    );
    request.header.routing.gateway_entry_identity =
        GatewayEntryIdentity::parse(format!("skiff-gateway-entry-v2:sha256:{}", "b".repeat(64)))
            .expect("wrong gateway entry identity");
    (request, signal)
}

/// Fake ingress invocation: keeps the client-disconnect signal alive for the
/// whole dispatch (mirrors the HTTP phase owning `CancelOnDrop`).
async fn run_unary(
    components: &RouterComponents,
    request: DispatchRequest,
    signal: CancelSignal,
) -> Result<UnaryHttpResponse, HttpDispatchError> {
    let _signal = signal;
    components.http_dispatcher.dispatch_unary(request).await
}

async fn wait_until(mut condition: impl FnMut() -> bool, label: &str, deadline: Duration) {
    let deadline = tokio::time::Instant::now() + deadline;
    loop {
        if condition() {
            return;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("condition never became true: {label}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

fn assert_service_unavailable(result: Result<UnaryHttpResponse, HttpDispatchError>) {
    match result {
        Err(HttpDispatchError::Control {
            code,
            status: Some(503),
            ..
        }) => {
            assert_eq!(code, "ServiceUnavailable");
        }
        other => panic!("expected 503 ServiceUnavailable, got {other:?}"),
    }
}

async fn dispatch_slow(
    components: Arc<RouterComponents>,
    request_id: String,
) -> Result<UnaryHttpResponse, HttpDispatchError> {
    let payload = format!(r#"{{"value":"slow-{request_id}"}}"#);
    let (request, signal) = dispatch_request(
        &components,
        &request_id,
        "unary",
        SLOW_PATH,
        payload.as_bytes(),
        10_000,
    );
    timeout(
        Duration::from_secs(15),
        run_unary(&components, request, signal),
    )
    .await
    .expect("slow dispatch must settle within 15s")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "driven by scripts/check-router-dispatch-live.mjs"]
    async fn router_live_dispatch_full_chain() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live.runtime_home_a, REPLICA_A);
        seed_runtime_home(&live.runtime_home_b, REPLICA_B);
        materialize_projection(&live);

        let repository = connect_repository(&live).await;
        repository.ensure_indexes().await.expect("ensure indexes");
        seed_committed(&live, &repository).await;

        let config_path = write_router_config(&live);
        let config = load_router_config(config_path.to_str().expect("config path utf8"))
            .expect("router config");
        let supervisor = RouterSupervisor::assemble_with(&config, &live.profile, repository)
            .await
            .expect("assemble production router composition");
        let components = Arc::clone(supervisor.components());
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

        let state = Arc::new(RelayState::new());
        let relay_task = tokio::spawn(relay_listen(
            live.relay_port,
            live.router_runtime_url(),
            Arc::clone(&state),
        ));

        // Runtime A (real process) -> relay -> real Router composition.
        let runtime_config_a = write_runtime_config(&live, "runtime-a.yml", &live.runtime_home_a);
        let mut runtime_a = spawn_runtime(&live, &runtime_config_a, "a");
        let first = wait_for_sequence(&state, 1, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &first);

        // 1. Success: fake ingress -> admission/pending -> real Runtime ->
        //    terminal, with wire frames on the real session.
        let (success_request, success_signal) = dispatch_request(
            &components,
            "request:success",
            "unary",
            ECHO_PATH,
            b"{\"value\":\"hello\"}",
            5000,
        );
        let success = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, success_request, success_signal),
        )
        .await
        .expect("success dispatch timed out")
        .expect("success dispatch must succeed");
        assert_eq!(success.status, 200);
        let body: Value = serde_json::from_slice(&success.payload).expect("echo JSON body");
        assert_eq!(body["value"], "hello");
        wait_until(
            || {
                count_frames(&state, 1, Direction::ToRuntime, "request.start") >= 1
                    && count_frames(&state, 1, Direction::ToRouter, "response.end") >= 1
            },
            "success roundtrip wire frames",
            CLIENT_TIMEOUT,
        )
        .await;
        let health = components.dispatcher.health();
        assert_eq!(health.pending.unary, 0);
        assert_eq!(health.admission.permits_held, 0);

        // 2. Missing/invalid selector: deployment absent from the epoch and
        //    an unadvertised dispatch mode both fail closed with 503.
        let (missing_request, missing_signal) =
            wrong_deployment_request(&components, "request:wrong-deployment");
        let missing = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, missing_request, missing_signal),
        )
        .await
        .expect("missing selector dispatch timed out");
        assert_service_unavailable(missing);
        let (invalid_request, invalid_signal) = dispatch_request(
            &components,
            "request:invalid-mode",
            "serverStream",
            ECHO_PATH,
            b"{}",
            5000,
        );
        let invalid_mode = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, invalid_request, invalid_signal),
        )
        .await
        .expect("invalid mode dispatch timed out");
        assert_service_unavailable(invalid_mode);

        // 3. Wrong entry: gateway entry identity does not match the epoch's
        //    exact entry; the real Runtime rejects it fail closed.
        let (wrong_entry_request, wrong_entry_signal) =
            wrong_entry_request(&components, "request:wrong-entry");
        let wrong_entry = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, wrong_entry_request, wrong_entry_signal),
        )
        .await
        .expect("wrong entry dispatch timed out");
        assert!(
            wrong_entry.is_err(),
            "wrong gateway entry must fail closed, got {wrong_entry:?}"
        );

        // 4. Duplicate request id: second admission is rejected 409 while the
        //    first is still pending on the real Runtime.
        let duplicate_first = tokio::spawn(dispatch_slow(
            Arc::clone(&components),
            "request:duplicate".to_string(),
        ));
        wait_until(
            || components.dispatcher.pending_count() == 1,
            "duplicate first pending",
            CLIENT_TIMEOUT,
        )
        .await;
        let (duplicate_request, duplicate_signal) = dispatch_request(
            &components,
            "request:duplicate",
            "unary",
            SLOW_PATH,
            b"{\"value\":\"dup\"}",
            10_000,
        );
        let duplicate_second = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, duplicate_request, duplicate_signal),
        )
        .await
        .expect("duplicate dispatch timed out");
        match duplicate_second {
            Err(HttpDispatchError::Control {
                code,
                status: Some(409),
                ..
            }) => {
                assert_eq!(code, "DuplicateRequest");
            }
            other => panic!("expected 409 DuplicateRequest, got {other:?}"),
        }
        duplicate_first
            .await
            .expect("duplicate first task panicked")
            .expect("duplicate first must complete");

        // 5. Timeout: the adapter deadline fires, the dispatcher sends
        //    `request.cancel` to the real Runtime, and the pending/permit
        //    returns to zero.
        let cancels_before_timeout =
            count_frames(&state, 1, Direction::ToRuntime, "request.cancel");
        let (timeout_request, timeout_signal) = dispatch_request(
            &components,
            "request:timeout",
            "unary",
            SLOW_PATH,
            b"{\"value\":\"timeout\"}",
            300,
        );
        let timed_out = timeout(
            Duration::from_secs(5),
            run_unary(&components, timeout_request, timeout_signal),
        )
        .await
        .expect("timeout dispatch timed out");
        assert!(
            matches!(timed_out, Err(HttpDispatchError::Timeout { .. })),
            "expected timeout terminal, got {timed_out:?}"
        );
        wait_until(
            || {
                count_frames(&state, 1, Direction::ToRuntime, "request.cancel")
                    > cancels_before_timeout
            },
            "request.cancel frame after timeout",
            CLIENT_TIMEOUT,
        )
        .await;
        wait_until(
            || {
                let health = components.dispatcher.health();
                health.pending.unary == 0 && health.admission.permits_held == 0
            },
            "timeout pending/permit zero",
            CLIENT_TIMEOUT,
        )
        .await;

        // 6. Disconnect: a pending on the real Runtime process terminates
        //    `runtime_disconnect` with no `request.cancel` frame and releases
        //    its permit.
        let disconnect_pending = tokio::spawn(dispatch_slow(
            Arc::clone(&components),
            "request:disconnect".to_string(),
        ));
        wait_until(
            || {
                let health = components.dispatcher.health();
                health.pending.unary == 1 && health.admission.permits_held == 1
            },
            "disconnect pending admitted",
            CLIENT_TIMEOUT,
        )
        .await;
        let cancels_before_disconnect =
            count_frames(&state, 1, Direction::ToRuntime, "request.cancel");
        kill_process(&mut runtime_a, "-9", "runtime-a");
        let (_, _) = wait_for_exit(&mut runtime_a, Duration::from_secs(10), "runtime-a");
        let disconnected = timeout(CLIENT_TIMEOUT, disconnect_pending)
            .await
            .expect("disconnect terminal timed out")
            .expect("disconnect task panicked");
        assert!(
            matches!(
                disconnected,
                Err(HttpDispatchError::Cancelled {
                    source: skiff_router::http::dispatch::PendingTerminalSource::RuntimeDisconnect,
                    ..
                })
            ),
            "expected runtime_disconnect terminal, got {disconnected:?}"
        );
        wait_for_pair_closed(&state, 1).await;
        assert_eq!(
            count_frames(&state, 1, Direction::ToRuntime, "request.cancel"),
            cancels_before_disconnect,
            "runtime disconnect must not emit request.cancel"
        );
        let health = components.dispatcher.health();
        assert_eq!(health.pending.unary, 0);
        assert_eq!(health.admission.permits_held, 0);

        // 7. Selection/replacement/disconnect race with two real Runtime
        //    replicas: capacity is never double-counted and everything
        //    returns to zero.
        let runtime_config_b = write_runtime_config(&live, "runtime-b.yml", &live.runtime_home_b);
        let mut runtime_b = spawn_runtime(&live, &runtime_config_b, "b");
        let second = wait_for_sequence(&state, 2, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &second);

        let runtime_config_a2 = write_runtime_config(&live, "runtime-a2.yml", &live.runtime_home_a);
        let mut runtime_a2 = spawn_runtime(&live, &runtime_config_a2, "a2");
        let third = wait_for_sequence(&state, 3, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &third);

        let mut race_tasks = Vec::new();
        for index in 1..=8 {
            race_tasks.push(tokio::spawn(dispatch_slow(
                Arc::clone(&components),
                format!("request:race-{index}"),
            )));
        }
        wait_until(
            || components.dispatcher.health().admission.permits_held == 8,
            "race capacity full across two replicas",
            CLIENT_TIMEOUT,
        )
        .await;
        let ledger = components.dispatcher.permit_ledger();
        assert_eq!(
            ledger.per_session.len(),
            2,
            "two sessions must each hold permits"
        );
        assert!(
            ledger
                .per_session
                .values()
                .all(|in_flight| *in_flight <= MAX_CONCURRENCY as usize),
            "no session may exceed maxConcurrency: {ledger:?}"
        );

        // Both sessions are full: two more admissions are rejected
        // `queue_full` with no permit leak.
        let (full_one_request, full_one_signal) = dispatch_request(
            &components,
            "request:full-1",
            "unary",
            SLOW_PATH,
            b"{\"value\":\"full-1\"}",
            10_000,
        );
        let full_one = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, full_one_request, full_one_signal),
        )
        .await
        .expect("queue full dispatch timed out");
        assert_service_unavailable(full_one);
        let (full_two_request, full_two_signal) = dispatch_request(
            &components,
            "request:full-2",
            "unary",
            SLOW_PATH,
            b"{\"value\":\"full-2\"}",
            10_000,
        );
        let full_two = timeout(
            CLIENT_TIMEOUT,
            run_unary(&components, full_two_request, full_two_signal),
        )
        .await
        .expect("queue full dispatch timed out");
        assert_service_unavailable(full_two);
        assert!(components.dispatcher.health().admission.queue_full_rejects >= 2);
        assert_eq!(components.dispatcher.health().admission.permits_held, 8);

        // SIGKILL the A' process: exactly its four pendings terminal
        // `runtime_disconnect`, B's four stay untouched, capacity is not
        // double-counted.
        let cancels_before_race_kill =
            count_frames(&state, 1, Direction::ToRuntime, "request.cancel")
                + count_frames(&state, 2, Direction::ToRuntime, "request.cancel")
                + count_frames(&state, 3, Direction::ToRuntime, "request.cancel");
        kill_process(&mut runtime_a2, "-9", "runtime-a2");
        let (_, _) = wait_for_exit(&mut runtime_a2, Duration::from_secs(10), "runtime-a2");
        wait_for_pair_closed(&state, 3).await;
        wait_until(
            || {
                let health = components.dispatcher.health();
                health.admission.permits_held == 4
                    && health
                        .terminal
                        .by_source
                        .get(&skiff_router::dispatch::TerminalSource::RuntimeDisconnect)
                        .copied()
                        .unwrap_or(0)
                        >= 4
            },
            "disconnect race terminals released exactly four permits",
            CLIENT_TIMEOUT,
        )
        .await;
        assert_eq!(
            count_frames(&state, 1, Direction::ToRuntime, "request.cancel")
                + count_frames(&state, 2, Direction::ToRuntime, "request.cancel")
                + count_frames(&state, 3, Direction::ToRuntime, "request.cancel"),
            cancels_before_race_kill,
            "disconnect race must not emit request.cancel"
        );

        // Same-replica replacement: a fresh real Runtime process re-registers
        // with the same replica id and new requests route again.
        let runtime_config_a3 = write_runtime_config(&live, "runtime-a3.yml", &live.runtime_home_a);
        let mut runtime_a3 = spawn_runtime(&live, &runtime_config_a3, "a3");
        let fourth = wait_for_sequence(&state, 4, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &fourth);
        let replacement_one = tokio::spawn(dispatch_slow(
            Arc::clone(&components),
            "request:replacement-1".to_string(),
        ));
        let replacement_two = tokio::spawn(dispatch_slow(
            Arc::clone(&components),
            "request:replacement-2".to_string(),
        ));
        replacement_one
            .await
            .expect("replacement task panicked")
            .expect("replacement request must succeed");
        replacement_two
            .await
            .expect("replacement task panicked")
            .expect("replacement request must succeed");

        // All original B pendings and replacement pendings settle; every
        // pending/permit returns to zero and release accounting is exact.
        let mut race_ok = 0;
        let mut race_disconnect = 0;
        for task in race_tasks {
            match task.await.expect("race task panicked") {
                Ok(_) => race_ok += 1,
                Err(HttpDispatchError::Cancelled {
                    source: skiff_router::http::dispatch::PendingTerminalSource::RuntimeDisconnect,
                    ..
                }) => race_disconnect += 1,
                other => panic!("unexpected race outcome: {other:?}"),
            }
        }
        assert_eq!(race_ok, 4, "exactly four race requests complete normally");
        assert_eq!(
            race_disconnect, 4,
            "exactly four race requests terminate on runtime disconnect"
        );
        wait_until(
            || {
                let health = components.dispatcher.health();
                health.pending.unary == 0
                    && health.admission.permits_held == 0
                    && components.dispatcher.permit_ledger().per_session.is_empty()
            },
            "full race pending/permit zero",
            CLIENT_TIMEOUT,
        )
        .await;
        let health = components.dispatcher.health();
        assert_eq!(health.pending.unary, 0);
        assert_eq!(health.pending.stream, 0);
        assert_eq!(health.pending.task_attempt, 0);
        assert_eq!(health.admission.permits_held, 0);
        assert_eq!(
            health.admission.releases,
            1 + 1 + 1 + 1 + 1 + 8 + 2,
            "every accepted request releases exactly one permit"
        );
        assert!(
            health
                .terminal
                .by_source
                .get(&skiff_router::dispatch::TerminalSource::RuntimeDisconnect)
                .copied()
                .unwrap_or(0)
                >= 4
        );

        // 8. Shutdown: production listeners drain, the in-process supervisor
        //    shuts down, the control port closes, and both runtime processes
        //    exit cleanly on SIGINT.
        listeners
            .shutdown()
            .await
            .expect("production listeners shutdown");
        supervisor.shutdown().await;
        assert_control_port_closed(&live);

        let runtime_b_pid = kill_process(&mut runtime_b, "-INT", "runtime-b");
        let (runtime_b_status, _) =
            wait_for_exit(&mut runtime_b, Duration::from_secs(30), "runtime-b");
        assert!(
            runtime_b_status.success(),
            "runtime-b must exit 0 on SIGINT, got {runtime_b_status}"
        );
        let runtime_a3_pid = kill_process(&mut runtime_a3, "-INT", "runtime-a3");
        let (runtime_a3_status, _) =
            wait_for_exit(&mut runtime_a3, Duration::from_secs(30), "runtime-a3");
        assert!(
            runtime_a3_status.success(),
            "runtime-a3 must exit 0 on SIGINT, got {runtime_a3_status}"
        );
        let _ = runtime_b_pid;
        let _ = runtime_a3_pid;

        relay_task.abort();
        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&runtime_config_a);
        let _ = std::fs::remove_file(&runtime_config_b);
        let _ = std::fs::remove_file(&runtime_config_a2);
        let _ = std::fs::remove_file(&runtime_config_a3);
        eprintln!("router-live:dispatch probe: PASS");
    }
}

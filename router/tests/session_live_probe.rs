//! `router-live:session` real boundary probe (E-session gate, plan §7/§8).
//!
//! Driven by `scripts/check-router-session-live.mjs`: the harness compiles a
//! real package/assembly artifact, starts an isolated temporary Mongo replica
//! set, leases router + relay ports and builds both explicit Rust binaries.
//! This ignored test spawns the real `skiff-router` binary, and spawns the
//! real `runtime` binary whose WebSocket connection is observed through a
//! test-only relay (real Router binary <-> relay <-> real Runtime process).
//! The relay records every binary frame in both directions so the probe can
//! assert the M4 capabilities-only handshake roundtrip: `router.bootstrap` ->
//! `runtime.capabilities` -> `runtime.registered` ACK -> `runtime.health`,
//! then same-replica reconnect, replacement, pre-auth limit/timeout, ingress
//! saturation, and shutdown with zero residue. The router holds no committed
//! activation state (M4: release pointer table only) and registration is
//! capabilities-only with no `assembly.activation` frame. Unary/HTTP/WS
//! business is deliberately not claimed.

use std::fs::OpenOptions;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use skiff_canonical_json::canonical_json_bytes;
use skiff_artifact_model::current_platform_error_projection_registry_ref;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH;
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_router_bootstrap_frame, decode_typed_binary_frame,
    encode_binary_frame, RuntimeCapabilitiesFrameHeader,
    RuntimeCapabilitiesFrameHeaderMetadata, RuntimeHealthCountersFrameHeader,
    RuntimeHealthFrameHeader, RuntimeRegisteredFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::net::TcpListener;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const REPLICA_ID: &str = "skiff-runtime-live-session-replica";
const SATURATION_REPLICA_ID: &str = "skiff-runtime-live-saturation-replica";
const LIVE_TIMEOUT: Duration = Duration::from_secs(60);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONCURRENCY: u64 = 4;

const HANDSHAKE_SEQUENCE: [&str; 4] = [
    "router.bootstrap",
    "runtime.capabilities",
    "runtime.registered",
    "runtime.health",
];

struct LiveProfile {
    mongo_url: String,
    artifact_root: PathBuf,
    profile: String,
    http_port: u16,
    runtime_port: u16,
    relay_port: u16,
    runtime_bin: PathBuf,
    runtime_home: PathBuf,
    temp_dir: PathBuf,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-router-session-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_SESSION_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ROUTER_SESSION_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let relay_port = required("SKIFF_ROUTER_SESSION_LIVE_RELAY_PORT")
            .parse()
            .expect("relay port");
        Self {
            mongo_url: required("SKIFF_ROUTER_SESSION_LIVE_MONGO_URL"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_SESSION_LIVE_ARTIFACT_ROOT")),
            profile: required("SKIFF_ROUTER_SESSION_LIVE_PROFILE"),
            http_port,
            runtime_port,
            relay_port,
            runtime_bin: PathBuf::from(required("SKIFF_ROUTER_SESSION_LIVE_RUNTIME_BIN")),
            runtime_home: PathBuf::from(required("SKIFF_ROUTER_SESSION_LIVE_RUNTIME_HOME")),
            temp_dir: PathBuf::from(required("SKIFF_ROUTER_SESSION_LIVE_TEMP_DIR")),
        }
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
}

fn seed_runtime_home(live: &LiveProfile) {
    std::fs::create_dir_all(&live.runtime_home).expect("create runtime home");
    std::fs::write(
        live.runtime_home.join("runtime-id"),
        format!("{REPLICA_ID}\n"),
    )
    .expect("seed runtime-id");
}

fn write_router_config(live: &LiveProfile) -> PathBuf {
    let path = live.temp_dir.join(format!(
        "router-session-{}-{}.yml",
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
    let path = live.temp_dir.join("runtime-session.yml");
    let contents = format!(
        "router: {}\n\
         runtime-home: {}\n",
        live.relay_runtime_url(),
        live.runtime_home.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
}

fn task_router(config_path: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_skiff-router"))
        .arg(config_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn skiff-router")
}

fn spawn_runtime(live: &LiveProfile, config_path: &Path) -> Child {
    let stdout_path = live.temp_dir.join("runtime.stdout.log");
    let stderr_path = live.temp_dir.join("runtime.stderr.log");
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
// Frame construction / decoding (canonical transport codec, no private copy).
// ---------------------------------------------------------------------------

fn capabilities_frame(live: &LiveProfile, replica_id: &str) -> Vec<u8> {
    let header = RuntimeCapabilitiesFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.capabilities".to_string(),
        runtime_id: replica_id.to_string(),
        capabilities: RuntimeCapabilitiesFrameHeaderMetadata {
            platform_error_projection_registry: current_platform_error_projection_registry_ref().clone(),
            artifact_root: Some(live.artifact_root.to_string_lossy().into_owned()),
            lazy_load: true,
            loaded_build_ids: Vec::new(),
            ..Default::default()
        },
    };
    encode_binary_frame(&header, &[]).expect("encode capabilities frame")
}

fn health_frame(replica_id: &str) -> Vec<u8> {
    let header = RuntimeHealthFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "runtime.health".to_string(),
        runtime_id: replica_id.to_string(),
        observed_at: "2026-08-02T00:00:00Z".to_string(),
        counters: RuntimeHealthCountersFrameHeader {
            outbound_requests_pending: 0,
            outbound_stream_leases_active: 0,
            stream_runtime_streams_active: 0,
            flag_backed_cancel_waiters_active: 0,
            task_requests_active: 0,
        },
    };
    encode_binary_frame(&header, &[]).expect("encode health frame")
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

    fn register_pair(&self, connection: u64, tasks: &[tokio::task::JoinHandle<()>]) {
        self.pairs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(PairHandle {
                connection,
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
            // Find any contiguous run matching the sequence (the runtime keeps
            // sending periodic health frames, so the tail never stays exact).
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
                assert_eq!(header.envelope_type, "router.bootstrap");
                assert_eq!(header.activation.profile, live.profile);
                assert_eq!(header.service_db.mongo_url, live.mongo_url);
            }
            "runtime.capabilities" => {
                assert_eq!(*direction, Direction::ToRouter);
                let (header, _) =
                    decode_typed_binary_frame::<RuntimeCapabilitiesFrameHeader>(bytes)
                        .expect("decode capabilities frame");
                assert_eq!(header.runtime_id, REPLICA_ID);
                assert!(
                    header.capabilities.lazy_load,
                    "real Runtime must register capabilities-only with lazy-load enabled"
                );
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

fn count_health_frames(state: &Arc<RelayState>, connection: u64) -> usize {
    relay_records(state)
        .iter()
        .filter(|record| {
            record.connection == connection
                && matches!(
                    record.kind,
                    RecordKind::Frame {
                        ref frame_type,
                        ..
                    } if frame_type == "runtime.health"
                )
        })
        .count()
}

// ---------------------------------------------------------------------------
// Direct WS client helpers (against the real Router binary).
// ---------------------------------------------------------------------------

type PeerSocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect_direct(addr: SocketAddr) -> PeerSocket {
    let url = format!("ws://{addr}/runtime");
    let (socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .expect("WS handshake timed out")
        .expect("WS handshake failed");
    assert_eq!(response.status(), 101);
    socket
}

async fn try_connect_direct(addr: SocketAddr) -> Result<PeerSocket, ()> {
    let url = format!("ws://{addr}/runtime");
    match timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url)).await {
        Ok(Ok((socket, response))) if response.status() == 101 => Ok(socket),
        _ => Err(()),
    }
}

async fn recv_binary(socket: &mut PeerSocket) -> Vec<u8> {
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

async fn recv_bootstrap(socket: &mut PeerSocket) -> Vec<u8> {
    let bytes = recv_binary(socket).await;
    assert_eq!(frame_type(&bytes), "router.bootstrap");
    bytes
}

async fn send_binary(socket: &mut PeerSocket, bytes: Vec<u8>) {
    timeout(CLIENT_TIMEOUT, socket.send(Message::Binary(bytes.into())))
        .await
        .expect("send timed out")
        .expect("send failed");
}

async fn send_binary_tolerant(socket: &mut PeerSocket, bytes: Vec<u8>) -> bool {
    timeout(CLIENT_TIMEOUT, socket.send(Message::Binary(bytes.into())))
        .await
        .is_ok_and(|result| result.is_ok())
}

async fn expect_closed(socket: &mut PeerSocket) {
    let outcome = timeout(CLIENT_TIMEOUT, socket.next())
        .await
        .expect("close timed out");
    assert!(
        matches!(outcome, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "expected closed connection, got {outcome:?}"
    );
}

async fn expect_refused_without_handshake(socket: &mut PeerSocket) {
    let outcome = timeout(CLIENT_TIMEOUT, socket.next())
        .await
        .expect("pre-auth refusal close timed out");
    match outcome {
        None | Some(Ok(Message::Close(_))) | Some(Err(_)) => {}
        Some(Ok(Message::Binary(bytes))) => {
            panic!(
                "refused connection must not enter the handshake, got frame {:?}",
                frame_type(&bytes)
            );
        }
        Some(Ok(other)) => panic!("refused connection got unexpected frame {other:?}"),
    }
}

async fn complete_direct_handshake(live: &LiveProfile, socket: &mut PeerSocket, replica_id: &str) {
    let bootstrap = recv_bootstrap(socket).await;
    let header = decode_router_bootstrap_frame(&bootstrap).expect("decode bootstrap frame");
    assert_eq!(header.activation.profile, live.profile);
    send_binary(socket, capabilities_frame(live, replica_id)).await;
    let ack = recv_binary(socket).await;
    assert_eq!(frame_type(&ack), "runtime.registered");
    let (ack_header, _) = decode_typed_binary_frame::<RuntimeRegisteredFrameHeader>(&ack)
        .expect("decode registered ACK");
    assert_eq!(ack_header.runtime_id, replica_id);
}

// ---------------------------------------------------------------------------
// Process-level scenario probes.
// ---------------------------------------------------------------------------

async fn pre_auth_limit_test(live: &LiveProfile) {
    // maxConcurrency=4 is the pre-auth pool limit; upgraded registered
    // sessions no longer hold the pre-upgrade listener permit, so four
    // pre-auth connections fit and the fifth is refused without handshake.
    let mut holders = Vec::new();
    for _ in 0..4 {
        let mut socket = connect_direct(live.runtime_control_addr()).await;
        let _ = recv_bootstrap(&mut socket).await;
        holders.push(socket);
    }

    let mut refused = connect_direct(live.runtime_control_addr()).await;
    expect_refused_without_handshake(&mut refused).await;

    drop(holders);
    let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
    let mut accepted = None;
    while tokio::time::Instant::now() < deadline {
        if let Ok(mut socket) = try_connect_direct(live.runtime_control_addr()).await {
            if let Ok(Some(Ok(Message::Binary(frame)))) =
                timeout(Duration::from_millis(500), socket.next()).await
            {
                assert_eq!(
                    frame_type(&frame),
                    "router.bootstrap",
                    "released slot must receive a bootstrap frame"
                );
                accepted = Some(socket);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut accepted = accepted.expect("pre-auth slot must be released after close");
    let _ = accepted.close(None).await;
}

async fn bootstrap_timeout_test(live: &LiveProfile) {
    // Process-level default bootstrap deadline is 10s; the connection must be
    // closed without ever becoming registered.
    let mut socket = connect_direct(live.runtime_control_addr()).await;
    let _ = recv_bootstrap(&mut socket).await;
    let outcome = timeout(Duration::from_secs(15), socket.next())
        .await
        .expect("bootstrap deadline close did not arrive within 15s");
    assert!(
        matches!(outcome, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "bootstrap timeout must close the connection, got {outcome:?}"
    );
}

async fn ingress_saturation_test(live: &LiveProfile, state: &Arc<RelayState>) {
    let health_before = count_health_frames(state, 3);
    let mut socket = connect_direct(live.runtime_control_addr()).await;
    complete_direct_handshake(live, &mut socket, SATURATION_REPLICA_ID).await;
    // 4200 health frames exceed the process default inbound frame cap
    // (4096 cumulative frames; C-session §5.3 default corrected 2026-08-03);
    // the exact session must be aborted while other sessions keep running.
    // Sends tolerate the abort mid-loop (the socket closes once the budget
    // is exceeded).
    for _ in 0..4200 {
        if !send_binary_tolerant(&mut socket, health_frame(SATURATION_REPLICA_ID)).await {
            break;
        }
    }
    expect_closed(&mut socket).await;
    let deadline = tokio::time::Instant::now() + LIVE_TIMEOUT;
    loop {
        if count_health_frames(state, 3) > health_before {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("real Runtime health frames stopped after another session was saturated");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "driven by scripts/check-router-session-live.mjs"]
    async fn router_live_session_roundtrip() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live);
        materialize_projection(&live);

        let config_path = write_router_config(&live);
        let mut router = task_router(&config_path);
        wait_for_listeners(&live, &mut router);

        let state = Arc::new(RelayState::new());
        let relay_task = tokio::spawn(relay_listen(
            live.relay_port,
            live.router_runtime_url(),
            Arc::clone(&state),
        ));

        let runtime_config_path = write_runtime_config(&live);
        let mut runtime = spawn_runtime(&live, &runtime_config_path);

        // 1. Real Runtime -> relay -> real Router: full handshake + health.
        let first = wait_for_sequence(&state, 1, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &first);

        // 2. Same-replica reconnect after a dropped connection.
        state.drop_latest_pair();
        wait_for_pair_closed(&state, 1).await;
        let second = wait_for_sequence(&state, 2, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &second);

        // 3. Replacement: a direct same-replica client replaces the Runtime
        // session; the Runtime reconnects and replaces the direct client.
        let mut direct = connect_direct(live.runtime_control_addr()).await;
        complete_direct_handshake(&live, &mut direct, REPLICA_ID).await;
        wait_for_pair_closed(&state, 2).await;
        let third = wait_for_sequence(&state, 3, &HANDSHAKE_SEQUENCE).await;
        assert_handshake(&live, &third);
        expect_closed(&mut direct).await;

        // 4. Pre-auth limit: overflow refused, slot released after close.
        pre_auth_limit_test(&live).await;

        // 5. Pre-auth bootstrap deadline closes the connection.
        bootstrap_timeout_test(&live).await;

        // 6. Ingress saturation aborts the exact session; the real Runtime
        // session keeps sending health.
        ingress_saturation_test(&live, &state).await;

        // 7. Shutdown: Router barrier completes and exits 0 with listeners
        // released; the Runtime exits 0 on SIGINT.
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

        let runtime_pid = runtime.id();
        let runtime_signaled = Command::new("kill")
            .args(["-INT", &runtime_pid.to_string()])
            .status()
            .expect("deliver SIGINT to runtime");
        assert!(runtime_signaled.success(), "kill -INT runtime must succeed");
        let (runtime_status, _) = wait_for_exit(&mut runtime, Duration::from_secs(30), "runtime");
        assert!(
            runtime_status.success(),
            "graceful runtime shutdown must exit 0, got {runtime_status}"
        );

        relay_task.abort();
        let _ = std::fs::remove_file(&config_path);
        let _ = std::fs::remove_file(&runtime_config_path);
        eprintln!("router-live:session probe: PASS");
    }
}

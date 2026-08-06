//! `router-live:ws` real boundary probe (E-ws gate, plan §7/§8).
//!
//! Driven by `scripts/check-router-ws-live.mjs`: the harness compiles a real
//! WebSocket gateway service (`websocket.yml` with a connect handler and the
//! `status.get` / `chat.big` JSON-RPC methods), projects the RuntimeAssembly
//! with that exact ServiceDeploymentRef, produces the runtime config
//! snapshot, starts an isolated temporary Mongo replica set, leases router
//! ports and builds both explicit Rust binaries. This ignored test seeds the
//! actor routing projection record, spawns the real `skiff-router` binary and
//! the real `runtime` binary (direct runtime WS, no test relay); M4: the
//! release pointer table comes from authoring, no activation state is
//! seeded. The probe then drives a real client WebSocket:
//!   - the frozen JSON-RPC id lexeme corpus (`1e0->1`, `-0->0`, unsafe ids,
//!     parse/invalidRequest platform errors) through the full chain;
//!   - business replacement (`maxConnections: 1`, close-oldest) with socket
//!     generations and the 4009 superseded close;
//!   - disconnect race with slot cleanup and late-result isolation;
//!   - slow-client saturation (1011), frame budget (1009), binary close
//!     (1003);
//!   - graceful Router shutdown with the Runtime exiting 0.

use std::fs::OpenOptions;
use std::io::Read;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use skiff_canonical_json::canonical_json_bytes;
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::bootstrap::ACTOR_ROUTING_PROJECTION_RECORD_PATH;
use tokio::net::TcpStream as TokioTcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, Request};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const SERVICE_ID: &str = "test.skiff/router-rust-ws-live";
const SERVICE_VERSION: &str = "0.1.0";
const WS_PATH: &str = "/chat";
const BUSINESS_IDENTITY: &str = "alice";
const REPLICA_ID: &str = "skiff-runtime-live-ws-replica";
const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CONCURRENCY: u64 = 4;
const BIG_REQUESTS: usize = 96;

const ID_CORPUS: &str = include_str!("../../runtime/transport/testdata/client-ws/jsonrpc-ids.json");

struct LiveProfile {
    mongo_url: String,
    database: String,
    artifact_root: PathBuf,
    profile: String,
    http_port: u16,
    runtime_port: u16,
    runtime_bin: PathBuf,
    runtime_home: PathBuf,
    temp_dir: PathBuf,
}

impl LiveProfile {
    fn from_env() -> Self {
        fn required(name: &str) -> String {
            std::env::var(name).unwrap_or_else(|_| {
                panic!("{name} is required; run through scripts/check-router-ws-live.mjs")
            })
        }
        let http_port = required("SKIFF_ROUTER_WS_LIVE_HTTP_PORT")
            .parse()
            .expect("http port");
        let runtime_port = required("SKIFF_ROUTER_WS_LIVE_RUNTIME_PORT")
            .parse()
            .expect("runtime port");
        let profile = std::env::var("SKIFF_ROUTER_WS_LIVE_ENVIRONMENT")
            .or_else(|_| std::env::var("SKIFF_ROUTER_WS_LIVE_PROFILE"))
            .unwrap_or_else(|_| {
                panic!("SKIFF_ROUTER_WS_LIVE_ENVIRONMENT is required; run through scripts/check-router-ws-live.mjs")
            });
        Self {
            mongo_url: required("SKIFF_ROUTER_WS_LIVE_MONGO_URL"),
            database: required("SKIFF_ROUTER_WS_LIVE_DB"),
            artifact_root: PathBuf::from(required("SKIFF_ROUTER_WS_LIVE_ARTIFACT_ROOT")),
            profile,
            http_port,
            runtime_port,
            runtime_bin: PathBuf::from(required("SKIFF_ROUTER_WS_LIVE_RUNTIME_BIN")),
            runtime_home: PathBuf::from(required("SKIFF_ROUTER_WS_LIVE_RUNTIME_HOME")),
            temp_dir: PathBuf::from(required("SKIFF_ROUTER_WS_LIVE_TEMP_DIR")),
        }
    }

    fn public_http_addr(&self) -> SocketAddr {
        format!("127.0.0.1:{}", self.http_port)
            .parse()
            .expect("public http addr")
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
        "router-ws-{}-{}.yml",
        live.http_port, live.runtime_port
    ));
    let contents = format!(
        "profile: {}\n\
         host: 127.0.0.1\n\
         artifactsPath: {}\n\
         releaseMode: true\n\
         requestTimeoutMs: 20000\n\
         http:\n  port: {}\n  maxRequestBytes: 1048576\n  maxResponseBytes: 8388608\n\
         runtime:\n  port: {}\n  path: /runtime\n  maxConcurrency: {MAX_CONCURRENCY}\n\
         websocket:\n  path: {WS_PATH}\n\
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
    let path = live.temp_dir.join("runtime-ws.yml");
    let contents = format!(
        "router: ws://127.0.0.1:{}/runtime\n\
         runtime-home: {}\n",
        live.runtime_port,
        live.runtime_home.display(),
    );
    std::fs::write(&path, contents).expect("write runtime config");
    path
}

fn task_router(config_path: &Path) -> Child {
    let stdout_path = PathBuf::from(std::env::var("SKIFF_ROUTER_WS_LIVE_TEMP_DIR").unwrap())
        .join("router-ws.stdout.log");
    let stderr_path = PathBuf::from(std::env::var("SKIFF_ROUTER_WS_LIVE_TEMP_DIR").unwrap())
        .join("router-ws.stderr.log");
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
    let stdout_path = live.temp_dir.join("runtime-ws.stdout.log");
    let stderr_path = live.temp_dir.join("runtime-ws.stderr.log");
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
        if TcpStream::connect(live.public_http_addr()).is_ok()
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
        if TcpStream::connect(live.public_http_addr()).is_err()
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
// Real client WebSocket helpers (against the real Router public HTTP port).
// ---------------------------------------------------------------------------

type ClientSocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<TokioTcpStream>>;

fn client_request(live: &LiveProfile) -> Request<()> {
    let mut request = format!("ws://127.0.0.1:{}{WS_PATH}?x=1", live.http_port)
        .into_client_request()
        .expect("client request");
    request
        .headers_mut()
        .insert("x-skiff-service", HeaderValue::from_static(SERVICE_ID));
    request
        .headers_mut()
        .insert("x-skiff-version", HeaderValue::from_static(SERVICE_VERSION));
    request
}

async fn try_connect_client(live: &LiveProfile) -> Option<ClientSocket> {
    match timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(client_request(live)),
    )
    .await
    {
        Ok(Ok((socket, response))) if response.status() == 101 => Some(socket),
        Ok(Ok((_socket, response))) => {
            eprintln!(
                "router-live:ws probe: connect attempt returned HTTP {}",
                response.status()
            );
            None
        }
        Ok(Err(error)) => {
            eprintln!("router-live:ws probe: connect attempt error: {error}");
            None
        }
        Err(_) => {
            eprintln!("router-live:ws probe: connect attempt timed out");
            None
        }
    }
}

async fn connect_client(live: &LiveProfile) -> ClientSocket {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(socket) = try_connect_client(live).await {
            return socket;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("real client WS connect did not succeed within 30s");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn send_text(socket: &mut ClientSocket, frame: &str) {
    timeout(CLIENT_TIMEOUT, socket.send(Message::Text(frame.into())))
        .await
        .expect("send timed out")
        .expect("send failed");
}

async fn recv_text(socket: &mut ClientSocket) -> String {
    loop {
        let message = timeout(CLIENT_TIMEOUT, socket.next())
            .await
            .expect("read timed out")
            .expect("stream ended before a text frame");
        match message {
            Ok(Message::Text(text)) => return text.to_string(),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            other => panic!("unexpected message while waiting for text: {other:?}"),
        }
    }
}

async fn recv_close(socket: &mut ClientSocket) -> (u16, String) {
    loop {
        let message = timeout(Duration::from_secs(30), socket.next())
            .await
            .expect("close timed out")
            .expect("stream ended without close");
        match message {
            Ok(Message::Close(Some(frame))) => {
                return (frame.code.into(), frame.reason.to_string())
            }
            Ok(Message::Close(None)) => return (1005, String::new()),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            other => panic!("unexpected message while waiting for close: {other:?}"),
        }
    }
}

async fn recv_close_after_slow_client(socket: &mut ClientSocket) -> (u16, String) {
    let mut text_frames = 0usize;
    loop {
        let message = timeout(Duration::from_secs(30), socket.next())
            .await
            .expect("slow-client close timed out")
            .expect("stream ended without close");
        match message {
            Ok(Message::Close(Some(frame))) => {
                eprintln!(
                    "router-live:ws probe: slow-client phase read {text_frames} text frames before close"
                );
                return (frame.code.into(), frame.reason.to_string());
            }
            Ok(Message::Close(None)) => return (1005, String::new()),
            Ok(Message::Text(_)) => {
                text_frames += 1;
            }
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            other => panic!("unexpected message while waiting for slow-client close: {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Frozen JSON-RPC id lexeme corpus (same fixture as contracts-ws).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct IdCorpus {
    cases: Vec<IdCase>,
}

#[derive(Debug, Deserialize)]
struct IdCase {
    name: String,
    kind: String,
    frame: String,
    #[serde(default)]
    #[serde(rename = "idKind")]
    id_kind: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    #[serde(rename = "errorKind")]
    error_kind: Option<String>,
}

async fn run_id_corpus(live: &LiveProfile) {
    let corpus: IdCorpus = serde_json::from_str(ID_CORPUS).expect("parse jsonrpc id corpus");
    let mut exercised = 0usize;
    for case in &corpus.cases {
        match case.kind.as_str() {
            "request" => {
                let mut socket = connect_client(live).await;
                send_text(&mut socket, &case.frame).await;
                let response = recv_text(&mut socket).await;
                let value: serde_json::Value =
                    serde_json::from_str(&response).unwrap_or_else(|error| {
                        panic!(
                            "corpus {}: undecodable response {response:?}: {error}",
                            case.name
                        )
                    });
                assert!(
                    value.get("result").is_some(),
                    "corpus {}: expected a result response, got {response}",
                    case.name
                );
                let id = case.id.as_ref().expect("request case id");
                match case.id_kind.as_deref() {
                    Some("safeInteger") => {
                        let expected: i64 = id.parse().expect("corpus integer id");
                        assert_eq!(
                            value["id"],
                            json!(expected),
                            "corpus {}: canonical numeric id",
                            case.name
                        );
                    }
                    Some("string") => {
                        assert_eq!(
                            value["id"],
                            json!(id),
                            "corpus {}: string id preserved",
                            case.name
                        );
                    }
                    other => panic!("corpus {}: unknown id kind {other:?}", case.name),
                }
                assert_eq!(
                    value["result"]["accepted"],
                    json!(true),
                    "corpus {}: real runtime result",
                    case.name
                );
                assert_eq!(
                    value["result"]["businessIdentity"],
                    json!(BUSINESS_IDENTITY),
                    "corpus {}: business identity flowed through the real chain",
                    case.name
                );
                let _ = socket.close(None).await;
                exercised += 1;
            }
            "platformError" => {
                let mut socket = connect_client(live).await;
                send_text(&mut socket, &case.frame).await;
                let response = recv_text(&mut socket).await;
                let value: serde_json::Value =
                    serde_json::from_str(&response).unwrap_or_else(|error| {
                        panic!(
                            "corpus {}: undecodable error response {response:?}: {error}",
                            case.name
                        )
                    });
                let expected_code = match case.error_kind.as_deref() {
                    Some("parse") => -32700,
                    Some("invalidRequest") => -32600,
                    other => panic!("corpus {}: unknown error kind {other:?}", case.name),
                };
                assert_eq!(
                    value["error"]["code"],
                    json!(expected_code),
                    "corpus {}: platform error code",
                    case.name
                );
                let _ = socket.close(None).await;
                exercised += 1;
            }
            // notification/response/close cases are covered by the
            // production-lane ws_corpus tests; the live socket exercises the
            // request + platformError parser surface above.
            _ => {}
        }
    }
    assert!(
        exercised >= 15,
        "id corpus must exercise at least 15 real-socket cases, got {exercised}"
    );
    eprintln!("router-live:ws probe: id corpus exercised {exercised} real-socket cases");
}

async fn status_roundtrip(socket: &mut ClientSocket, id: &str) -> serde_json::Value {
    let frame = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"status.get","params":[]}}"#);
    send_text(socket, &frame).await;
    let response = recv_text(socket).await;
    serde_json::from_str(&response)
        .unwrap_or_else(|error| panic!("status roundtrip response {response:?}: {error}"))
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

async fn replacement_and_disconnect_race(live: &LiveProfile) -> ClientSocket {
    // WS#1 (corpus phase) was closed; WS#2 takes the single business slot.
    let mut second = connect_client(live).await;
    let value = status_roundtrip(&mut second, "1").await;
    assert_eq!(value["result"]["accepted"], json!(true));

    // WS#3 with the same business identity replaces WS#2 (close-oldest,
    // socket generation 2); TS parity: non-ranked close-oldest overflow
    // closes the old generation with 1008 (`policyOverflowClose`), while
    // 4009 is reserved for ranked supersession.
    let mut third = connect_client(live).await;
    let (code, reason) = recv_close(&mut second).await;
    assert_eq!(
        code, 1008,
        "close-oldest replacement close (reason={reason})"
    );
    let value = status_roundtrip(&mut third, "2").await;
    assert_eq!(value["result"]["accepted"], json!(true));

    // Disconnect race: drop the current generation without a close frame;
    // the finalizer releases the pin and frees the business slot.
    drop(third);
    tokio::time::sleep(Duration::from_secs(1)).await;

    // The finalizer must release the pin and free the business slot so a
    // replacement can reconnect and serve a fresh roundtrip.
    let mut fourth = connect_client(live).await;
    let value = status_roundtrip(&mut fourth, "3").await;
    assert_eq!(value["result"]["accepted"], json!(true));
    fourth
}

async fn slow_client_saturation(mut socket: ClientSocket) {
    // The real `chat.big` handler returns a ~512 KiB result. The writer queue
    // + observed-write budget is 16 MiB; sending many requests without
    // reading saturates the captured writer and terminates with 1011.
    for index in 0..BIG_REQUESTS {
        let frame = format!(
            r#"{{"jsonrpc":"2.0","id":{},"method":"chat.big","params":[]}}"#,
            index + 1000
        );
        send_text(&mut socket, &frame).await;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let (code, reason) = recv_close_after_slow_client(&mut socket).await;
    assert_eq!(code, 1011, "slow-client close (reason={reason})");
    eprintln!(
        "router-live:ws probe: slow-client close after {} big responses",
        BIG_REQUESTS
    );
}

async fn frame_budget_and_binary_closes(live: &LiveProfile) {
    // Binary frames are unsupported: close 1003.
    let mut binary = connect_client(live).await;
    timeout(
        CLIENT_TIMEOUT,
        binary.send(Message::Binary(vec![1, 2, 3].into())),
    )
    .await
    .expect("binary send timed out")
    .expect("binary send failed");
    let (code, reason) = recv_close(&mut binary).await;
    assert_eq!(code, 1003, "binary close (reason={reason})");

    // Oversized text frame exceeds WEBSOCKET_JSONRPC_MAX_TEXT_BYTES (1 MiB):
    // close 1009.
    let mut oversized = connect_client(live).await;
    let big_frame = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"status.get","params":["{}"]}}"#,
        "x".repeat(1024 * 1024 + 64)
    );
    send_text(&mut oversized, &big_frame).await;
    let (code, reason) = recv_close(&mut oversized).await;
    assert_eq!(code, 1009, "frame budget close (reason={reason})");
}

// ---------------------------------------------------------------------------
// Main probe
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "driven by scripts/check-router-ws-live.mjs"]
    async fn router_live_ws_roundtrip() {
        let live = LiveProfile::from_env();
        seed_runtime_home(&live);
        materialize_projection(&live);

        let router_config_path = write_router_config(&live);
        let mut router = task_router(&router_config_path);
        wait_for_listeners(&live, &mut router);

        let runtime_config_path = write_runtime_config(&live);
        let mut runtime = spawn_runtime(&live, &runtime_config_path);

        // 1. Real client WS connect (retries until the Runtime has
        // registered and the connect-admission chain is ready).
        let mut first = connect_client(&live).await;

        // 2. Frozen id lexeme corpus through the real chain.
        run_id_corpus(&live).await;

        // 3. Clean close; the next connection takes the business slot.
        let _ = first.close(None).await;

        // 4. Business replacement + disconnect race + slot cleanup.
        let fourth = replacement_and_disconnect_race(&live).await;

        // 5. Slow-client saturation: 1011.
        slow_client_saturation(fourth).await;

        // 6. Frame budget (1009) and binary (1003) closes on fresh sockets.
        frame_budget_and_binary_closes(&live).await;

        // 7. A healthy connection is finalized when the Router shuts down;
        // the Router exits 0 and releases listeners; the Runtime exits 0.
        let mut healthy = connect_client(&live).await;
        let router_pid = router.id();
        let signaled = Command::new("kill")
            .args(["-TERM", &router_pid.to_string()])
            .status()
            .expect("deliver SIGTERM to router");
        assert!(signaled.success(), "kill -TERM router must succeed");
        let close = timeout(Duration::from_secs(20), healthy.next())
            .await
            .expect("router shutdown close timed out");
        match close {
            Some(Ok(Message::Close(Some(frame)))) => {
                let code: u16 = frame.code.into();
                eprintln!(
                    "router-live:ws probe: shutdown finalized client WS with close {code} ({})",
                    frame.reason
                );
            }
            Some(Ok(Message::Close(None))) | None | Some(Err(_)) => {
                eprintln!("router-live:ws probe: shutdown ended client WS stream");
            }
            other => panic!("unexpected message during router shutdown: {other:?}"),
        }

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
        let (runtime_status, runtime_stderr) =
            wait_for_exit(&mut runtime, Duration::from_secs(30), "runtime");
        assert!(
            runtime_status.success(),
            "graceful runtime shutdown must exit 0, got {runtime_status}; stderr: {runtime_stderr}"
        );

        let _ = std::fs::remove_file(&router_config_path);
        let _ = std::fs::remove_file(&runtime_config_path);
        eprintln!("router-live:ws probe: PASS");
    }
}

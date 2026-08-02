//! Real boundary probe: real client WebSocket (hyper upgrade +
//! tokio-tungstenite) through the production `WebSocketLane` to a fake
//! dispatcher / fake runtime (C-ws §5.8, C-model-connection §6.8).
//!
//! Asserts canonical `1e0` -> `1` / `-0` -> `0` terminal frame bytes,
//! platform error frames, Runtime outbound RPC roundtrip, business
//! replacement close (1008), profile budget close (1009) and shutdown close
//! (1001) over real sockets.

mod ws_harness;

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use skiff_router::ws::{
    AttachMeta, BrokerRuntimeSource, BusinessKey, InboundDispatchResult, OverflowPolicy,
    PeerWriter, WebSocketLane, WebSocketLaneOptions,
};
use skiff_runtime_transport::connection_protocol::WebSocketRpcProfile;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, Role};
use tokio_tungstenite::WebSocketStream;

use ws_harness::{
    FakeDispatchInbound, FakeMethodCatalog, FakeRuntimePeer, FakeRuntimeResponder,
    FakeRuntimeSessionClose, FakeRuntimeViolationSink,
};

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);

enum Outbound {
    Text(String),
    Close(u16, String),
}

/// Real socket single-writer adapter: bounded queue + writer task; terminate
/// aborts the socket immediately (C-client-lifecycle §3.4).
#[derive(Debug)]
struct SocketPeerWriter {
    tx: mpsc::Sender<Outbound>,
    buffered: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl PeerWriter for SocketPeerWriter {
    fn write_text(&self, frame: String) -> Result<(), String> {
        let bytes = frame.len() as u64;
        self.tx
            .try_send(Outbound::Text(frame))
            .map_err(|_| "writer queue full".to_string())?;
        self.buffered.fetch_add(bytes, Ordering::SeqCst);
        Ok(())
    }

    fn buffered_bytes(&self) -> u64 {
        self.buffered.load(Ordering::SeqCst)
    }

    fn close(&self, code: u16, reason: &str) -> Result<(), String> {
        if self
            .tx
            .try_send(Outbound::Close(code, reason.to_string()))
            .is_err()
        {
            // Slow-client overflow: never wait for the queue to accept the
            // close frame; abort the socket.
            self.task.abort();
        }
        Ok(())
    }

    fn terminate(&self) {
        self.task.abort();
    }
}

async fn writer_loop<S>(
    mut write_half: SplitSink<WebSocketStream<S>, Message>,
    mut rx: mpsc::Receiver<Outbound>,
    buffered: Arc<AtomicU64>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(command) = rx.recv().await {
        match command {
            Outbound::Text(text) => {
                let bytes = text.len() as u64;
                let sent = write_half.send(Message::Text(text.into())).await;
                buffered.fetch_sub(bytes, Ordering::SeqCst);
                if sent.is_err() {
                    break;
                }
            }
            Outbound::Close(code, reason) => {
                let _ = write_half
                    .send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Iana(code),
                        reason: reason.into(),
                    })))
                    .await;
                break;
            }
        }
    }
    let _ = write_half.send(Message::Close(None)).await;
}

struct ProbeServer {
    addr: SocketAddr,
    lane: Arc<WebSocketLane>,
    dispatch: Arc<FakeDispatchInbound>,
    responder: Arc<FakeRuntimeResponder>,
    close_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ProbeServer {
    async fn start() -> Self {
        let dispatch = Arc::new(FakeDispatchInbound::new());
        let responder = Arc::new(FakeRuntimeResponder::new());
        let lane = WebSocketLane::new(
            WebSocketLaneOptions {
                index: skiff_router::ws::ClientConnectionIndexOptions {
                    connection_limit: 8,
                    slow_client_budget_bytes: 1024 * 1024,
                    high_water_capacity: 8,
                },
                ..Default::default()
            },
            Arc::new(FakeRuntimePeer::new()),
            Arc::new(FakeRuntimeSessionClose::new()),
            Arc::new(skiff_router::ws::AllowAnyPendingAdmission),
            Arc::new(FakeMethodCatalog::new()),
            Arc::new(skiff_router::ws::NoopNotificationObserver),
            Arc::new(FakeRuntimeViolationSink::new()),
            dispatch.clone(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let connections = Arc::new(AtomicU64::new(0));
        let (close_tx, close_rx) = watch::channel(false);
        let task = tokio::spawn(run_server(listener, lane.clone(), connections, close_rx));
        Self {
            addr,
            lane,
            dispatch,
            responder,
            close_tx,
            task,
        }
    }

    fn url(&self, user: Option<&str>) -> String {
        match user {
            Some(user) => format!("ws://{}/ws?user={user}", self.addr),
            None => format!("ws://{}/ws", self.addr),
        }
    }
}

async fn run_server(
    listener: TcpListener,
    lane: Arc<WebSocketLane>,
    connections: Arc<AtomicU64>,
    mut close_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = close_rx.changed() => {
                let _ = changed;
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = match accepted {
                    Ok(accepted) => accepted,
                    Err(_) => continue,
                };
                let lane = lane.clone();
                let connections = connections.clone();
                tokio::spawn(handle_connection(stream, lane, connections));
            }
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    lane: Arc<WebSocketLane>,
    connections: Arc<AtomicU64>,
) {
    let builder = http1::Builder::new();
    let service = service_fn(move |request: Request<Incoming>| {
        let lane = lane.clone();
        let connections = connections.clone();
        async move { upgrade_response(request, lane, connections).await }
    });
    let connection = builder
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades();
    let _ = connection.await;
}

async fn upgrade_response(
    request: Request<Incoming>,
    lane: Arc<WebSocketLane>,
    connections: Arc<AtomicU64>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let key = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .expect("websocket key");
    let accept = derive_accept_key(key.as_bytes());
    let connection_id = format!("probe-{}", connections.fetch_add(1, Ordering::SeqCst));
    let business_key = request.uri().query().and_then(|query| {
        query.split('&').find_map(|pair| {
            pair.strip_prefix("user=")
                .map(|user| BusinessKey::from_parts("example.com/chat", &websocket_entry(), user))
        })
    });
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = StatusCode::SWITCHING_PROTOCOLS;
    response
        .headers_mut()
        .insert(UPGRADE, "websocket".parse().unwrap());
    response
        .headers_mut()
        .insert(CONNECTION, "Upgrade".parse().unwrap());
    response
        .headers_mut()
        .insert(SEC_WEBSOCKET_ACCEPT, accept.parse().unwrap());

    tokio::spawn(async move {
        if let Ok(upgraded) = hyper::upgrade::on(request).await {
            let socket =
                WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
            let (write_half, read_half) = socket.split();
            let (tx, rx) = mpsc::channel::<Outbound>(64);
            let buffered = Arc::new(AtomicU64::new(0));
            let writer_task = tokio::spawn(writer_loop(write_half, rx, buffered.clone()));
            let writer: Arc<dyn PeerWriter> = Arc::new(SocketPeerWriter {
                tx,
                buffered,
                task: writer_task,
            });
            lane.reserve(&connection_id).expect("reserve");
            let _ = lane.admit(
                &connection_id,
                business_key,
                None,
                1,
                OverflowPolicy::CloseOldest,
            );
            let generation = connection_id
                .trim_start_matches("probe-")
                .parse::<u64>()
                .unwrap_or(1);
            lane.attach(
                &connection_id,
                generation,
                connection_id.clone(),
                ws_harness::runtime_session("probe-runtime"),
                writer,
                AttachMeta {
                    service_id: "example.com/chat".to_string(),
                    websocket_entry_id: websocket_entry(),
                    profile: WebSocketRpcProfile::JsonRpc2_0Text,
                },
            )
            .expect("attach");
            reader_loop(lane, connection_id, read_half).await;
        }
    });
    Ok(response)
}

async fn reader_loop<S>(
    lane: Arc<WebSocketLane>,
    connection_id: String,
    mut read_half: SplitStream<WebSocketStream<S>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = read_half.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let _ = lane.handle_peer_text(&connection_id, text.as_bytes());
            }
            Ok(Message::Binary(_)) => {
                let _ = lane.handle_peer_binary(&connection_id);
            }
            Ok(Message::Close(_)) => {
                let _ = lane.handle_peer_disconnect(&connection_id);
                break;
            }
            Ok(Message::Pong(_)) | Ok(Message::Ping(_)) => {}
            Err(_) => {
                let _ = lane.handle_peer_disconnect(&connection_id);
                break;
            }
            Ok(Message::Frame(_)) => {}
        }
    }
}

fn websocket_entry() -> String {
    format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64))
}

async fn connect(url: &str) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>> {
    timeout(PROBE_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .expect("connect timeout")
        .expect("connect")
        .0
}

async fn next_text(
    socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
) -> String {
    loop {
        let message = timeout(PROBE_TIMEOUT, socket.next())
            .await
            .expect("read timeout")
            .expect("stream ended");
        match message {
            Ok(Message::Text(text)) => return text.to_string(),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            other => panic!("unexpected message {other:?}"),
        }
    }
}

async fn next_close(
    socket: &mut WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
) -> (u16, String) {
    loop {
        let message = timeout(PROBE_TIMEOUT, socket.next())
            .await
            .expect("close timeout")
            .expect("stream ended without close");
        match message {
            Ok(Message::Close(Some(frame))) => {
                return (frame.code.into(), frame.reason.to_string())
            }
            Ok(Message::Close(None)) => return (1005, String::new()),
            Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
            other => panic!("unexpected message while waiting for close {other:?}"),
        }
    }
}

async fn wait_for_dispatch(
    server: &ProbeServer,
    connection_id: &str,
    expected_peer_key: &str,
) -> skiff_router::ws::InboundDispatchAction {
    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        if let Some(action) = server.dispatch.actions().into_iter().find(|action| {
            action.connection_id == connection_id
                && action.peer_id.canonical_key() == expected_peer_key
        }) {
            return action;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dispatch action for {connection_id}/{expected_peer_key} never arrived"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

async fn wait_for_finalizers(lane: &Arc<WebSocketLane>) {
    let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
    loop {
        if lane.snapshot().finalizer_pending == 0 {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "finalizer did not drain"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn real_client_ws_to_fake_dispatcher_roundtrip_and_terminal_codes() {
        let server = ProbeServer::start().await;
        let lane = server.lane.clone();

        // 1. Canonical `1e0` request -> peer key n:1 -> terminal frame id 1.
        let mut a = connect(&server.url(Some("u1"))).await;
        a.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":1e0,"method":"chat.send","params":{}}"#.into(),
        ))
        .await
        .expect("send 1e0");
        let action = wait_for_dispatch(&server, "probe-0", "n:1").await;
        assert_eq!(
            action.peer_id.canonical_key(),
            "n:1",
            "1e0 must canonicalize to n:1"
        );
        lane.complete_inbound(
            &action.execution_token,
            InboundDispatchResult::Success {
                result: br#"{"ok":1}"#.to_vec(),
            },
        );
        assert_eq!(
            next_text(&mut a).await,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":1}}"#,
            "1e0 terminal frame must carry canonical id 1"
        );

        // 2. `-0` request -> canonical id 0.
        a.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":-0,"method":"chat.send","params":[]}"#.into(),
        ))
        .await
        .expect("send -0");
        let action = wait_for_dispatch(&server, "probe-0", "n:0").await;
        assert_eq!(action.peer_id.canonical_key(), "n:0");
        lane.complete_inbound(
            &action.execution_token,
            InboundDispatchResult::Success {
                result: br#"{"ok":2}"#.to_vec(),
            },
        );
        assert_eq!(
            next_text(&mut a).await,
            r#"{"jsonrpc":"2.0","id":0,"result":{"ok":2}}"#,
            "-0 terminal frame must carry canonical id 0"
        );

        // 3. Invalid numeric id -> platform error with id null.
        a.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":1.5,"method":"chat.send","params":{}}"#.into(),
        ))
        .await
        .expect("send invalid id");
        assert_eq!(
            next_text(&mut a).await,
            r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid Request"}}"#
        );

        // 4. Runtime outbound RPC: client receives `probe-0:0` and responds.
        let owner = lane.broker.owner_token("probe-0").expect("owner").0;
        let source = BrokerRuntimeSource {
            sender: ws_harness::runtime_session("probe-runtime"),
            session_token: "session-probe-runtime".to_string(),
            respond: server.responder.clone(),
        };
        let runtime_request = skiff_router::ws::RuntimeRequest {
            request_id: "probe-req".to_string(),
            service_id: "example.com/chat".to_string(),
            websocket_entry_id: websocket_entry(),
            owner_token: owner,
            profile: WebSocketRpcProfile::JsonRpc2_0Text,
            method: "chat.send".to_string(),
            payload: br#"{"n":1}"#.to_vec(),
            deadline: None,
        };
        assert_eq!(
            lane.handle_runtime_request("probe-0", &source, &runtime_request),
            skiff_router::ws::RuntimeRequestOutcome::Success
        );
        assert_eq!(
            next_text(&mut a).await,
            r#"{"jsonrpc":"2.0","id":"probe-0:0","method":"chat.send","params":{"n":1}}"#,
            "outbound runtime request must reach the peer with the generated id"
        );
        a.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":"probe-0:0","result":"ok"}"#.into(),
        ))
        .await
        .expect("send outbound response");
        let deadline = tokio::time::Instant::now() + PROBE_TIMEOUT;
        loop {
            if let Some(response) = server
                .responder
                .responses()
                .into_iter()
                .find(|response| response.request_id == "probe-req")
            {
                assert_eq!(
                    response.outcome,
                    skiff_runtime_transport::connection_protocol::ConnectionResponseOutcome::Success
                );
                assert_eq!(response.payload, br#""ok""#);
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "runtime response never settled"
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        }

        // 5. Business replacement: a second client with the same business key
        // closes the first (1008) and stays functional.
        let mut b = connect(&server.url(Some("u1"))).await;
        let (code, reason) = next_close(&mut a).await;
        assert_eq!(code, 1008, "replacement close code (reason={reason})");
        assert_eq!(
            lane.index.connection_terminal("probe-0").as_deref(),
            Some("Replacement")
        );
        b.send(Message::Text(
            r#"{"jsonrpc":"2.0","id":1e0,"method":"chat.send","params":{}}"#.into(),
        ))
        .await
        .expect("send on replacement");
        let action = wait_for_dispatch(&server, "probe-1", "n:1").await;
        lane.complete_inbound(
            &action.execution_token,
            InboundDispatchResult::Success {
                result: br#"{"ok":3}"#.to_vec(),
            },
        );
        assert_eq!(
            next_text(&mut b).await,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":3}}"#,
            "replacement generation must be independent"
        );

        // 6. Profile budget: depth-65 frame closes 1009.
        let deep = format!("{}1{}", "[".repeat(65), "]".repeat(65));
        b.send(Message::Text(deep.into()))
            .await
            .expect("send deep frame");
        let (code, reason) = next_close(&mut b).await;
        assert_eq!(code, 1009, "profile budget close (reason={reason})");

        // 7. Shutdown closes a healthy connection with 1001.
        let mut c = connect(&server.url(None)).await;
        let _ = lane.shutdown();
        let (code, reason) = next_close(&mut c).await;
        assert_eq!(code, 1001, "shutdown close (reason={reason})");

        wait_for_finalizers(&lane).await;
        let health = lane.snapshot();
        assert_eq!(health.connection_count, 0);
        assert_eq!(health.generation_count, 0);
        assert_eq!(health.finalizer_pending, 0);
        assert_eq!(health.outbound_pending, 0);
        assert_eq!(health.inbound_pending, 0);

        let _ = server.close_tx.send(true);
        let _ = timeout(PROBE_TIMEOUT, server.task).await;
    }
}

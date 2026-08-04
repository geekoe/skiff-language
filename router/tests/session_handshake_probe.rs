//! Real-socket handshake probe: a fake Runtime peer drives the W-session
//! `/runtime` endpoint byte-by-byte from the contracts-session corpus.
//!
//! C-model-registration §5.8: assert the wire `router.bootstrap` and
//! `runtime.registered` frames are byte-identical to the fixtures, negative
//! frames close the connection with zero directory residue, replacement and
//! pre-auth limits behave, and shutdown drains via the barrier.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_router::config::RouterConfig;
use skiff_router::listener::{start_listeners_with_session, ListenerStartOptions, RouterListeners};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::demux::InboundSinkSet;
use skiff_router::session::health::RuntimeHealthLedger;
use skiff_router::session::identity::RegisteredAssemblyTuple;
use skiff_router::session::layer::{SessionLayer, SessionLayerOptions, SessionTiming};
use skiff_router::supervisor::session_ports::SessionHandle;
use skiff_router::supervisor::sinks::ConnectionFrameSink;
use skiff_router::ws::types::{
    EmptyMethodCatalog, InboundDispatchAction, NoopNotificationObserver, NoopRuntimeViolationSink,
};
use skiff_router::ws::{
    AllowAnyPendingAdmission, DispatchInbound, RuntimeGenerationPeer, RuntimeSessionClose,
    WebSocketLane, WebSocketLaneOptions,
};
use skiff_runtime_transport::protocol::{
    encode_binary_frame, ConnectionSendFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

fn corpus_frames() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("registration-handshake")
        .join("frames.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
        .expect("frames.json parses")
}

fn frame(frames: &Value, name: &str) -> Vec<u8> {
    hex_decode(
        frames["frames"][name]["frameHex"]
            .as_str()
            .unwrap_or_else(|| panic!("frame {name} missing")),
    )
}

fn test_config(runtime_max_concurrency: u64) -> RouterConfig {
    RouterConfig {
        activation_prepare_timeout_ms: 120_000,
        artifacts_path: "/opt/skiff/artifacts".into(),
        dev_reload: None,
        host: "127.0.0.1".to_string(),
        http_max_request_bytes: 1,
        http_max_response_bytes: 8_388_608,
        http_port: 4000,
        manifests: vec![],
        profile: "dev".to_string(),
        release_mode: None,
        request_timeout_ms: 20_000,
        rewrite: vec![],
        runtime_path: "/runtime".to_string(),
        runtime_port: 4001,
        runtime_max_concurrency,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

fn committed_epoch() -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: "prod".to_string(),
        generation: 42,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        },
        config_snapshot: RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("snapshot id"),
        },
    }
}

fn layer_options(timing: Option<SessionTiming>) -> SessionLayerOptions {
    SessionLayerOptions {
        committed_epoch: Some(committed_epoch()),
        pending_epoch: None,
        manifest: ConsumerManifest::default_installed(),
        consumers: vec![Arc::new(RuntimeHealthLedger::new())],
        timing: timing.unwrap_or_default(),
        budgets: Default::default(),
        writer_delay: None,
    }
}

async fn start(
    config: RouterConfig,
    timing: Option<SessionTiming>,
    writer_delay: Option<Duration>,
) -> (RouterListeners, Arc<SessionLayer>) {
    let mut options = layer_options(timing);
    options.writer_delay = writer_delay;
    let layer =
        Arc::new(SessionLayer::with_options(config.clone(), options).expect("layer options valid"));
    let listeners = start_listeners_with_session(
        &config,
        &ListenerStartOptions {
            public_bind: Some("127.0.0.1:0".parse().expect("public bind")),
            runtime_control_bind: Some("127.0.0.1:0".parse().expect("control bind")),
            drain_deadline: Duration::from_secs(2),
        },
        Arc::clone(&layer),
    )
    .await
    .expect("listeners start");
    (listeners, layer)
}

type PeerSocket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr) -> PeerSocket {
    let url = format!("ws://{addr}/runtime");
    let (socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
        .await
        .expect("WS handshake timed out")
        .expect("WS handshake failed");
    assert_eq!(response.status(), 101);
    socket
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

async fn expect_closed(socket: &mut PeerSocket) {
    let outcome = timeout(CLIENT_TIMEOUT, socket.next())
        .await
        .expect("close timed out");
    assert!(
        matches!(outcome, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
        "expected closed connection, got {outcome:?}"
    );
}

async fn send_binary(socket: &mut PeerSocket, bytes: Vec<u8>) {
    timeout(CLIENT_TIMEOUT, socket.send(Message::Binary(bytes.into())))
        .await
        .expect("send timed out")
        .expect("send failed");
}

async fn complete_handshake(socket: &mut PeerSocket, frames: &Value) {
    let bootstrap = recv_binary(socket).await;
    assert_eq!(
        bootstrap,
        frame(frames, "bootstrap.prod.42"),
        "wire router.bootstrap must match the corpus bytes"
    );
    send_binary(socket, frame(frames, "capabilities.runtime-a")).await;
    send_binary(socket, frame(frames, "register.prod.42.a")).await;
    let ack = recv_binary(socket).await;
    assert_eq!(
        ack,
        frame(frames, "registered.runtime-a"),
        "wire runtime.registered must match the corpus bytes"
    );
}

fn connection_send_frame(connection_id: &str, both_targets: bool) -> Vec<u8> {
    let header = ConnectionSendFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "connection.send".to_string(),
        service_id: "test.skiff/router-rust-ws-live".to_string(),
        websocket_entry_id: Some(
            "skiff-websocket-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        ),
        business_identity: both_targets.then(|| "alice".to_string()),
        connection_id: Some(connection_id.to_string()),
        payload_kind: Some("text".to_string()),
    };
    encode_binary_frame(&header, br#"{"eventName":"chat/text-delta"}"#).expect("encode send")
}

mod noop_ws_ports {
    use super::*;

    #[derive(Debug)]
    pub struct NoopRuntimeGenerationPeer;

    impl RuntimeGenerationPeer for NoopRuntimeGenerationPeer {
        fn send_control(
            &self,
            _runtime: &skiff_router::session::RuntimeSessionEpoch,
            _control: &skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleControl,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    #[derive(Debug)]
    pub struct NoopRuntimeSessionClose;

    impl RuntimeSessionClose for NoopRuntimeSessionClose {
        fn close_session(
            &self,
            _runtime: &skiff_router::session::RuntimeSessionEpoch,
            _code: u16,
            _reason: &str,
        ) {
        }
    }

    #[derive(Debug)]
    pub struct NoopDispatchInbound;

    impl DispatchInbound for NoopDispatchInbound {
        fn dispatch(&self, _action: InboundDispatchAction) -> Result<(), String> {
            Err("noop dispatch".to_string())
        }
    }
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
    while !condition() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not met within timeout"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn accept_sequence_is_byte_exact_and_health_becomes_observation() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(4), None, None).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut socket, &frames).await;
        send_binary(&mut socket, frame(&frames, "health.empty")).await;

        wait_until(|| layer.health().observed_total() == 1).await;
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 1);
        assert_eq!(snapshot.health_before_ack, 0);
        assert_eq!(snapshot.pre_auth_connections, 0, "ACK released pre-auth");
        assert_eq!(
            layer.candidates(&committed_epoch()),
            vec![skiff_router::session::RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 1,
            }]
        );
        let _ = socket.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 0);
        assert_eq!(snapshot.live_session_tasks, 0);
        assert_eq!(snapshot.consumer_permits_held, 0);
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_register_pre_ack_closes_with_zero_directory_residue() {
        let frames = corpus_frames();
        // Delay the ACK write so the duplicate register is deterministically
        // processed in RegisterValidated (before the ACK reaches the wire).
        let (listeners, layer) =
            start(test_config(4), None, Some(Duration::from_millis(300))).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut socket).await;
        send_binary(&mut socket, frame(&frames, "capabilities.runtime-a")).await;
        send_binary(&mut socket, frame(&frames, "register.prod.42.a")).await;
        send_binary(&mut socket, frame(&frames, "register.prod.42.a")).await;
        expect_closed(&mut socket).await;
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 0);
        assert_eq!(snapshot.pending_sessions, 0);
        assert_eq!(snapshot.pre_auth_connections, 0);
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn negative_handshake_frames_close_connection() {
        let frames = corpus_frames();
        let cases: [(&str, bool); 4] = [
            // (frame sent after capabilities, expect register-before-capabilities too)
            ("register.prod.42.a", false),
            ("register.prod.41.a", true),
            ("register.prod.42.b", true),
            ("legacy.runtime.register", true),
        ];
        for (negative_frame, after_capabilities) in cases {
            let (listeners, layer) = start(test_config(4), None, None).await;
            let mut socket = connect(listeners.runtime_control.addr()).await;
            let _ = recv_binary(&mut socket).await;
            if after_capabilities {
                send_binary(&mut socket, frame(&frames, "capabilities.runtime-a")).await;
            }
            send_binary(&mut socket, frame(&frames, negative_frame)).await;
            expect_closed(&mut socket).await;
            assert_eq!(layer.health_snapshot().registered_sessions, 0);
            assert_eq!(layer.health_snapshot().pending_sessions, 0);
            listeners.shutdown().await.expect("graceful shutdown");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replacement_cancels_old_session_and_old_barrier_never_deletes_new() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(4), None, None).await;
        let mut first = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut first, &frames).await;

        let mut second = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut second, &frames).await;

        // Old connection must be cancelled by the replacement and closed.
        expect_closed(&mut first).await;
        wait_until(|| layer.health_snapshot().registered_sessions == 1).await;
        assert_eq!(
            layer.candidates(&committed_epoch()),
            vec![skiff_router::session::RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 2,
            }],
            "old close barrier must not delete the replacement"
        );
        let _ = second.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pre_auth_limit_rejects_overflow_and_releases_permit_on_ack() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(1), None, None).await;

        let mut first = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut first).await;

        // Second connection is refused without entering the handshake.
        let mut refused = connect(listeners.runtime_control.addr()).await;
        expect_closed(&mut refused).await;
        assert_eq!(layer.health_snapshot().pre_auth_refused, 1);

        // Complete the first handshake: the pre-auth permit releases on ACK.
        send_binary(&mut first, frame(&frames, "capabilities.runtime-a")).await;
        send_binary(&mut first, frame(&frames, "register.prod.42.a")).await;
        let _ = recv_binary(&mut first).await;
        wait_until(|| layer.health_snapshot().pre_auth_connections == 0).await;

        // Now a new connection is accepted.
        let mut third = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut third).await;
        let _ = third.close(None).await;
        let _ = first.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handshake_timeouts_close_connection_and_release_pre_auth() {
        let frames = corpus_frames();
        let timing = SessionTiming {
            bootstrap: Duration::from_secs(10),
            capabilities: Duration::from_millis(150),
            register: Duration::from_millis(150),
            ack_write: Duration::from_secs(5),
            close_barrier: Duration::from_secs(2),
            shutdown_total: Duration::from_secs(5),
        };
        let (listeners, layer) = start(test_config(2), Some(timing), None).await;

        let mut no_capabilities = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut no_capabilities).await;
        expect_closed(&mut no_capabilities).await;

        let mut no_register = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut no_register).await;
        send_binary(&mut no_register, frame(&frames, "capabilities.runtime-a")).await;
        expect_closed(&mut no_register).await;

        wait_until(|| layer.health_snapshot().pre_auth_connections == 0).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 0);
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_text_frame_closes_connection() {
        let (listeners, layer) = start(test_config(4), None, None).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut socket).await;
        timeout(
            CLIENT_TIMEOUT,
            socket.send(Message::text("not a skiff frame")),
        )
        .await
        .expect("send timed out")
        .expect("send failed");
        expect_closed(&mut socket).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 0);
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_closes_all_sessions_via_barrier_with_zero_residue() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(4), None, None).await;
        let mut first = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut first, &frames).await;
        send_binary(&mut first, frame(&frames, "health.empty")).await;

        let mut second = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut second).await;
        send_binary(&mut second, frame(&frames, "capabilities.runtime-b")).await;
        send_binary(&mut second, frame(&frames, "register.prod.42.b")).await;
        let _ = recv_binary(&mut second).await;

        wait_until(|| layer.health_snapshot().registered_sessions == 2).await;
        listeners.shutdown().await.expect("graceful shutdown");
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 0);
        assert_eq!(snapshot.pending_sessions, 0);
        assert_eq!(snapshot.cancelled_sessions, 0);
        assert_eq!(snapshot.barrier_pending, 0);
        assert_eq!(snapshot.consumer_permits_held, 0);
        assert_eq!(snapshot.live_session_tasks, 0);
        assert_eq!(snapshot.pre_auth_connections, 0);
        assert_eq!(snapshot.observed_health, 1);
        assert!(snapshot.fail_stop.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handshake_deadline_does_not_fire_after_registration() {
        let frames = corpus_frames();
        let timing = SessionTiming {
            bootstrap: Duration::from_millis(150),
            capabilities: Duration::from_millis(150),
            register: Duration::from_millis(150),
            ack_write: Duration::from_secs(5),
            close_barrier: Duration::from_secs(2),
            shutdown_total: Duration::from_secs(5),
        };
        let (listeners, layer) = start(test_config(4), Some(timing), None).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut socket, &frames).await;
        // Every handshake deadline has long expired; a registered session
        // must never be closed by the bootstrap/capabilities/register window.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 1);
        assert_eq!(snapshot.live_session_tasks, 1);
        // The connection is still usable: a health frame is observed.
        send_binary(&mut socket, frame(&frames, "health.empty")).await;
        wait_until(|| layer.health().observed_total() == 1).await;
        let _ = socket.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cold_start_capabilities_within_window_registers() {
        let frames = corpus_frames();
        // Cold-start parity: a fresh Runtime may provision its whole-assembly
        // service DB indexes before sending capabilities; the window must
        // cover that delay instead of killing the first connection.
        let timing = SessionTiming {
            bootstrap: Duration::from_secs(10),
            capabilities: Duration::from_secs(3),
            register: Duration::from_secs(3),
            ack_write: Duration::from_secs(5),
            close_barrier: Duration::from_secs(2),
            shutdown_total: Duration::from_secs(5),
        };
        let (listeners, layer) = start(test_config(4), Some(timing), None).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        let _ = recv_binary(&mut socket).await;
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        send_binary(&mut socket, frame(&frames, "capabilities.runtime-a")).await;
        send_binary(&mut socket, frame(&frames, "register.prod.42.a")).await;
        let _ = recv_binary(&mut socket).await;
        wait_until(|| layer.health_snapshot().registered_sessions == 1).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 1);
        let _ = socket.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reconnect_registers_again_after_disconnect() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(4), None, None).await;
        let mut first = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut first, &frames).await;
        let _ = first.close(None).await;
        wait_until(|| layer.health_snapshot().registered_sessions == 0).await;

        let mut second = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut second, &frames).await;
        wait_until(|| layer.health_snapshot().registered_sessions == 1).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 1);
        assert_eq!(
            layer.candidates(&committed_epoch()),
            vec![skiff_router::session::RuntimeSessionEpoch {
                replica_id: "runtime-a".to_string(),
                connection_generation: 2,
            }]
        );
        let _ = second.close(None).await;
        listeners.shutdown().await.expect("graceful shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connection_send_delivery_miss_keeps_registered_session_alive() {
        let frames = corpus_frames();
        let (listeners, layer) = start(test_config(4), None, None).await;
        let lane = WebSocketLane::new(
            WebSocketLaneOptions::default(),
            Arc::new(noop_ws_ports::NoopRuntimeGenerationPeer),
            Arc::new(noop_ws_ports::NoopRuntimeSessionClose),
            Arc::new(AllowAnyPendingAdmission),
            Arc::new(EmptyMethodCatalog),
            Arc::new(NoopNotificationObserver),
            Arc::new(NoopRuntimeViolationSink),
            Arc::new(noop_ws_ports::NoopDispatchInbound),
        );
        let handle = SessionHandle::new();
        handle.set(Arc::clone(&layer));
        layer.install_inbound_sinks(Arc::new(InboundSinkSet {
            connection: Some(Arc::new(ConnectionFrameSink::new(lane, handle))),
            ..Default::default()
        }));

        let mut socket = connect(listeners.runtime_control.addr()).await;
        complete_handshake(&mut socket, &frames).await;

        // A `connection.send` for an unknown client connection is a delivery
        // miss (TS parity: warn, continue) and must not kill the Runtime
        // session.
        send_binary(
            &mut socket,
            connection_send_frame("missing-connection", false),
        )
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 1);

        // An envelope with both connectionId and businessIdentity is a
        // protocol violation and terminates the exact session (TS 1008).
        send_binary(
            &mut socket,
            connection_send_frame("missing-connection", true),
        )
        .await;
        expect_closed(&mut socket).await;
        wait_until(|| layer.health_snapshot().registered_sessions == 0).await;
        assert_eq!(layer.health_snapshot().registered_sessions, 0);
        listeners.shutdown().await.expect("graceful shutdown");
    }
}

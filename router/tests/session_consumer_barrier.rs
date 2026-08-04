//! Consumer manifest / reserved terminal / ACK barrier / fail-stop probes
//! (C-session §3.2(3)/§5, C-process-lifecycle S6).

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use skiff_artifact_model::AssemblyActivationControl;
use skiff_router::config::RouterConfig;
use skiff_router::listener::{start_listeners_with_session, ListenerStartOptions};
use skiff_router::session::consumer::{ConsumerManifest, SessionConsumer};
use skiff_router::session::health::RuntimeHealthLedger;
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::layer::{SessionLayer, SessionLayerOptions, SessionTiming};
use skiff_router::session::ConsumerKind;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, encode_assembly_activation_frame,
    AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, encode_binary_frame, RuntimeCapabilitiesFrameHeader,
};
use tokio::time::timeout;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

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

fn committed_epoch() -> skiff_router::session::RegisteredAssemblyTuple {
    skiff_router::session::RegisteredAssemblyTuple {
        profile: "prod".to_string(),
        generation: 42,
        assembly: skiff_artifact_model::RuntimeAssemblyRef {
            assembly_identity: skiff_artifact_model::AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        },
        config_snapshot: skiff_artifact_model::RuntimeConfigSnapshotRef {
            snapshot_id: skiff_artifact_model::RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("snapshot id"),
        },
    }
}

fn timing() -> SessionTiming {
    SessionTiming {
        bootstrap: Duration::from_secs(10),
        capabilities: Duration::from_secs(10),
        register: Duration::from_secs(10),
        ack_write: Duration::from_secs(5),
        close_barrier: Duration::from_secs(2),
        shutdown_total: Duration::from_secs(5),
    }
}

fn frames() -> serde_json::Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("runtime")
        .join("transport")
        .join("testdata")
        .join("registration-handshake")
        .join("frames.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("frames.json must exist"))
        .expect("frames.json parses")
}

fn frame(frames: &serde_json::Value, name: &str) -> Vec<u8> {
    let hex = frames["frames"][name]["frameHex"]
        .as_str()
        .unwrap_or_else(|| panic!("frame {name} missing"));
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("valid hex"))
        .collect()
}

#[derive(Debug)]
struct AckingConsumer;

impl SessionConsumer for AckingConsumer {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::RequestDispatcher
    }

    fn on_session_closed(&self, _session: &RuntimeSessionEpoch) -> Result<(), String> {
        Ok(())
    }
}

/// Consumer that parks inside cleanup until the test releases it. Optional
/// `parked` signal reports the first cleanup call.
#[derive(Debug)]
struct GatedConsumer {
    parked: Mutex<Option<Sender<()>>>,
    release_rx: Mutex<Receiver<()>>,
}

impl GatedConsumer {
    fn new(parked: Option<Sender<()>>) -> (Arc<Self>, Sender<()>) {
        let (release_tx, release_rx) = mpsc::channel();
        (
            Arc::new(Self {
                parked: Mutex::new(parked),
                release_rx: Mutex::new(release_rx),
            }),
            release_tx,
        )
    }
}

impl SessionConsumer for GatedConsumer {
    fn kind(&self) -> ConsumerKind {
        ConsumerKind::RequestDispatcher
    }

    fn on_session_closed(&self, _session: &RuntimeSessionEpoch) -> Result<(), String> {
        if let Some(tx) = self.parked.lock().expect("parked lock").take() {
            let _ = tx.send(());
        }
        let _ = self.release_rx.lock().expect("release lock").recv();
        Ok(())
    }
}

async fn start_with(
    gated: Arc<dyn SessionConsumer>,
    runtime_max_concurrency: u64,
    timing: SessionTiming,
) -> (skiff_router::listener::RouterListeners, Arc<SessionLayer>) {
    let config = test_config(runtime_max_concurrency);
    let options = SessionLayerOptions {
        committed_epoch: Some(committed_epoch()),
        pending_epoch: None,
        manifest: ConsumerManifest::installed([
            ConsumerKind::HealthLedger,
            ConsumerKind::RequestDispatcher,
        ]),
        consumers: vec![Arc::new(RuntimeHealthLedger::new()), gated],
        timing,
        budgets: Default::default(),
        writer_delay: None,
    };
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

async fn connect(
    addr: std::net::SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (socket, response) = timeout(
        CLIENT_TIMEOUT,
        tokio_tungstenite::connect_async(format!("ws://{addr}/runtime")),
    )
    .await
    .expect("WS handshake timed out")
    .expect("WS handshake failed");
    assert_eq!(response.status(), 101);
    socket
}

async fn recv_binary(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<u8> {
    match timeout(CLIENT_TIMEOUT, socket.next()).await {
        Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)))) => bytes.to_vec(),
        other => panic!("expected binary frame, got {other:?}"),
    }
}

async fn send_binary(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    bytes: Vec<u8>,
) {
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

async fn complete(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    replica: &str,
) {
    let _ = recv_binary(socket).await;
    send_binary(socket, capabilities_bytes(replica)).await;
    send_binary(socket, register_bytes(replica)).await;
    let _ = recv_binary(socket).await;
}

fn capabilities_bytes(replica_id: &str) -> Vec<u8> {
    let bytes = frame(&frames(), "capabilities.runtime-a");
    let (mut header, _): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&bytes).expect("capabilities decodes");
    header.runtime_id = replica_id.to_string();
    encode_binary_frame(&header, &[]).expect("capabilities encodes")
}

fn register_bytes(replica_id: &str) -> Vec<u8> {
    let bytes = frame(&frames(), "register.prod.42.a");
    let control =
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &bytes)
            .expect("register decodes");
    let AssemblyActivationControl::Register {
        profile,
        generation,
        assembly,
        config_snapshot,
        ..
    } = control
    else {
        panic!("expected register");
    };
    let control = AssemblyActivationControl::Register {
        profile,
        generation,
        assembly,
        config_snapshot,
        replica_id: replica_id.to_string(),
    };
    encode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, &control)
        .expect("register encodes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn all_consumer_acks_complete_barrier_with_zero_residue() {
        let (listeners, layer) = start_with(Arc::new(AckingConsumer), 8, timing()).await;
        let mut first = connect(listeners.runtime_control.addr()).await;
        complete(&mut first, "runtime-a").await;
        let mut second = connect(listeners.runtime_control.addr()).await;
        complete(&mut second, "runtime-b").await;

        wait_until(|| layer.health_snapshot().registered_sessions == 2).await;
        // Acking consumer answers immediately: shutdown completes without
        // fail-stop and leaves zero residue.
        listeners.shutdown().await.expect("shutdown must succeed");
        let snapshot = layer.health_snapshot();
        assert_eq!(snapshot.registered_sessions, 0);
        assert_eq!(snapshot.consumer_permits_held, 0);
        assert_eq!(snapshot.live_session_tasks, 0);
        assert!(snapshot.fail_stop.is_none());
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn missing_consumer_ack_times_out_to_fail_stop() {
        let (gated, release) = GatedConsumer::new(None);
        let (listeners, layer) = start_with(gated, 8, timing()).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        complete(&mut socket, "runtime-a").await;
        wait_until(|| layer.health_snapshot().registered_sessions == 1).await;

        let shutdown = listeners.shutdown().await;
        assert!(
            shutdown.is_err(),
            "missing barrier ACK must fail-stop (got {shutdown:?})"
        );
        wait_until(|| {
            layer
                .fail_stop_reason()
                .is_some_and(|reason| reason.contains("ACK timeout"))
        })
        .await;
        // Release the parked consumer so the mailbox task can exit cleanly.
        let _ = release.send(());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reserved_terminal_slot_failure_fail_stops() {
        let (parked_tx, parked_rx) = mpsc::channel();
        let (gated, release) = GatedConsumer::new(Some(parked_tx));
        // Pre-auth limit 1 sizes the reserved terminal lane to one slot. The
        // parked consumer processes session A (lane empty), session B's terminal
        // fills the lane, and session C's delivery then fail-stops.
        let mut long_barrier = timing();
        long_barrier.close_barrier = Duration::from_secs(10);
        long_barrier.shutdown_total = Duration::from_secs(15);
        let (listeners, layer) = start_with(gated, 1, long_barrier).await;
        let mut first = connect(listeners.runtime_control.addr()).await;
        complete(&mut first, "runtime-a").await;
        let mut second = connect(listeners.runtime_control.addr()).await;
        complete(&mut second, "runtime-b").await;
        let mut third = connect(listeners.runtime_control.addr()).await;
        complete(&mut third, "runtime-c").await;
        wait_until(|| layer.health_snapshot().registered_sessions == 3).await;

        // First disconnect parks the consumer inside cleanup; the reserved
        // terminal lane then holds session B's queued terminal.
        let _ = first.close(None).await;
        timeout(CLIENT_TIMEOUT, async { parked_rx.recv() })
            .await
            .expect("consumer parked signal timed out")
            .expect("parked signal");

        // Second disconnect queues behind the parked consumer (lane now full)...
        let _ = second.close(None).await;
        // ...and the third disconnect cannot deliver at all.
        let _ = third.close(None).await;
        wait_until(|| {
            layer
                .fail_stop_reason()
                .is_some_and(|reason| reason.contains("reserved terminal slot"))
        })
        .await;

        // Release the parked consumer for A and then for B's queued terminal.
        let _ = release.send(());
        let _ = release.send(());
        let shutdown = listeners.shutdown().await;
        assert!(
            shutdown.is_err(),
            "fail-stop must surface as a shutdown error (got {shutdown:?})"
        );
    }

    #[test]
    fn session_consumer_manifest_checker_rejects_mismatch() {
        let (gated, _) = GatedConsumer::new(None);
        let options = SessionLayerOptions {
            committed_epoch: Some(committed_epoch()),
            pending_epoch: None,
            // RequestDispatcher consumer registered but missing from the manifest.
            manifest: ConsumerManifest::installed([ConsumerKind::HealthLedger]),
            consumers: vec![Arc::new(RuntimeHealthLedger::new()), gated],
            timing: timing(),
            budgets: Default::default(),
            writer_delay: None,
        };
        let error = SessionLayer::with_options(test_config(8), options).expect_err("must fail");
        assert!(
            error.to_string().contains("consumer manifest mismatch"),
            "unexpected error: {error}"
        );
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
}

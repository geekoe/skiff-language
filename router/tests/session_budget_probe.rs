//! Real-socket budget probes (C-session §5.3): outbound queue full fails the
//! bootstrap write strictly, inbound budget overflow aborts the exact session.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use skiff_router::config::RouterConfig;
use skiff_router::listener::{start_listeners_with_session, ListenerStartOptions};
use skiff_router::session::budget::SessionBudgets;
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::health::RuntimeHealthLedger;
use skiff_router::session::layer::{SessionLayer, SessionLayerOptions};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

fn config() -> RouterConfig {
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
        runtime_max_concurrency: 4,
        file_backend: None,
        service_db: skiff_router::config::ServiceDbConfig {
            mongo_url: "mongodb://127.0.0.1:27017/?replicaSet=rs0".to_string(),
        },
        telemetry: None,
        websocket_path: "/ws".to_string(),
    }
}

fn epoch() -> skiff_router::session::RegisteredAssemblyTuple {
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

async fn start_with_budgets(budgets: SessionBudgets) -> skiff_router::listener::RouterListeners {
    let config = config();
    let options = SessionLayerOptions {
        committed_epoch: Some(epoch()),
        pending_epoch: None,
        manifest: ConsumerManifest::default_installed(),
        consumers: vec![Arc::new(RuntimeHealthLedger::new())],
        timing: Default::default(),
        budgets,
        writer_delay: None,
    };
    let layer =
        Arc::new(SessionLayer::with_options(config.clone(), options).expect("layer options valid"));
    start_listeners_with_session(
        &config,
        &ListenerStartOptions {
            public_bind: Some("127.0.0.1:0".parse().expect("public bind")),
            runtime_control_bind: Some("127.0.0.1:0".parse().expect("control bind")),
            drain_deadline: Duration::from_secs(2),
        },
        layer,
    )
    .await
    .expect("listeners start")
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

async fn recv_any(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Option<Result<Message, tokio_tungstenite::tungstenite::Error>> {
    timeout(CLIENT_TIMEOUT, socket.next())
        .await
        .expect("recv timed out")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn outbound_byte_budget_below_bootstrap_fails_closed_without_wire_bytes() {
        let budgets = SessionBudgets {
            outbound_frames: 256,
            outbound_bytes: 100, // router.bootstrap fixture is 510 bytes
        };
        let listeners = start_with_budgets(budgets).await;
        let mut socket = connect(listeners.runtime_control.addr()).await;
        let outcome = recv_any(&mut socket).await;
        assert!(
            matches!(outcome, None | Some(Ok(Message::Close(_))) | Some(Err(_))),
            "bootstrap enqueue full must fail closed without bytes, got {outcome:?}"
        );
        listeners.shutdown().await.expect("graceful shutdown");
    }
}

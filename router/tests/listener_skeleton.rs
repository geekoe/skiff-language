//! Real-socket probes for the PR 0b listener skeleton assembled from the
//! C-net mechanism: public HTTP empty responses, control HTTP health
//! placeholder, `/runtime` WebSocket upgrade, connection caps and graceful
//! shutdown. No business protocol is implemented or asserted.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use futures_util::SinkExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use skiff_router::config::RouterConfig;
use skiff_router::listener::{start_listeners, ListenerStartOptions, RouterListeners};

const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(runtime_max_concurrency: u64) -> RouterConfig {
        RouterConfig {
            activation_prepare_timeout_ms: 120_000,
            artifacts_path: PathBuf::from("/tmp/skiff-artifacts"),
            dev_reload: None,
            host: "127.0.0.1".to_string(),
            http_max_request_bytes: 1,
            http_max_response_bytes: 1,
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
                mongo_url: "mongodb://127.0.0.1:27017/skiff".to_string(),
            },
            telemetry: None,
            websocket_path: "/ws".to_string(),
        }
    }

    async fn start(config: RouterConfig) -> RouterListeners {
        start_listeners(
            &config,
            &ListenerStartOptions {
                public_bind: Some("127.0.0.1:0".parse().expect("public bind")),
                runtime_control_bind: Some("127.0.0.1:0".parse().expect("control bind")),
                drain_deadline: Duration::from_secs(2),
            },
        )
        .await
        .expect("start listeners")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn public_listener_serves_empty_http() {
        let listeners = start(test_config(4)).await;
        let response = send_empty_request(listeners.public.addr())
            .await
            .expect("public HTTP round trip");
        assert_empty_http_response(&response);
        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_listener_serves_health_placeholder() {
        let listeners = start(test_config(4)).await;
        let response = send_request(listeners.runtime_control.addr(), "/__router/health")
            .await
            .expect("control HTTP round trip");
        assert_empty_http_response(&response);
        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_websocket_upgrade_on_configured_path() {
        let listeners = start(test_config(4)).await;
        let url = format!("ws://{}/runtime", listeners.runtime_control.addr());
        let (mut socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
            .await
            .expect("WS handshake timed out")
            .expect("WS handshake failed");
        assert_eq!(response.status(), 101);
        assert!(
            response.headers().contains_key("sec-websocket-accept"),
            "101 response must carry Sec-WebSocket-Accept"
        );
        let frame = tokio_tungstenite::tungstenite::Message::text(String::new());
        socket.send(frame).await.expect("send empty frame");
        let _ = socket.close(None).await;
        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn public_listener_does_not_upgrade_websocket() {
        let listeners = start(test_config(4)).await;
        let mut stream = connect(listeners.public.addr()).await.expect("connect");
        stream
            .write_all(
                b"GET / HTTP/1.1\r\nhost: probe\r\nconnection: Upgrade\r\nupgrade: websocket\r\nsec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==\r\nsec-websocket-version: 13\r\n\r\n",
            )
            .await
            .expect("send upgrade request");
        let mut response = Vec::new();
        let mut buffer = [0u8; 1024];
        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        while !response.windows(4).any(|window| window == b"\r\n\r\n") {
            assert!(
                tokio::time::Instant::now() < deadline,
                "response headers did not arrive"
            );
            let read = timeout(CLIENT_TIMEOUT, stream.read(&mut buffer))
                .await
                .expect("response read timed out")
                .expect("response read failed");
            assert!(read > 0, "response ended before headers");
            response.extend_from_slice(&buffer[..read]);
        }
        let text = String::from_utf8_lossy(&response);
        assert!(
            text.starts_with("HTTP/1.1 200"),
            "public listener must not upgrade, got: {text:?}"
        );
        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn runtime_connection_limit_rejects_overflow_and_releases() {
        let listeners = start(test_config(1)).await;
        let addr = listeners.runtime_control.addr();

        let mut first = connect(addr).await.expect("first connection");
        first
            .write_all(b"GET / HTTP/1.1\r\nhost: probe\r\n\r\n")
            .await
            .expect("first request");
        let mut head = [0u8; 128];
        let read = timeout(CLIENT_TIMEOUT, first.read(&mut head))
            .await
            .expect("first response timed out")
            .expect("first response read failed");
        assert!(
            String::from_utf8_lossy(&head[..read]).starts_with("HTTP/1.1 200"),
            "first connection must be served"
        );

        let mut second = connect(addr).await.expect("second connection");
        let mut rejection = Vec::new();
        timeout(CLIENT_TIMEOUT, second.read_to_end(&mut rejection))
            .await
            .expect("rejection timed out")
            .expect("rejection read failed");
        assert!(
            String::from_utf8_lossy(&rejection).starts_with("HTTP/1.1 503"),
            "overflowing connection must be rejected, got: {rejection:?}"
        );

        drop(first);
        let deadline = tokio::time::Instant::now() + CLIENT_TIMEOUT;
        let mut accepted_after_release = false;
        while tokio::time::Instant::now() < deadline {
            if let Ok(response) = send_empty_request(addr).await {
                if String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200") {
                    accepted_after_release = true;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            accepted_after_release,
            "server must accept a new connection after the previous one closed"
        );
        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_shutdown_stops_accepting_and_joins() {
        let listeners = start(test_config(4)).await;
        let addr = listeners.runtime_control.addr();
        let response = send_empty_request(addr)
            .await
            .expect("request before shutdown");
        assert_empty_http_response(&response);

        listeners
            .shutdown()
            .await
            .expect("listeners must shut down");
        assert!(
            connect(addr).await.is_err(),
            "listener must stop accepting after shutdown"
        );
    }

    async fn send_empty_request(addr: SocketAddr) -> io::Result<Vec<u8>> {
        send_request(addr, "/").await
    }

    async fn send_request(addr: SocketAddr, path: &str) -> io::Result<Vec<u8>> {
        let mut stream = connect(addr).await?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.1\r\nhost: probe\r\nconnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok(response)
    }

    async fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
        timeout(CLIENT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))?
    }

    fn assert_empty_http_response(response: &[u8]) {
        let text = String::from_utf8_lossy(response);
        assert!(
            text.starts_with("HTTP/1.1 200"),
            "expected 200, got: {text:?}"
        );
        let header_end = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .expect("response must end headers");
        assert!(
            response[header_end + 4..].is_empty(),
            "response body must be empty, got: {text:?}"
        );
    }
}

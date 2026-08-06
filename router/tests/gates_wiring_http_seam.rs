//! E-ws HTTP gateway upgrade seam tests: the additive `HttpGatewayServer`
//! upgrade hook must leave the unset behavior untouched and must hand
//! matching WebSocket upgrade requests to the handler when set.

use std::collections::BTreeMap;
use std::sync::Arc;

use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use skiff_router::http::fake::FakeHttpDispatcher;
use skiff_router::http::ingress::StoreHttpIngressResolver;
use skiff_router::http::{
    start_http_gateway, GatewayUpgradeHandler, GatewayUpgradeOptions, HttpGatewayServer,
    HttpGatewayServerOptions, HttpGatewaySurfaceView,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Debug)]
struct MarkerHandler;

#[async_trait::async_trait]
impl GatewayUpgradeHandler for MarkerHandler {
    async fn handle(
        &self,
        _request: Request<Incoming>,
    ) -> Result<Response<BoxBody<bytes::Bytes, hyper::Error>>, hyper::Error> {
        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header("x-upgrade-handler", "called")
            .body(
                Full::new(bytes::Bytes::new())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("static response"))
    }
}

fn empty_resolver() -> Arc<StoreHttpIngressResolver> {
    Arc::new(StoreHttpIngressResolver::new(
        Arc::new(
            HttpGatewaySurfaceView::from_deployment_gateway_entries(&BTreeMap::new())
                .expect("empty surface"),
        ),
        "prod",
    ))
}

async fn start(seam: bool) -> (HttpGatewayServer, std::net::SocketAddr) {
    let options = HttpGatewayServerOptions::new(
        "127.0.0.1:0".parse().expect("bind"),
        1024 * 1024,
        1024 * 1024,
    );
    let options = if seam {
        HttpGatewayServerOptions {
            websocket_upgrade: Some(GatewayUpgradeOptions {
                path: "/ws".to_string(),
                handler: Arc::new(MarkerHandler),
            }),
            ..options
        }
    } else {
        options
    };
    let server = start_http_gateway(
        options,
        empty_resolver(),
        Arc::new(FakeHttpDispatcher::new(vec![])),
    )
    .await
    .expect("gateway");
    let addr = server.addr();
    (server, addr)
}

async fn raw_upgrade(addr: std::net::SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\
         Connection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write upgrade request");
    // Read only the response head (keep-alive connections stay open after
    // the headers, so `read_to_end` would block).
    let mut response = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut buffer = [0u8; 1024];
    while !response.windows(4).any(|window| window == b"\r\n\r\n") {
        assert!(
            tokio::time::Instant::now() < deadline,
            "response head timed out: {}",
            String::from_utf8_lossy(&response)
        );
        let read =
            tokio::time::timeout(std::time::Duration::from_secs(1), stream.read(&mut buffer))
                .await
                .expect("read response")
                .expect("read bytes");
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8_lossy(&response).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn gateway_without_seam_does_not_hijack_upgrade_requests() {
        let (server, addr) = start(false).await;
        let response = raw_upgrade(addr, "/ws").await;
        let _ = server.shutdown().await;
        assert!(
            !response.starts_with("HTTP/1.1 101"),
            "unset seam must not return 101: {response}"
        );
        assert!(!response.contains("Sec-WebSocket-Accept"));
    }

    #[tokio::test]
    async fn gateway_with_seam_routes_matching_upgrade_to_handler() {
        let (server, addr) = start(true).await;
        let response = raw_upgrade(addr, "/ws").await;
        let _ = server.shutdown().await;
        assert!(
            response.starts_with("HTTP/1.1 101"),
            "matching upgrade must be handed to the handler: {response}"
        );
        assert!(
            response.contains("x-upgrade-handler: called"),
            "handler response must be returned: {response}"
        );
    }

    #[tokio::test]
    async fn gateway_with_seam_keeps_http_behavior_for_other_paths() {
        let (server, addr) = start(true).await;
        let response = raw_upgrade(addr, "/not-ws").await;
        let _ = server.shutdown().await;
        assert!(
            !response.contains("x-upgrade-handler: called"),
            "non-matching path must stay on the HTTP path: {response}"
        );
    }
}

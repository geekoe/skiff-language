//! Real-socket probes for the C-net final listener mechanism.
//!
//! These probes exercise the mechanism frozen in
//! `doc/implementation/router-rust-migration-c-net-contract.md` over real local
//! sockets (127.0.0.1:0). They implement no business protocol and no control
//! endpoint: the probe service only answers empty HTTP requests, performs an
//! empty HTTP→WebSocket upgrade with a single echo, honors a connection cap,
//! and drains on shutdown.

#[cfg(test)]
mod tests {
    use std::io;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use futures_util::{SinkExt, StreamExt};
    use http_body_util::Full;
    use hyper::body::Incoming;
    use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{watch, Notify, Semaphore};
    use tokio::task::JoinSet;
    use tokio::time::timeout;
    use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
    use tokio_tungstenite::tungstenite::protocol::Role;
    use tokio_tungstenite::WebSocketStream;

    const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);
    const DEFAULT_DRAIN_DEADLINE: Duration = Duration::from_secs(2);

    struct ProbeServer {
        addr: SocketAddr,
        shutdown_tx: watch::Sender<()>,
        server_task: tokio::task::JoinHandle<()>,
    }

    impl ProbeServer {
        async fn start(
            limit: usize,
            hold: Option<Arc<Notify>>,
            drain_deadline: Duration,
        ) -> io::Result<Self> {
            let listener = TcpListener::bind("127.0.0.1:0").await?;
            let addr = listener.local_addr()?;
            let semaphore = Arc::new(Semaphore::new(limit));
            let (shutdown_tx, shutdown_rx) = watch::channel(());
            let server_task = tokio::spawn(run_server(
                listener,
                semaphore,
                shutdown_rx,
                hold,
                drain_deadline,
            ));
            Ok(Self {
                addr,
                shutdown_tx,
                server_task,
            })
        }

        fn trigger_shutdown(&self) {
            let _ = self.shutdown_tx.send(());
        }

        async fn shutdown(self) {
            self.trigger_shutdown();
            let joined = timeout(CLIENT_TIMEOUT, self.server_task)
                .await
                .expect("server did not exit within the client timeout");
            joined.expect("server task panicked");
        }
    }

    async fn run_server(
        listener: TcpListener,
        semaphore: Arc<Semaphore>,
        shutdown_rx: watch::Receiver<()>,
        hold: Option<Arc<Notify>>,
        drain_deadline: Duration,
    ) {
        let builder = http1::Builder::new();
        let mut connections = JoinSet::new();
        let mut shutdown_rx = shutdown_rx;

        loop {
            tokio::select! {
                changed = shutdown_rx.changed() => {
                    let _ = changed;
                    break;
                }
                accepted = listener.accept() => {
                    let (stream, _peer) = match accepted {
                        Ok(accepted) => accepted,
                        Err(_) => continue,
                    };
                    let permit = match Arc::clone(&semaphore).try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            reject_over_capacity(stream).await;
                            continue;
                        }
                    };
                    let service_hold = hold.clone();
                    let service = service_fn(move |request: Request<Incoming>| {
                        let hold = service_hold.clone();
                        async move { handle_request(request, hold).await }
                    });
                    let connection = builder
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades();
                    let mut connection_shutdown = shutdown_rx.clone();
                    connections.spawn(async move {
                        let _permit = permit;
                        tokio::pin!(connection);
                        tokio::select! {
                            result = connection.as_mut() => {
                                let _ = result;
                            }
                            _ = connection_shutdown.changed() => {
                                connection.as_mut().graceful_shutdown();
                                let _ = connection.await;
                            }
                        }
                    });
                }
            }
        }

        // Drain in-flight HTTP connections, then abort stragglers so shutdown
        // is bounded. Upgraded WebSocket tasks are detached from hyper and are
        // tracked separately by the supervisor (see contract §5).
        let _ = timeout(drain_deadline, async {
            while connections.join_next().await.is_some() {}
        })
        .await;
        connections.shutdown().await;
    }

    async fn reject_over_capacity(mut stream: TcpStream) {
        let response =
            b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let _ = stream.write_all(response).await;
        let _ = stream.shutdown().await;
    }

    async fn handle_request(
        mut request: Request<Incoming>,
        hold: Option<Arc<Notify>>,
    ) -> Result<Response<Full<Bytes>>, io::Error> {
        if is_websocket_upgrade(&request) {
            return handle_websocket_upgrade(&mut request).await;
        }
        if let Some(hold) = hold {
            hold.notified().await;
        }
        Ok(Response::new(Full::new(Bytes::new())))
    }

    fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
        request
            .headers()
            .get(UPGRADE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
    }

    async fn handle_websocket_upgrade(
        request: &mut Request<Incoming>,
    ) -> Result<Response<Full<Bytes>>, io::Error> {
        let Some(key) = request
            .headers()
            .get(SEC_WEBSOCKET_KEY)
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::new(Bytes::new()))
                .expect("static response is valid"));
        };
        let accept = derive_accept_key(key.as_bytes());
        let upgrade = hyper::upgrade::on(request);
        tokio::spawn(async move {
            let upgraded = match upgrade.await {
                Ok(upgraded) => upgraded,
                Err(_) => return,
            };
            let mut socket =
                WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
            if let Some(Ok(message)) = socket.next().await {
                let _ = socket.send(message).await;
                let _ = socket.close(None).await;
            }
        });
        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "upgrade")
            .header(SEC_WEBSOCKET_ACCEPT, accept)
            .body(Full::new(Bytes::new()))
            .expect("static response is valid"))
    }

    async fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
        timeout(CLIENT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))?
    }

    async fn send_empty_request(addr: SocketAddr) -> io::Result<Vec<u8>> {
        let mut stream = connect(addr).await?;
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: probe\r\nconnection: close\r\n\r\n")
            .await?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        Ok(response)
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

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_http_request_and_response_over_real_socket() {
        let server = ProbeServer::start(4, None, DEFAULT_DRAIN_DEADLINE)
            .await
            .expect("start probe server");
        let response = send_empty_request(server.addr)
            .await
            .expect("empty HTTP round trip");
        assert_empty_http_response(&response);
        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_websocket_upgrade_over_real_socket() {
        let server = ProbeServer::start(4, None, DEFAULT_DRAIN_DEADLINE)
            .await
            .expect("start probe server");
        let url = format!("ws://127.0.0.1:{}/", server.addr.port());
        let (mut socket, response) = timeout(CLIENT_TIMEOUT, tokio_tungstenite::connect_async(url))
            .await
            .expect("WebSocket handshake timed out")
            .expect("WebSocket handshake failed");
        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert!(
            response.headers().contains_key(SEC_WEBSOCKET_ACCEPT),
            "101 response must carry Sec-WebSocket-Accept"
        );
        let empty = tokio_tungstenite::tungstenite::Message::text(String::new());
        socket.send(empty.clone()).await.expect("send empty frame");
        let echoed = timeout(CLIENT_TIMEOUT, socket.next())
            .await
            .expect("echo timed out")
            .expect("echo stream ended")
            .expect("echo frame errored");
        assert_eq!(echoed, empty);
        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connection_limit_rejects_overflow_and_releases() {
        let server = ProbeServer::start(1, None, DEFAULT_DRAIN_DEADLINE)
            .await
            .expect("start probe server");

        let mut first = connect(server.addr).await.expect("first connection");
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

        let mut second = connect(server.addr).await.expect("second connection");
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
            if let Ok(response) = send_empty_request(server.addr).await {
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
        server.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_shutdown_drains_in_flight_requests() {
        let hold = Arc::new(Notify::new());
        let server = ProbeServer::start(4, Some(Arc::clone(&hold)), DEFAULT_DRAIN_DEADLINE)
            .await
            .expect("start probe server");

        let mut stream = connect(server.addr).await.expect("connect");
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: probe\r\n\r\n")
            .await
            .expect("send held request");
        let mut buf = [0u8; 64];
        assert!(
            timeout(Duration::from_millis(200), stream.read(&mut buf))
                .await
                .is_err(),
            "request must still be held before shutdown"
        );

        server.trigger_shutdown();
        assert!(
            timeout(Duration::from_millis(200), stream.read(&mut buf))
                .await
                .is_err(),
            "in-flight connection must stay open while draining"
        );

        hold.notify_waiters();
        let mut response = Vec::new();
        timeout(CLIENT_TIMEOUT, stream.read_to_end(&mut response))
            .await
            .expect("drained response timed out")
            .expect("drained response read failed");
        assert_empty_http_response(&response);

        timeout(CLIENT_TIMEOUT, server.server_task)
            .await
            .expect("server did not exit after draining")
            .expect("server task panicked");
        assert!(
            connect(server.addr).await.is_err(),
            "listener must stop accepting after shutdown"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_shutdown_aborts_stragglers_after_deadline() {
        let hold = Arc::new(Notify::new());
        let server = ProbeServer::start(4, Some(Arc::clone(&hold)), Duration::from_millis(300))
            .await
            .expect("start probe server");

        let mut stream = connect(server.addr).await.expect("connect");
        stream
            .write_all(b"GET / HTTP/1.1\r\nhost: probe\r\n\r\n")
            .await
            .expect("send held request");
        let mut buf = [0u8; 64];
        assert!(
            timeout(Duration::from_millis(200), stream.read(&mut buf))
                .await
                .is_err(),
            "request must still be held before shutdown"
        );

        server.trigger_shutdown();
        // Never release the hold: the drain deadline must force-close the
        // straggler so the server exits.
        let closed = timeout(Duration::from_secs(2), stream.read_to_end(&mut Vec::new()))
            .await
            .expect("straggler close timed out")
            .expect("straggler read failed");
        assert_eq!(closed, 0, "straggler must observe EOF after the deadline");
        timeout(CLIENT_TIMEOUT, server.server_task)
            .await
            .expect("server did not exit after aborting stragglers")
            .expect("server task panicked");
    }
}

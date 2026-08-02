//! Final listener skeleton (PR 0b), assembled from the C-net frozen mechanism.
//!
//! Public HTTP and the shared runtime/control socket are bound with the
//! frozen stack (Tokio multi-thread, hyper 1 with upgrades, tokio-tungstenite
//! 0.26, Semaphore caps, watch + drain + deadline abort). Only mechanism is
//! assembled here: empty HTTP responses, a health placeholder and an empty
//! `/runtime` WebSocket upgrade. No request dispatch, WS broker, activation
//! transaction or runtime registration business exists in this PR.

use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tokio::task::{AbortHandle, JoinError, JoinSet};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::WebSocketStream;

use crate::config::RouterConfig;

/// Public listener connection cap. The frozen Router config has no public
/// connection-limit field; this placeholder keeps the C-net Semaphore
/// mechanism active on every listener until the C-client-lifecycle/C-ws lanes
/// freeze the final public socket capacity semantics.
pub const DEFAULT_PUBLIC_MAX_CONNECTIONS: usize = 1024;

/// Drain deadline for in-flight HTTP connections before stragglers are
/// aborted. The full shutdown order of C-process-lifecycle is a later lane;
/// this is the C-net socket-layer bound.
pub const DEFAULT_DRAIN_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub enum ListenerError {
    Io(std::io::Error),
    Resolve(String),
    Join(String),
}

impl fmt::Display for ListenerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenerError::Io(error) => write!(formatter, "listener io error: {error}"),
            ListenerError::Resolve(message) => {
                write!(formatter, "listener address error: {message}")
            }
            ListenerError::Join(message) => write!(formatter, "listener join error: {message}"),
        }
    }
}

impl std::error::Error for ListenerError {}

/// Per-listener start overrides. Tests bind `127.0.0.1:0` and read the actual
/// address from `ListenerHandle::addr`.
#[derive(Debug, Clone)]
pub struct ListenerStartOptions {
    pub public_bind: Option<SocketAddr>,
    pub runtime_control_bind: Option<SocketAddr>,
    pub drain_deadline: Duration,
}

impl Default for ListenerStartOptions {
    fn default() -> Self {
        Self {
            public_bind: None,
            runtime_control_bind: None,
            drain_deadline: DEFAULT_DRAIN_DEADLINE,
        }
    }
}

/// A running listener: actual bound address, shutdown trigger and joined task.
pub struct ListenerHandle {
    name: &'static str,
    addr: SocketAddr,
    shutdown_tx: watch::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

impl ListenerHandle {
    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn shutdown(self) -> Result<(), JoinError> {
        let _ = self.shutdown_tx.send(());
        self.task.await
    }
}

/// The three logical listener services assembled by PR 0b: public HTTP plus
/// the shared runtime/control socket (control HTTP and `/runtime` WS).
pub struct RouterListeners {
    pub public: ListenerHandle,
    pub runtime_control: ListenerHandle,
}

impl RouterListeners {
    pub async fn shutdown(self) -> Result<(), ListenerError> {
        let (public, runtime_control) =
            tokio::join!(self.public.shutdown(), self.runtime_control.shutdown(),);
        match (public, runtime_control) {
            (Ok(()), Ok(())) => Ok(()),
            (public, runtime_control) => {
                let mut messages = Vec::new();
                if let Err(error) = public {
                    messages.push(format!("public: {error}"));
                }
                if let Err(error) = runtime_control {
                    messages.push(format!("runtime-control: {error}"));
                }
                Err(ListenerError::Join(messages.join("; ")))
            }
        }
    }
}

#[derive(Debug, Clone)]
enum ListenerKind {
    Public,
    RuntimeControl { runtime_path: String },
}

/// Starts the public and runtime/control listeners from a validated
/// `RouterConfig`. This is the only listener construction point in PR 0b.
pub async fn start_listeners(
    config: &RouterConfig,
    options: &ListenerStartOptions,
) -> Result<RouterListeners, ListenerError> {
    let public_addr = match options.public_bind {
        Some(addr) => addr,
        None => resolve_bind_addr(&config.host, config.http_port)?,
    };
    let runtime_control_addr = match options.runtime_control_bind {
        Some(addr) => addr,
        None => resolve_bind_addr(&config.host, config.runtime_port)?,
    };

    let public_listener = TcpListener::bind(public_addr)
        .await
        .map_err(ListenerError::Io)?;
    let public_addr = public_listener.local_addr().map_err(ListenerError::Io)?;
    let runtime_control_listener = TcpListener::bind(runtime_control_addr)
        .await
        .map_err(ListenerError::Io)?;
    let runtime_control_addr = runtime_control_listener
        .local_addr()
        .map_err(ListenerError::Io)?;

    let (public_shutdown_tx, public_shutdown_rx) = watch::channel(());
    let (runtime_control_shutdown_tx, runtime_control_shutdown_rx) = watch::channel(());

    let public = ListenerHandle {
        name: "public",
        addr: public_addr,
        shutdown_tx: public_shutdown_tx,
        task: tokio::spawn(serve_listener(
            public_listener,
            Arc::new(Semaphore::new(DEFAULT_PUBLIC_MAX_CONNECTIONS)),
            ListenerKind::Public,
            public_shutdown_rx,
            options.drain_deadline,
        )),
    };
    let runtime_control = ListenerHandle {
        name: "runtime-control",
        addr: runtime_control_addr,
        shutdown_tx: runtime_control_shutdown_tx,
        task: tokio::spawn(serve_listener(
            runtime_control_listener,
            Arc::new(Semaphore::new(
                usize::try_from(config.runtime_max_concurrency).unwrap_or(usize::MAX),
            )),
            ListenerKind::RuntimeControl {
                runtime_path: config.runtime_path.clone(),
            },
            runtime_control_shutdown_rx,
            options.drain_deadline,
        )),
    };

    Ok(RouterListeners {
        public,
        runtime_control,
    })
}

/// Runs the listeners until SIGINT/SIGTERM, then shuts them down gracefully.
pub async fn run_router(config: RouterConfig) -> Result<(), ListenerError> {
    let listeners = start_listeners(&config, &ListenerStartOptions::default()).await?;
    wait_for_shutdown_signal().await;
    listeners.shutdown().await
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT handler");
        tokio::select! {
            _ = sigterm.recv() => {}
            _ = sigint.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn resolve_bind_addr(host: &str, port: u16) -> Result<SocketAddr, ListenerError> {
    (host, port)
        .to_socket_addrs()
        .map_err(|error| ListenerError::Resolve(format!("{host}:{port}: {error}")))?
        .next()
        .ok_or_else(|| ListenerError::Resolve(format!("{host}:{port} resolved to no address")))
}

async fn serve_listener(
    listener: TcpListener,
    semaphore: Arc<Semaphore>,
    kind: ListenerKind,
    mut shutdown_rx: watch::Receiver<()>,
    drain_deadline: Duration,
) {
    let mut connections = JoinSet::new();
    let websocket_registry = Arc::new(Mutex::new(Vec::<AbortHandle>::new()));

    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
            accepted = listener.accept() => {
                let (stream, _peer) = match accepted {
                    Ok(accepted) => accepted,
                    Err(_) => continue,
                };
                let permit = match semaphore.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        reject_over_capacity(stream).await;
                        continue;
                    }
                };
                let kind = kind.clone();
                let mut connection_shutdown = shutdown_rx.clone();
                let websocket_registry = Arc::clone(&websocket_registry);
                connections.spawn(async move {
                    let _permit = permit;
                    let service_shutdown = connection_shutdown.clone();
                    let service = service_fn(move |request: Request<Incoming>| {
                        let kind = kind.clone();
                        let connection_shutdown = service_shutdown.clone();
                        let websocket_registry = Arc::clone(&websocket_registry);
                        async move {
                            handle_request(&kind, &connection_shutdown, &websocket_registry, request)
                                .await
                        }
                    });
                    let connection = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .with_upgrades();
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

    // Stop accepting, drain in-flight HTTP connections, then abort stragglers
    // (including upgraded WebSocket tasks detached from hyper).
    let _ = timeout(drain_deadline, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    connections.shutdown().await;
    let websocket_tasks = websocket_registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .drain(..)
        .collect::<Vec<_>>();
    for handle in websocket_tasks {
        handle.abort();
    }
}

type RouterResponse = Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>;

async fn handle_request(
    kind: &ListenerKind,
    shutdown_rx: &watch::Receiver<()>,
    websocket_registry: &Arc<Mutex<Vec<AbortHandle>>>,
    request: Request<Incoming>,
) -> RouterResponse {
    match kind {
        ListenerKind::Public => Ok(empty_response(StatusCode::OK)),
        ListenerKind::RuntimeControl { runtime_path } => {
            if is_websocket_upgrade(&request) && request.uri().path() == runtime_path {
                return handle_websocket_upgrade(request, shutdown_rx, websocket_registry).await;
            }
            Ok(empty_response(StatusCode::OK))
        }
    }
}

fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
    request
        .headers()
        .get(UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

async fn handle_websocket_upgrade(
    mut request: Request<Incoming>,
    shutdown_rx: &watch::Receiver<()>,
    websocket_registry: &Arc<Mutex<Vec<AbortHandle>>>,
) -> RouterResponse {
    let Some(key) = request
        .headers()
        .get(SEC_WEBSOCKET_KEY)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(empty_response(StatusCode::BAD_REQUEST));
    };
    let accept = derive_accept_key(key.as_bytes());
    let upgrade = hyper::upgrade::on(&mut request);
    let shutdown_rx = shutdown_rx.clone();
    let handle = tokio::spawn(async move {
        let upgraded = match upgrade.await {
            Ok(upgraded) => upgraded,
            Err(_) => return,
        };
        let mut socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        let mut shutdown_rx = shutdown_rx;
        tokio::select! {
            _ = shutdown_rx.changed() => {
                let _ = socket.close(None).await;
            }
            _ = drain_websocket(&mut socket) => {}
        }
    });
    websocket_registry
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(handle.abort_handle());
    Ok(Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(UPGRADE, "websocket")
        .header(CONNECTION, "upgrade")
        .header(SEC_WEBSOCKET_ACCEPT, accept)
        .body(empty_body())
        .expect("static upgrade response is valid"))
}

/// Reads frames until the client closes or errors; no business protocol is
/// interpreted in the PR 0b skeleton.
async fn drain_websocket(socket: &mut WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>) {
    while let Some(frame) = socket.next().await {
        match frame {
            Ok(_) => {}
            Err(_) => return,
        }
    }
}

fn empty_response(status: StatusCode) -> Response<BoxBody<Bytes, hyper::Error>> {
    Response::builder()
        .status(status)
        .body(empty_body())
        .expect("static empty response is valid")
}

fn empty_body() -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

async fn reject_over_capacity(mut stream: TcpStream) {
    let response =
        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    let _ = stream.write_all(response).await;
    let _ = stream.shutdown().await;
}

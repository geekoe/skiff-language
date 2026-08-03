//! Final listener skeleton (PR 0b), assembled from the C-net frozen mechanism
//! with the W-session `/runtime` WebSocket assembly.
//!
//! Public HTTP and the shared runtime/control socket are bound with the
//! frozen stack (Tokio multi-thread, hyper 1 with upgrades, tokio-tungstenite
//! 0.26, Semaphore caps, watch + drain + deadline abort). Only mechanism is
//! assembled here: empty HTTP responses, a health placeholder and the
//! `/runtime` WebSocket upgrade handed to `SessionLayer` (W-session). No
//! request dispatch, WS broker or activation transaction business exists.
//!
//! `run_router` additionally owns the E-bootstrap wiring: the committed epoch
//! must be assembled and published before any listener is bound, and the
//! session layer receives the epoch store as its epoch source (plan §7
//! E-bootstrap, C-bootstrap §2.5 readiness).

use std::fmt;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::header::{CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, UPGRADE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::{AbortHandle, JoinError, JoinSet};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Message, Role};
use tokio_tungstenite::WebSocketStream;

use crate::activation::http::ActivationHttpHandler;
use crate::activation::ASSEMBLY_ACTIVATION_CONTROL_PATH;
use crate::bootstrap::ActiveRoutingEpochStore;
use crate::config::RouterConfig;
use crate::health::HealthAggregator;
use crate::http::selector::{
    parse_request_target, parse_service_deployment_selector, RequestTarget,
};
use crate::http::GatewayUpgradeHandler;
use crate::session::identity::RuntimeSessionEpoch;
use crate::session::SessionLayer;
use crate::supervisor::ws::{
    ConnectOutcome, WsConnectMetadata, WsConnectSelector, WsConnectionRecord, WsDispatchStore,
    WsGatewaySurfaceView,
};
use crate::test_dispatch::{TestDispatchHttpHandler, TEST_DISPATCH_CONTROL_PATH};
use crate::ws::{AttachMeta, BusinessKey, ClientTerminal, PeerWriter, WebSocketLane};
use skiff_artifact_model::AssemblyIdentity;
use skiff_runtime_transport::connection_protocol::WebSocketRpcProfile;
use skiff_runtime_transport::websocket_generation_lifecycle::WebSocketGenerationLifecycleTuple;

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
    FailStop(String),
    Http(String),
}

impl fmt::Display for ListenerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListenerError::Io(error) => write!(formatter, "listener io error: {error}"),
            ListenerError::Resolve(message) => {
                write!(formatter, "listener address error: {message}")
            }
            ListenerError::Join(message) => write!(formatter, "listener join error: {message}"),
            ListenerError::FailStop(message) => write!(formatter, "listener fail-stop: {message}"),
            ListenerError::Http(message) => write!(formatter, "http gateway error: {message}"),
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

    /// Stops accepting new connections without joining the task (used by the
    /// supervisor shutdown order: stop accept, drain the session barrier,
    /// then join).
    pub fn begin_shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }

    /// Joins the listener task after `begin_shutdown`.
    pub async fn join_shutdown(self) -> Result<(), JoinError> {
        self.task.await
    }
}

/// The three logical listener services assembled by PR 0b: public HTTP plus
/// the shared runtime/control socket (control HTTP and `/runtime` WS).
pub struct RouterListeners {
    pub public: ListenerHandle,
    pub runtime_control: ListenerHandle,
    pub session: Arc<SessionLayer>,
}

impl RouterListeners {
    pub async fn shutdown(self) -> Result<(), ListenerError> {
        let RouterListeners {
            public,
            runtime_control,
            session,
        } = self;
        // C-process-lifecycle: S1 stop accepting, then S6 close Runtime
        // sessions via the barrier, then join listener tasks.
        let _ = public.shutdown_tx.send(());
        let _ = runtime_control.shutdown_tx.send(());
        let session_result = session.shutdown().await;
        let (public, runtime_control) = tokio::join!(public.task, runtime_control.task);
        let mut errors = Vec::new();
        let mut session_failed = false;
        if let Err(error) = session_result {
            errors.push(format!("session: {error}"));
            session_failed = true;
        }
        if let Err(error) = public {
            errors.push(format!("public: {error}"));
        }
        if let Err(error) = runtime_control {
            errors.push(format!("runtime-control: {error}"));
        }
        if errors.is_empty() {
            return Ok(());
        }
        let message = errors.join("; ");
        if session_failed {
            return Err(ListenerError::FailStop(message));
        }
        Err(ListenerError::Join(message))
    }
}

#[derive(Debug, Clone)]
enum ListenerKind {
    Public,
    RuntimeControl {
        runtime_path: String,
        session_layer: Arc<SessionLayer>,
        activation_http: Option<Arc<ActivationHttpHandler>>,
        health: Option<Arc<HealthAggregator>>,
        test_dispatch: Option<Arc<TestDispatchHttpHandler>>,
    },
}

/// Starts the public and runtime/control listeners from a validated
/// `RouterConfig`. This is the only listener construction point in PR 0b.
pub async fn start_listeners(
    config: &RouterConfig,
    options: &ListenerStartOptions,
) -> Result<RouterListeners, ListenerError> {
    start_listeners_with_session(config, options, Arc::new(SessionLayer::new(config.clone()))).await
}

/// Starts only the shared runtime/control listener (supervisor composition
/// runs the public HTTP gateway separately through `start_http_gateway`).
pub async fn start_runtime_control_listener(
    config: &RouterConfig,
    options: &ListenerStartOptions,
    session_layer: Arc<SessionLayer>,
) -> Result<ListenerHandle, ListenerError> {
    start_runtime_control_listener_with_control(config, options, session_layer, None).await
}

/// Starts the shared runtime/control listener with the activation control
/// HTTP handler (E-activation: `POST /__skiff/activate-assembly`). The
/// handler is optional so the legacy listener seam (and tests that call the
/// 3-argument form) keep the previous empty-response behavior byte for byte.
pub async fn start_runtime_control_listener_with_control(
    config: &RouterConfig,
    options: &ListenerStartOptions,
    session_layer: Arc<SessionLayer>,
    activation_http: Option<Arc<ActivationHttpHandler>>,
) -> Result<ListenerHandle, ListenerError> {
    start_runtime_control_listener_with_control_and_health(
        config,
        options,
        session_layer,
        activation_http,
        None,
    )
    .await
}

/// Starts the shared runtime/control listener with the activation control
/// HTTP handler and the health projection aggregator (batch 12: the only
/// production wiring of `/__router/health`).
pub async fn start_runtime_control_listener_with_control_and_health(
    config: &RouterConfig,
    options: &ListenerStartOptions,
    session_layer: Arc<SessionLayer>,
    activation_http: Option<Arc<ActivationHttpHandler>>,
    health: Option<Arc<HealthAggregator>>,
) -> Result<ListenerHandle, ListenerError> {
    start_runtime_control_listener_with_control_and_health_and_test_dispatch(
        config,
        options,
        session_layer,
        activation_http,
        health,
        None,
    )
    .await
}

/// Starts the shared runtime/control listener with the activation control
/// HTTP handler, the health projection aggregator and the test-dispatch
/// control handler (plan §7 E-http: `POST /__skiff/test-dispatch`).
pub async fn start_runtime_control_listener_with_control_and_health_and_test_dispatch(
    config: &RouterConfig,
    options: &ListenerStartOptions,
    session_layer: Arc<SessionLayer>,
    activation_http: Option<Arc<ActivationHttpHandler>>,
    health: Option<Arc<HealthAggregator>>,
    test_dispatch: Option<Arc<TestDispatchHttpHandler>>,
) -> Result<ListenerHandle, ListenerError> {
    let runtime_control_addr = match options.runtime_control_bind {
        Some(addr) => addr,
        None => resolve_listener_addr(&config.host, config.runtime_port)?,
    };
    let runtime_control_listener = TcpListener::bind(runtime_control_addr)
        .await
        .map_err(ListenerError::Io)?;
    let runtime_control_addr = runtime_control_listener
        .local_addr()
        .map_err(ListenerError::Io)?;
    let (runtime_control_shutdown_tx, runtime_control_shutdown_rx) = watch::channel(());
    Ok(ListenerHandle {
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
                session_layer: Arc::clone(&session_layer),
                activation_http,
                health,
                test_dispatch,
            },
            runtime_control_shutdown_rx,
            options.drain_deadline,
        )),
    })
}

/// Listener construction with an explicitly assembled session layer (tests
/// inject the corpus committed epoch, timing and fake consumer manifests).
pub async fn start_listeners_with_session(
    config: &RouterConfig,
    options: &ListenerStartOptions,
    session_layer: Arc<SessionLayer>,
) -> Result<RouterListeners, ListenerError> {
    let public_addr = match options.public_bind {
        Some(addr) => addr,
        None => resolve_listener_addr(&config.host, config.http_port)?,
    };

    let public_listener = TcpListener::bind(public_addr)
        .await
        .map_err(ListenerError::Io)?;
    let public_addr = public_listener.local_addr().map_err(ListenerError::Io)?;

    let (public_shutdown_tx, public_shutdown_rx) = watch::channel(());

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
    let runtime_control =
        start_runtime_control_listener(config, options, Arc::clone(&session_layer)).await?;

    Ok(RouterListeners {
        public,
        runtime_control,
        session: session_layer,
    })
}

/// Runs the listeners until SIGINT/SIGTERM, then shuts them down gracefully.
pub async fn run_router(config: RouterConfig) -> Result<(), ListenerError> {
    // W-composition: the supervisor owns the full production assembly
    // (bootstrap epoch, dispatcher/admission, WS lane, activation coordinator,
    // actor lane, HTTP gateway) and the lifecycle.
    let supervisor = crate::supervisor::RouterSupervisor::assemble(&config)
        .await
        .map_err(|error| {
            let message = match &error {
                crate::supervisor::SupervisorError::EnvironmentMissing
                | crate::supervisor::SupervisorError::Bootstrap(_) => {
                    format!("bootstrap failed closed: {error}")
                }
                _ => format!("router composition failed: {error}"),
            };
            ListenerError::FailStop(message)
        })?;
    let listeners = supervisor
        .start_listeners(&ListenerStartOptions::default())
        .await?;
    let mut fail_stop_rx = listeners.session.fail_stop_subscribe();
    tokio::select! {
        _ = wait_for_shutdown_signal() => {}
        _ = fail_stop_rx.changed() => {}
    }
    let shutdown_result = listeners.shutdown().await;
    supervisor.shutdown().await;
    shutdown_result
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

/// Resolves the bind address for one configured host/port pair (public
/// composition seam).
pub fn resolve_listener_addr(host: &str, port: u16) -> Result<SocketAddr, ListenerError> {
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
                    let service = service_fn(move |request: Request<Incoming>| {
                        let kind = kind.clone();
                        let websocket_registry = Arc::clone(&websocket_registry);
                        async move {
                            handle_request(&kind, &websocket_registry, request).await
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
    websocket_registry: &Arc<Mutex<Vec<AbortHandle>>>,
    request: Request<Incoming>,
) -> RouterResponse {
    match kind {
        ListenerKind::Public => Ok(empty_response(StatusCode::OK)),
        ListenerKind::RuntimeControl {
            runtime_path,
            session_layer,
            activation_http,
            health,
            test_dispatch,
        } => {
            if is_websocket_upgrade(&request) && request.uri().path() == runtime_path {
                return handle_websocket_upgrade(
                    request,
                    websocket_registry,
                    Arc::clone(session_layer),
                )
                .await;
            }
            if request.uri().path() == "/__router/health" {
                if let Some(health) = health {
                    return handle_health_request(Arc::clone(health), request).await;
                }
                return Ok(empty_response(StatusCode::OK));
            }
            if request.uri().path() == ASSEMBLY_ACTIVATION_CONTROL_PATH {
                if let Some(handler) = activation_http {
                    return handler.handle(request).await;
                }
            }
            if request.uri().path() == TEST_DISPATCH_CONTROL_PATH {
                if let Some(handler) = test_dispatch {
                    return handler.handle(request).await;
                }
            }
            Ok(empty_response(StatusCode::OK))
        }
    }
}

/// `/__router/health` production route (batch 12 health leaf; TS
/// `AssemblyControlPlane` parity: GET-only, `?detail=loop-risk` adds the
/// loopRisk object).
async fn handle_health_request(
    health: Arc<HealthAggregator>,
    request: Request<Incoming>,
) -> RouterResponse {
    if request.method() != Method::GET {
        // Drain the request body before responding so hyper closes the
        // connection with FIN, not RST (mirrors the public gateway
        // early-error path; bounded to the control body cap).
        drain_request_body(request.into_body(), HEALTH_REQUEST_BODY_DRAIN_CAP).await;
        let body = serde_json::json!({
            "error": {
                "code": "MethodNotAllowed",
                "message": "router health requires GET",
            }
        })
        .to_string();
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header("content-type", "application/json")
            .header("allow", "GET")
            .body(full_body(body))
            .expect("static health 405 response is valid"));
    }
    let with_loop_risk = request
        .uri()
        .query()
        .is_some_and(|query| query.split('&').any(|part| part == "detail=loop-risk"));
    let body = health.render(with_loop_risk).await.to_string();
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .body(full_body(body))
        .expect("health response is valid"))
}

/// Control-path body drain cap for early-error responses (1 MiB, same bound
/// as the activation control endpoint).
const HEALTH_REQUEST_BODY_DRAIN_CAP: usize = 1024 * 1024;

async fn drain_request_body(mut body: Incoming, limit: usize) {
    let mut total = 0usize;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            break;
        };
        if let Some(data) = frame.data_ref() {
            total += data.len();
            if total > limit {
                break;
            }
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
    websocket_registry: &Arc<Mutex<Vec<AbortHandle>>>,
    session_layer: Arc<SessionLayer>,
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
    let handle = tokio::spawn(async move {
        let upgraded = match upgrade.await {
            Ok(upgraded) => upgraded,
            Err(_) => return,
        };
        let socket =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        session_layer.accept(socket);
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

fn full_body(body: String) -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::from(body))
        .map_err(|never| match never {})
        .boxed()
}

async fn reject_over_capacity(mut stream: TcpStream) {
    let response =
        b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
    let _ = stream.write_all(response).await;
    let _ = stream.shutdown().await;
}

// ---------------------------------------------------------------------------
// Client WebSocket accept path (E-ws wiring).
//
// The public HTTP gateway upgrade seam (`http::server`) hands matching
// `websocket_path` upgrade requests to [`ClientWsContext`]; this module owns
// the socket-level accept path: selector/ingress resolution, connect
// admission through the WS lane + composition store, the 101 handshake and
// the upgraded peer task. No WS lane internals are touched.
// ---------------------------------------------------------------------------

/// Tracks spawned client WS tasks so the supervisor can abort stragglers at
/// shutdown (C-net §5 upgraded-task tracking).
#[derive(Debug, Default)]
pub struct WsTaskRegistry {
    tasks: Mutex<Vec<AbortHandle>>,
}

impl WsTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn track(&self, handle: AbortHandle) {
        self.tasks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(handle);
    }

    pub fn abort_all(&self) {
        let tasks = std::mem::take(
            &mut *self
                .tasks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for handle in tasks {
            handle.abort();
        }
    }
}

/// Production client WebSocket accept context (E-ws).
#[derive(Debug, Clone)]
pub struct ClientWsContext {
    pub surface: Arc<WsGatewaySurfaceView>,
    pub lane: Arc<WebSocketLane>,
    pub store: Arc<WsDispatchStore>,
    pub selector: Arc<dyn WsConnectSelector>,
    pub epoch_store: Arc<ActiveRoutingEpochStore>,
    pub dispatcher: Arc<crate::dispatch::RequestDispatcher>,
    pub path: String,
    pub connect_timeout_ms: u64,
    pub tasks: Arc<WsTaskRegistry>,
    next_connection_id: Arc<AtomicU64>,
}

impl ClientWsContext {
    pub fn new(
        surface: Arc<WsGatewaySurfaceView>,
        lane: Arc<WebSocketLane>,
        store: Arc<WsDispatchStore>,
        selector: Arc<dyn WsConnectSelector>,
        epoch_store: Arc<ActiveRoutingEpochStore>,
        dispatcher: Arc<crate::dispatch::RequestDispatcher>,
        path: String,
        connect_timeout_ms: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            surface,
            lane,
            store,
            selector,
            epoch_store,
            dispatcher,
            path,
            connect_timeout_ms,
            tasks: Arc::new(WsTaskRegistry::new()),
            next_connection_id: Arc::new(AtomicU64::new(0)),
        })
    }

    fn new_connection_id(&self) -> String {
        format!(
            "wsconn-{}-{}",
            now_nanos(),
            self.next_connection_id.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn fail_reservation(&self, connection_id: &str) {
        self.selector.release(connection_id);
        let _ = self
            .lane
            .finish(connection_id, ClientTerminal::PolicyRejected, None);
    }
}

#[async_trait]
impl GatewayUpgradeHandler for ClientWsContext {
    async fn handle(&self, mut request: Request<Incoming>) -> RouterResponse {
        let Some(key) = request
            .headers()
            .get(SEC_WEBSOCKET_KEY)
            .and_then(|value| value.to_str().ok())
        else {
            return Ok(empty_response(StatusCode::BAD_REQUEST));
        };
        let accept = derive_accept_key(key.as_bytes());
        let selector = match parse_service_deployment_selector(request.headers()) {
            Ok(selector) => selector,
            Err(_) => return Ok(empty_response(StatusCode::BAD_REQUEST)),
        };
        let target = match parse_request_target(request.headers(), request.uri()) {
            Ok(target) => target,
            Err(_) => return Ok(empty_response(StatusCode::BAD_REQUEST)),
        };
        if target.path != self.path {
            return Ok(empty_response(StatusCode::NOT_FOUND));
        }
        let Some(binding) = self
            .surface
            .resolve(&selector.service_id, &target.path)
            .cloned()
        else {
            return Ok(empty_response(StatusCode::NOT_FOUND));
        };
        if !binding.connect_handler && binding.methods.is_empty() {
            // The WS lane attach requires an exact runtime; handler-less and
            // method-less bindings fail closed (composition decision, TS
            // `requiresRuntimePin == false` parity gap documented in the leaf).
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        }
        let Some(epoch) = self.epoch_store.capture() else {
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        };
        let assembly_identity = AssemblyIdentity::new(epoch.assembly_identity().to_string());
        let assembly_generation = epoch.assembly_generation();
        let connection_id = self.new_connection_id();
        if let Err(_) = self.lane.reserve(&connection_id) {
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        }
        let runtime = match self.selector.select(&connection_id, &binding) {
            Ok(runtime) => runtime,
            Err(_) => {
                self.fail_reservation(&connection_id);
                return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
            }
        };
        // Admission expectation. `router_session_id` is the router-side
        // session token; parity with the Runtime-minted
        // `skiff-router-session-v1:opaque:<uuid>` router session id is a
        // documented E-ws lane item (see leaf; fake-runtime tests use a
        // self-consistent tuple).
        let tuple = WebSocketGenerationLifecycleTuple {
            router_session_id: format!("{}#{}", runtime.replica_id, runtime.connection_generation),
            service_id: binding.service_id.clone(),
            assembly_identity: assembly_identity.clone(),
            assembly_generation,
            websocket_entry_id: binding.websocket_entry_id.clone(),
            connection_id: connection_id.clone(),
        };
        if let Err(_) = self.lane.ledger.expect_connection(tuple) {
            self.fail_reservation(&connection_id);
            return Ok(empty_response(StatusCode::INTERNAL_SERVER_ERROR));
        }
        let metadata = build_ws_connect_metadata(&target, request.headers());
        let (connect_request_id, mut connect_wait) = match self.store.connect_begin(
            &connection_id,
            &binding,
            &runtime,
            &assembly_identity,
            assembly_generation,
            &metadata,
            self.connect_timeout_ms,
        ) {
            Ok(parts) => parts,
            Err(_) => {
                self.fail_reservation(&connection_id);
                return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
            }
        };
        let spawn_parent_deadline = crate::dispatch::RequestDeadline {
            timeout_ms: self.connect_timeout_ms,
            expires_at: crate::supervisor::ws::format_iso8601_now_plus(self.connect_timeout_ms),
        };
        if let Err(_) = self.dispatcher.register_spawn_parent(
            connect_request_id.clone(),
            runtime.clone(),
            epoch.clone(),
            Some(spawn_parent_deadline),
        ) {
            // The admission cannot act as a spawn parent; fail closed so a
            // connect handler that spawns gets a deterministic error instead
            // of a mid-flight parent loss.
            self.store.connect_unavailable(
                &connect_request_id,
                "websocket connect spawn parent registration failed".to_string(),
            );
            self.fail_reservation(&connection_id);
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        }
        let outcome = tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(self.connect_timeout_ms + 1000)) => {
                self.store.connect_unavailable(
                    &connect_request_id,
                    "websocket connect timed out".to_string(),
                );
                None
            }
            changed = connect_wait.changed() => {
                let _ = changed;
                connect_wait.borrow_and_update().clone()
            }
        };
        self.selector.release(&connection_id);
        self.dispatcher.unregister_spawn_parent(&connect_request_id);
        let Some(outcome) = outcome else {
            let _ = self
                .lane
                .finish(&connection_id, ClientTerminal::ReleaseTimeout, None);
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        };
        let ConnectOutcome::Accepted {
            business_identity,
            admission_rank,
            max_connections,
            overflow,
            close_code: _,
            close_reason: _,
        } = outcome
        else {
            let _ = self
                .lane
                .finish(&connection_id, ClientTerminal::PolicyRejected, None);
            return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
        };
        let business_key = business_identity.as_deref().map(|identity| {
            BusinessKey::from_parts(&binding.service_id, &binding.websocket_entry_id, identity)
        });
        let admission = self.lane.admit(
            &connection_id,
            business_key,
            admission_rank,
            usize::try_from(max_connections).unwrap_or(usize::MAX),
            overflow,
        );
        let close_after_upgrade = match admission {
            crate::ws::AdmissionOutcome::Accepted => None,
            crate::ws::AdmissionOutcome::Rejected { close } => {
                let _ = self.lane.finish(
                    &connection_id,
                    ClientTerminal::PolicyRejected,
                    Some(close.clone()),
                );
                if admission_rank.is_some() {
                    // Ranked high-water rejection closes after upgrade
                    // (TS parity).
                    Some(close)
                } else {
                    return Ok(empty_response(StatusCode::SERVICE_UNAVAILABLE));
                }
            }
        };
        self.store.register_connection(WsConnectionRecord {
            connection_id: connection_id.clone(),
            runtime: runtime.clone(),
            binding: binding.clone(),
            business_identity: business_identity.clone(),
            assembly_identity,
            assembly_generation,
        });
        let upgrade = hyper::upgrade::on(&mut request);
        let context = self.clone();
        let handle = tokio::spawn(async move {
            let upgraded = match upgrade.await {
                Ok(upgraded) => upgraded,
                Err(_) => {
                    context.store.unregister_connection(&connection_id);
                    return;
                }
            };
            let socket =
                WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
            if let Some(close) = close_after_upgrade {
                let (mut write_half, _) = socket.split();
                let _ = write_half
                    .send(Message::Close(Some(CloseFrame {
                        code: CloseCode::Iana(close.code),
                        reason: close.reason.into(),
                    })))
                    .await;
                let _ = write_half.send(Message::Close(None)).await;
                context.store.unregister_connection(&connection_id);
                return;
            }
            run_client_ws_peer(context, connection_id, runtime, socket).await;
        });
        self.tasks.track(handle.abort_handle());
        Ok(Response::builder()
            .status(StatusCode::SWITCHING_PROTOCOLS)
            .header(UPGRADE, "websocket")
            .header(CONNECTION, "upgrade")
            .header(SEC_WEBSOCKET_ACCEPT, accept)
            .body(empty_body())
            .expect("static upgrade response is valid"))
    }
}

enum Outbound {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, String),
}

/// Real socket single-writer adapter: bounded queue + writer task; terminate
/// aborts the socket immediately (C-client-lifecycle §3.4).
#[derive(Debug)]
struct SocketPeerWriter {
    tx: mpsc::Sender<Outbound>,
    buffered: Arc<AtomicU64>,
    task: tokio::task::JoinHandle<()>,
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

    fn write_binary(&self, payload: Vec<u8>) -> Result<(), String> {
        let bytes = payload.len() as u64;
        self.tx
            .try_send(Outbound::Binary(payload))
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
    mut write_half: futures_util::stream::SplitSink<WebSocketStream<S>, Message>,
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
            Outbound::Binary(payload) => {
                let bytes = payload.len() as u64;
                let sent = write_half.send(Message::Binary(payload.into())).await;
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

async fn run_client_ws_peer(
    context: ClientWsContext,
    connection_id: String,
    runtime: RuntimeSessionEpoch,
    socket: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>,
) {
    let Some(record) = context.store.connection_record(&connection_id) else {
        return;
    };
    let (write_half, read_half) = socket.split();
    let (tx, rx) = mpsc::channel::<Outbound>(64);
    let buffered = Arc::new(AtomicU64::new(0));
    let writer_task = tokio::spawn(writer_loop(write_half, rx, buffered.clone()));
    let writer: Arc<dyn PeerWriter> = Arc::new(SocketPeerWriter {
        tx,
        buffered,
        task: writer_task,
    });
    if let Err(_) = context.lane.attach(
        &connection_id,
        1,
        connection_id.clone(),
        runtime,
        writer,
        AttachMeta {
            service_id: record.binding.service_id.clone(),
            websocket_entry_id: record.binding.websocket_entry_id.clone(),
            profile: WebSocketRpcProfile::JsonRpc2_0Text,
        },
    ) {
        context.store.unregister_connection(&connection_id);
        let _ = context
            .lane
            .finish(&connection_id, ClientTerminal::TransportError, None);
        return;
    }
    reader_loop(context, connection_id, read_half).await;
}

async fn reader_loop<S>(
    context: ClientWsContext,
    connection_id: String,
    mut read_half: futures_util::stream::SplitStream<WebSocketStream<S>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    while let Some(message) = read_half.next().await {
        match message {
            Ok(Message::Text(text)) => {
                let _ = context
                    .lane
                    .handle_peer_text(&connection_id, text.as_bytes());
            }
            Ok(Message::Binary(_)) => {
                let _ = context.lane.handle_peer_binary(&connection_id);
            }
            Ok(Message::Close(_)) | Err(_) => {
                let _ = context.lane.handle_peer_disconnect(&connection_id);
                break;
            }
            _ => {}
        }
    }
    context.store.unregister_connection(&connection_id);
}

fn build_ws_connect_metadata(
    target: &RequestTarget,
    headers: &hyper::HeaderMap,
) -> WsConnectMetadata {
    let query = target
        .query
        .iter()
        .map(|value| skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestNameValueFrameHeader {
            name: value.name.clone(),
            value: value.value.clone(),
        })
        .collect();
    let mut request_headers = Vec::new();
    let mut cookies = Vec::new();
    for (name, value) in headers {
        let name = name.as_str().to_string();
        let value = value.to_str().unwrap_or_default().to_string();
        if name.eq_ignore_ascii_case("cookie") {
            for segment in value.split(';') {
                let segment = segment.trim();
                if segment.is_empty() {
                    continue;
                }
                let (cookie_name, cookie_value) = match segment.split_once('=') {
                    Some((name, value)) => (name.to_string(), value.to_string()),
                    None => (segment.to_string(), String::new()),
                };
                cookies.push(
                    skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestNameValueFrameHeader {
                        name: cookie_name,
                        value: cookie_value,
                    },
                );
            }
        } else {
            request_headers.push(
                skiff_runtime_transport::runtime_assembly_request::RuntimeAssemblyRequestNameValueFrameHeader {
                    name,
                    value,
                },
            );
        }
    }
    WsConnectMetadata {
        url: target.url.replacen("http://", "ws://", 1),
        query,
        headers: request_headers,
        cookies,
    }
}

fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

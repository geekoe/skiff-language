//! Real-socket public HTTP gateway (C-net mechanism: hyper 1, Semaphore
//! connection cap, watch + drain deadline + abort). W-http delivers the real
//! HTTP → `HttpDispatchPort` boundary here; the production listener assembly
//! remains owned by E-bootstrap gate.

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::http::header::HeaderName;
use hyper::http::response::Builder as ResponseBuilder;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{HeaderMap, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::bootstrap::RoutingEpoch;

use super::cors;
use super::dispatch::{
    cancel_channel, CancelOnDrop, DispatchRequest, HttpDispatchError, HttpDispatchPort,
    PendingTerminalSource,
};
use super::error::HttpError;
use super::frame::{build_request_start_header, new_request_id};
use super::ingress::{HttpDispatchMode, HttpIngressResolver};
use super::selector::{
    build_http_request_metadata, has_test_case_correlation_headers, parse_request_target,
    parse_service_deployment_selector, parse_test_case_correlation,
};
use super::stream::{
    ChannelStreamBody, ChannelStreamSink, HttpStreamSink, DEFAULT_STREAM_CHANNEL_CAPACITY,
};
use super::HttpGatewayHealth;

pub const DEFAULT_HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
pub const DEFAULT_BACKPRESSURE_DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
pub const DEFAULT_PUBLIC_MAX_CONNECTIONS: usize = 1024;
pub const DEFAULT_DRAIN_DEADLINE: Duration = Duration::from_secs(2);

/// Optional WebSocket upgrade seam (E-ws wiring; additive, default `None`).
///
/// When set, the gateway accept loop calls [`GatewayUpgradeHandler::handle`]
/// for requests whose path matches [`GatewayUpgradeOptions::path`] and whose
/// `Upgrade` header is `websocket`, and the hyper connection is served with
/// `.with_upgrades()`. The handler owns the 101 handshake response and any
/// upgraded connection task (the supervisor tracks those tasks for shutdown).
/// When unset, the gateway keeps its exact pre-seam behavior: no upgrade
/// handling and no `.with_upgrades()`.
#[async_trait]
pub trait GatewayUpgradeHandler: Send + Sync + fmt::Debug {
    async fn handle(&self, request: Request<Incoming>) -> GatewayResponse;
}

#[derive(Debug, Clone)]
pub struct GatewayUpgradeOptions {
    pub path: String,
    pub handler: Arc<dyn GatewayUpgradeHandler>,
}

#[derive(Debug, Clone)]
pub struct HttpGatewayServerOptions {
    pub bind: SocketAddr,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub request_timeout: Duration,
    pub backpressure_drain_timeout: Duration,
    pub max_connections: usize,
    pub drain_deadline: Duration,
    pub stream_channel_capacity: usize,
    /// Additive E-ws seam: absent by default; see [`GatewayUpgradeOptions`].
    pub websocket_upgrade: Option<GatewayUpgradeOptions>,
}

impl HttpGatewayServerOptions {
    pub fn new(bind: SocketAddr, max_request_bytes: usize, max_response_bytes: usize) -> Self {
        Self {
            bind,
            max_request_bytes,
            max_response_bytes,
            request_timeout: DEFAULT_HTTP_REQUEST_TIMEOUT,
            backpressure_drain_timeout: DEFAULT_BACKPRESSURE_DRAIN_TIMEOUT,
            max_connections: DEFAULT_PUBLIC_MAX_CONNECTIONS,
            drain_deadline: DEFAULT_DRAIN_DEADLINE,
            stream_channel_capacity: DEFAULT_STREAM_CHANNEL_CAPACITY,
            websocket_upgrade: None,
        }
    }
}

#[derive(Debug)]
pub enum HttpServerError {
    Io(std::io::Error),
    Join(String),
}

impl fmt::Display for HttpServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "http gateway io error: {error}"),
            Self::Join(message) => write!(formatter, "http gateway join error: {message}"),
        }
    }
}

impl std::error::Error for HttpServerError {}

/// Running HTTP gateway: bound address, shutdown trigger and joined task.
pub struct HttpGatewayServer {
    addr: SocketAddr,
    shutdown_tx: watch::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    counters: Arc<Counters>,
}

impl HttpGatewayServer {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub async fn shutdown(self) -> Result<(), HttpServerError> {
        let _ = self.shutdown_tx.send(());
        self.task
            .await
            .map_err(|error| HttpServerError::Join(error.to_string()))
    }

    pub fn health(&self) -> HttpGatewayHealth {
        self.counters.snapshot()
    }
}

struct Counters {
    requests: AtomicU64,
    unary_dispatches: AtomicU64,
    stream_dispatches: AtomicU64,
    cors_preflights: AtomicU64,
    service_managed_cors: AtomicU64,
    selector_rejects: AtomicU64,
    ingress_misses: AtomicU64,
    request_too_large: AtomicU64,
    response_too_large: AtomicU64,
    backpressure_cancels: AtomicU64,
    client_disconnect_cancels: AtomicU64,
    timeouts: AtomicU64,
    platform_errors: AtomicU64,
}

impl Counters {
    fn new() -> Self {
        Self {
            requests: AtomicU64::new(0),
            unary_dispatches: AtomicU64::new(0),
            stream_dispatches: AtomicU64::new(0),
            cors_preflights: AtomicU64::new(0),
            service_managed_cors: AtomicU64::new(0),
            selector_rejects: AtomicU64::new(0),
            ingress_misses: AtomicU64::new(0),
            request_too_large: AtomicU64::new(0),
            response_too_large: AtomicU64::new(0),
            backpressure_cancels: AtomicU64::new(0),
            client_disconnect_cancels: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            platform_errors: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> HttpGatewayHealth {
        HttpGatewayHealth {
            requests: self.requests.load(Ordering::Relaxed),
            unary_dispatches: self.unary_dispatches.load(Ordering::Relaxed),
            stream_dispatches: self.stream_dispatches.load(Ordering::Relaxed),
            cors_preflights: self.cors_preflights.load(Ordering::Relaxed),
            service_managed_cors: self.service_managed_cors.load(Ordering::Relaxed),
            selector_rejects: self.selector_rejects.load(Ordering::Relaxed),
            ingress_misses: self.ingress_misses.load(Ordering::Relaxed),
            request_too_large: self.request_too_large.load(Ordering::Relaxed),
            response_too_large: self.response_too_large.load(Ordering::Relaxed),
            backpressure_cancels: self.backpressure_cancels.load(Ordering::Relaxed),
            client_disconnect_cancels: self.client_disconnect_cancels.load(Ordering::Relaxed),
            timeouts: self.timeouts.load(Ordering::Relaxed),
            platform_errors: self.platform_errors.load(Ordering::Relaxed),
        }
    }

    fn bump(&self, field: &AtomicU64) {
        field.fetch_add(1, Ordering::Relaxed);
    }
}

struct GatewayContext {
    epoch: Arc<RoutingEpoch>,
    resolver: Arc<dyn HttpIngressResolver>,
    dispatcher: Arc<dyn HttpDispatchPort>,
    options: HttpGatewayServerOptions,
    counters: Arc<Counters>,
}

/// Binds and starts the public HTTP gateway on a real socket.
pub async fn start_http_gateway(
    options: HttpGatewayServerOptions,
    epoch: Arc<RoutingEpoch>,
    resolver: Arc<dyn HttpIngressResolver>,
    dispatcher: Arc<dyn HttpDispatchPort>,
) -> Result<HttpGatewayServer, HttpServerError> {
    let listener = TcpListener::bind(options.bind)
        .await
        .map_err(HttpServerError::Io)?;
    let addr = listener.local_addr().map_err(HttpServerError::Io)?;
    let semaphore = Arc::new(Semaphore::new(options.max_connections.max(1)));
    let (shutdown_tx, shutdown_rx) = watch::channel(());
    let counters = Arc::new(Counters::new());
    let context = Arc::new(GatewayContext {
        epoch,
        resolver,
        dispatcher,
        options,
        counters: Arc::clone(&counters),
    });
    let task = tokio::spawn(serve(listener, semaphore, shutdown_rx, context));
    Ok(HttpGatewayServer {
        addr,
        shutdown_tx,
        task,
        counters,
    })
}

async fn serve(
    listener: TcpListener,
    semaphore: Arc<Semaphore>,
    mut shutdown_rx: watch::Receiver<()>,
    context: Arc<GatewayContext>,
) {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown_rx.changed() => break,
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
                let context = Arc::clone(&context);
                let upgrade_seam = context
                    .options
                    .websocket_upgrade
                    .as_ref()
                    .map(|options| {
                        (
                            options.path.clone(),
                            Arc::clone(&options.handler),
                        )
                    });
                let use_upgrades = upgrade_seam.is_some();
                let mut connection_shutdown = shutdown_rx.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let service = service_fn(move |request: Request<Incoming>| {
                        let context = Arc::clone(&context);
                        let upgrade_seam = upgrade_seam.clone();
                        async move {
                            if let Some((path, handler)) = &upgrade_seam {
                                if is_websocket_upgrade(&request)
                                    && request.uri().path() == path
                                {
                                    return handler.handle(request).await;
                                }
                            }
                            handle_request(&context, request).await
                        }
                    });
                    if use_upgrades {
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
                    } else {
                        let connection = http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service);
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
                    }
                });
            }
        }
    }
    let _ = timeout(context.options.drain_deadline, async {
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

type GatewayResponse = Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error>;

fn is_websocket_upgrade(request: &Request<Incoming>) -> bool {
    request
        .headers()
        .get(hyper::header::UPGRADE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
}

async fn handle_request(context: &GatewayContext, request: Request<Incoming>) -> GatewayResponse {
    context.counters.bump(&context.counters.requests);
    let (cancel_signal, cancel_watch) = cancel_channel();
    let mut cancel_on_drop = CancelOnDrop::new(cancel_signal.clone());
    let method = request.method().clone();
    let method_str = method.as_str().to_ascii_uppercase();
    let path = request.uri().path().to_string();
    let origin = first_header_value(request.headers(), "origin");

    let path_is_control = path == "/__router/health" || path == "/__router/prune-runtimes";
    if path_is_control {
        cancel_on_drop.defuse();
        return early_error_response(
            request,
            HttpError::platform(
                404,
                "ControlEndpointNotFound",
                "router control endpoints are served by the runtime/control listener",
                None,
            ),
            &cors_headers_for(origin.as_deref(), false),
            context.options.max_request_bytes,
        )
        .await;
    }
    if path == "/favicon.ico" {
        cancel_on_drop.defuse();
        return early_empty_response(
            request,
            StatusCode::NO_CONTENT,
            context.options.max_request_bytes,
        )
        .await;
    }

    let selector_result = parse_service_deployment_selector(request.headers());
    let selector = match selector_result {
        Ok(selector) => selector,
        Err(error) => {
            context.counters.bump(&context.counters.selector_rejects);
            cancel_on_drop.defuse();
            return early_error_response(
                request,
                error,
                &cors_headers_for(origin.as_deref(), false),
                context.options.max_request_bytes,
            )
            .await;
        }
    };
    let target = match parse_request_target(request.headers(), request.uri()) {
        Ok(target) => target,
        Err(error) => {
            context.counters.bump(&context.counters.selector_rejects);
            cancel_on_drop.defuse();
            return early_error_response(
                request,
                error,
                &cors_headers_for(origin.as_deref(), false),
                context.options.max_request_bytes,
            )
            .await;
        }
    };
    let service_manages_cors =
        context
            .resolver
            .has_explicit_options_ingress(&context.epoch, &selector, &target.path);
    if service_manages_cors {
        context
            .counters
            .bump(&context.counters.service_managed_cors);
    }
    let cors_headers = cors_headers_for(origin.as_deref(), service_manages_cors);

    if !service_manages_cors
        && cors::is_preflight_request(
            &method_str,
            origin.is_some(),
            has_header(request.headers(), "access-control-request-method"),
        )
    {
        if has_test_case_correlation_headers(request.headers()) {
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return early_error_response(
                request,
                HttpError::platform(
                    400,
                    "InvalidTestCaseCorrelation",
                    "test case capability self-ingress cannot use automatic CORS preflight",
                    None,
                ),
                &cors_headers,
                context.options.max_request_bytes,
            )
            .await;
        }
        if let Err(error) = parse_test_case_correlation(request.headers()) {
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return early_error_response(
                request,
                error,
                &cors_headers,
                context.options.max_request_bytes,
            )
            .await;
        }
        if !has_http_ingress_path(&context.epoch, &selector, &target.path) {
            context.counters.bump(&context.counters.ingress_misses);
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return early_error_response(
                request,
                HttpError::platform(
                    404,
                    "AssemblyIngressNotFound",
                    format!(
                        "No committed RuntimeAssembly ingress matches {} OPTIONS {}",
                        selector, target.path
                    ),
                    None,
                ),
                &cors_headers,
                context.options.max_request_bytes,
            )
            .await;
        }
        context.counters.bump(&context.counters.cors_preflights);
        cancel_on_drop.defuse();
        let response = preflight_response(
            origin.as_deref().unwrap_or_default(),
            request.headers().get("access-control-request-headers"),
        );
        drain_request_body(request.into_body(), context.options.max_request_bytes).await;
        return Ok(response);
    }

    let test_correlation = match parse_test_case_correlation(request.headers()) {
        Ok(correlation) => correlation,
        Err(error) => {
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return Ok(error_response(error, &cors_headers));
        }
    };
    let binding =
        match context
            .resolver
            .resolve(&context.epoch, &selector, &method_str, &target.path)
        {
            Ok(binding) => binding,
            Err(error) => {
                if error.status == 404 {
                    context.counters.bump(&context.counters.ingress_misses);
                }
                context.counters.bump(&context.counters.platform_errors);
                cancel_on_drop.defuse();
                return early_error_response(
                    request,
                    error,
                    &cors_headers,
                    context.options.max_request_bytes,
                )
                .await;
            }
        };
    let metadata = build_http_request_metadata(&method, &target, request.headers());
    let body = match read_request_body(request.into_body(), context.options.max_request_bytes).await
    {
        Ok(body) => body,
        Err(error) => {
            if error.status == 413 {
                context.counters.bump(&context.counters.request_too_large);
            }
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return Ok(error_response(error, &cors_headers));
        }
    };
    let header = match build_request_start_header(
        &context.epoch,
        &binding,
        new_request_id(),
        context.options.request_timeout,
        &metadata,
        test_correlation.as_ref(),
    ) {
        Ok(header) => header,
        Err(error) => {
            context.counters.bump(&context.counters.platform_errors);
            cancel_on_drop.defuse();
            return Ok(error_response(error, &cors_headers));
        }
    };

    let stream_watch = cancel_watch.clone();
    let dispatch_request = DispatchRequest {
        header,
        payload_bytes: body,
        timeout: context.options.request_timeout,
        client_disconnect: cancel_watch,
    };
    match binding.mode {
        HttpDispatchMode::Unary => {
            context.counters.bump(&context.counters.unary_dispatches);
            // The dispatch runs in its own task so a client disconnect that
            // drops this handler future does not silently destroy the pending
            // correlation; the dispatcher observes the cancel signal and
            // records the terminal (mirrors W-dispatch owning pending).
            let (result_tx, result_rx) = tokio::sync::oneshot::channel();
            let dispatcher = Arc::clone(&context.dispatcher);
            tokio::spawn(async move {
                let result = dispatcher.dispatch_unary(dispatch_request).await;
                let _ = result_tx.send(result);
            });
            let result = match result_rx.await {
                Ok(result) => result,
                Err(_) => Err(HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::CallbackError,
                    message: "unary dispatch task terminated without a result".to_string(),
                }),
            };
            match result {
                Ok(response) => {
                    cancel_on_drop.defuse();
                    if response.payload.len() > context.options.max_response_bytes {
                        context.counters.bump(&context.counters.response_too_large);
                        context.counters.bump(&context.counters.platform_errors);
                        return Ok(error_response(
                            HttpError::platform(
                                502,
                                "ResponseTooLarge",
                                format!(
                                    "runtime response exceeds {} bytes",
                                    context.options.max_response_bytes
                                ),
                                None,
                            ),
                            &cors_headers,
                        ));
                    }
                    Ok(response_with_headers(
                        response.status,
                        &response.headers,
                        Full::new(response.payload)
                            .map_err(|never| match never {})
                            .boxed(),
                        &cors_headers,
                    ))
                }
                Err(error) => {
                    cancel_on_drop.defuse();
                    count_dispatch_error(context, &error);
                    context.counters.bump(&context.counters.platform_errors);
                    Ok(error_response(map_dispatch_error(error), &cors_headers))
                }
            }
        }
        HttpDispatchMode::ServerStream => {
            context.counters.bump(&context.counters.stream_dispatches);
            let (sink, mut rx) = ChannelStreamSink::channel(
                context.options.stream_channel_capacity,
                context.options.max_response_bytes,
                context.options.backpressure_drain_timeout,
                cancel_signal.clone(),
            );
            let dispatcher = Arc::clone(&context.dispatcher);
            let mut dispatch_task = tokio::spawn(async move {
                let result = dispatcher
                    .dispatch_stream(dispatch_request, sink.clone())
                    .await;
                sink.close();
                result
            });
            let first = tokio::select! {
                biased;
                message = rx.recv() => match message {
                    Some(super::stream::StreamMessage::Start(http_response)) => {
                        cancel_on_drop.defuse();
                        stream_start_response(
                            http_response,
                            rx,
                            cancel_signal,
                            &cors_headers,
                        )
                    }
                    Some(_) => {
                        dispatch_task.abort();
                        Err(HttpError::platform(
                            502,
                            "InvalidHttpResponse",
                            "response.chunk/end received before response.start",
                            None,
                        ))
                    }
                    None => {
                        let result = match dispatch_task.await {
                            Ok(result) => result,
                            Err(join) => Err(HttpDispatchError::Cancelled {
                                source: PendingTerminalSource::CallbackError,
                                message: format!("stream dispatch task failed: {join}"),
                            }),
                        };
                        stream_dispatch_failure(context, result)
                    }
                },
                result = &mut dispatch_task => {
                    let result = match result {
                        Ok(result) => result,
                        Err(join) => Err(HttpDispatchError::Cancelled {
                            source: PendingTerminalSource::CallbackError,
                            message: format!("stream dispatch task failed: {join}"),
                        }),
                    };
                    cancel_on_drop.defuse();
                    stream_dispatch_failure(context, result)
                }
                _ = stream_watch.wait() => {
                    dispatch_task.abort();
                    cancel_on_drop.defuse();
                    context.counters.bump(&context.counters.client_disconnect_cancels);
                    context.counters.bump(&context.counters.platform_errors);
                    Err(HttpError::provider_unavailable(
                        "HTTP client disconnected before response.start",
                    ))
                }
            };
            match first {
                Ok(response) => Ok(response),
                Err(error) => Ok(error_response(error, &cors_headers)),
            }
        }
    }
}

fn stream_dispatch_failure(
    context: &GatewayContext,
    result: Result<(), HttpDispatchError>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, HttpError> {
    match result {
        Ok(()) => Err(HttpError::internal(
            "stream dispatch completed before response.start",
        )),
        Err(error) => {
            count_dispatch_error(context, &error);
            context.counters.bump(&context.counters.platform_errors);
            Err(map_dispatch_error(error))
        }
    }
}

fn stream_start_response(
    http_response: skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader,
    rx: tokio::sync::mpsc::Receiver<super::stream::StreamMessage>,
    cancel_signal: super::dispatch::CancelSignal,
    cors_headers: &[(String, String)],
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, HttpError> {
    let status = StatusCode::from_u16(http_response.status).map_err(|_| {
        HttpError::platform(
            502,
            "InvalidRuntimeResponse",
            "runtime response status is invalid",
            None,
        )
    })?;
    let mut builder = Response::builder().status(status);
    append_headers(&mut builder, cors_headers);
    append_runtime_headers(&mut builder, &http_response.headers, cors_headers)?;
    let body =
        BoxBody::new(ChannelStreamBody::new(rx, cancel_signal).map_err(|never| match never {}));
    builder
        .body(body)
        .map_err(|_| HttpError::internal("invalid stream response"))
}

fn count_dispatch_error(context: &GatewayContext, error: &HttpDispatchError) {
    match error {
        HttpDispatchError::Timeout { .. } => {
            context.counters.bump(&context.counters.timeouts);
        }
        HttpDispatchError::Cancelled { source, .. } => match source {
            PendingTerminalSource::ClientDisconnect => {
                context
                    .counters
                    .bump(&context.counters.client_disconnect_cancels);
            }
            PendingTerminalSource::Backpressure => {
                context
                    .counters
                    .bump(&context.counters.backpressure_cancels);
            }
            _ => {}
        },
        _ => {}
    }
}

fn map_dispatch_error(error: HttpDispatchError) -> HttpError {
    match error {
        HttpDispatchError::Control {
            code,
            message,
            status,
            details,
        } => HttpError::control_error(code, message, status, details),
        HttpDispatchError::FixedService(error) => {
            HttpError::fixed_service(error.envelope().trace_id(), error.envelope().error_id())
        }
        HttpDispatchError::Timeout { timeout_ms } => HttpError::timeout(timeout_ms),
        HttpDispatchError::Cancelled { source, message } => match source {
            PendingTerminalSource::ProtocolError | PendingTerminalSource::CallbackError => {
                HttpError::platform(502, "InvalidHttpResponse", message, None)
            }
            _ => HttpError::provider_unavailable(format!(
                "Runtime request {}: {message}",
                source.as_str()
            )),
        },
    }
}

fn cors_headers_for(origin: Option<&str>, service_manages_cors: bool) -> Vec<(String, String)> {
    if service_manages_cors {
        Vec::new()
    } else {
        origin.map(cors::automatic_cors_headers).unwrap_or_default()
    }
}

fn has_http_ingress_path(
    epoch: &RoutingEpoch,
    selector: &super::selector::ServiceDeploymentSelector,
    path: &str,
) -> bool {
    epoch.ingress_projection().iter().any(|binding| {
        binding.selector.protocol == skiff_artifact_model::IngressProtocol::Http
            && binding.selector.path == path
            && binding.deployment.service_id == selector.service_id
            && binding.deployment.contract_version == selector.contract_version
    })
}

fn preflight_response(
    origin: &str,
    requested_headers: Option<&hyper::header::HeaderValue>,
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let headers = cors::automatic_preflight_headers(
        origin,
        requested_headers.and_then(|value| value.to_str().ok()),
    );
    let mut builder = Response::builder().status(StatusCode::NO_CONTENT);
    append_headers(&mut builder, &headers);
    builder
        .body(empty_boxed())
        .expect("static preflight response is valid")
}

fn empty_response(
    status: StatusCode,
    headers: &[(String, String)],
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let mut builder = Response::builder().status(status);
    append_headers(&mut builder, headers);
    builder
        .body(empty_boxed())
        .expect("static empty response is valid")
}

/// Error response for paths that have not consumed the request body yet:
/// drains the body first so the connection closes with FIN, not RST.
async fn early_error_response(
    request: Request<Incoming>,
    error: HttpError,
    cors_headers: &[(String, String)],
    limit: usize,
) -> GatewayResponse {
    drain_request_body(request.into_body(), limit).await;
    Ok(error_response(error, cors_headers))
}

async fn early_empty_response(
    request: Request<Incoming>,
    status: StatusCode,
    limit: usize,
) -> GatewayResponse {
    drain_request_body(request.into_body(), limit).await;
    Ok(empty_response(status, &[]))
}

async fn drain_request_body(mut body: Incoming, limit: usize) {
    let mut total = 0usize;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            return;
        };
        if let Ok(data) = frame.into_data() {
            total = total.saturating_add(data.len());
            if total > limit {
                return;
            }
        }
    }
}

fn error_response(
    error: HttpError,
    cors_headers: &[(String, String)],
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let body = serde_json::to_vec(&error.json_body())
        .unwrap_or_else(|_| b"{\"error\":{\"code\":\"InternalGatewayError\",\"message\":\"error body failed to serialize\"}}".to_vec());
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(error.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR))
        .header("content-type", "application/json; charset=utf-8");
    append_headers(&mut builder, cors_headers);
    builder
        .body(
            Full::new(Bytes::from(body))
                .map_err(|never| match never {})
                .boxed(),
        )
        .expect("static error response is valid")
}

fn response_with_headers(
    status: u16,
    headers: &[skiff_runtime_transport::protocol::RuntimeHttpNameValueFrameHeader],
    body: BoxBody<Bytes, hyper::Error>,
    cors_headers: &[(String, String)],
) -> Response<BoxBody<Bytes, hyper::Error>> {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    append_headers(&mut builder, cors_headers);
    if let Err(error) = append_runtime_headers(&mut builder, headers, cors_headers) {
        return error_response(error, cors_headers);
    }
    builder.body(body).unwrap_or_else(|_| {
        error_response(
            HttpError::internal("invalid response headers"),
            cors_headers,
        )
    })
}

fn append_runtime_headers(
    builder: &mut ResponseBuilder,
    headers: &[skiff_runtime_transport::protocol::RuntimeHttpNameValueFrameHeader],
    cors_headers: &[(String, String)],
) -> Result<(), HttpError> {
    let platform_cors: Vec<String> = cors_headers.iter().map(|(name, _)| name.clone()).collect();
    for header in headers {
        let name = header.name.to_ascii_lowercase();
        if cors::is_cors_response_header(&name) && platform_cors.contains(&name) {
            continue;
        }
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            let value = hyper::header::HeaderValue::from_str(&header.value).map_err(|_| {
                HttpError::platform(
                    502,
                    "InvalidRuntimeResponse",
                    format!("runtime response header {name} is invalid"),
                    None,
                )
            })?;
            if let Some(headers) = builder.headers_mut() {
                headers.append(name, value);
            }
        }
    }
    Ok(())
}

fn append_headers(builder: &mut ResponseBuilder, headers: &[(String, String)]) {
    for (name, value) in headers {
        if let Ok(name) = HeaderName::from_bytes(name.as_bytes()) {
            if let Ok(value) = hyper::header::HeaderValue::from_str(value) {
                if let Some(headers) = builder.headers_mut() {
                    headers.append(name, value);
                }
            }
        }
    }
}

fn empty_boxed() -> BoxBody<Bytes, hyper::Error> {
    Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

async fn read_request_body(mut body: Incoming, limit: usize) -> Result<Bytes, HttpError> {
    let mut chunks = Vec::new();
    let mut total = 0usize;
    loop {
        let Some(frame) = body.frame().await else {
            break;
        };
        let frame = frame.map_err(|_| {
            HttpError::platform(400, "RequestDecodeError", "request body read failed", None)
        })?;
        if let Ok(data) = frame.into_data() {
            total = total.checked_add(data.len()).ok_or_else(|| {
                HttpError::platform(413, "RequestTooLarge", "request body is too large", None)
            })?;
            if total > limit {
                return Err(HttpError::platform(
                    413,
                    "RequestTooLarge",
                    format!("request body exceeds {limit} bytes"),
                    None,
                ));
            }
            chunks.push(data);
        }
    }
    Ok(chunks.into_iter().flatten().collect())
}

fn first_header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn has_header(headers: &HeaderMap, name: &str) -> bool {
    headers.get(name).is_some()
}

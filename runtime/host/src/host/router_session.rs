use std::collections::HashSet;

use futures_util::{Sink, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use skiff_runtime_capability_context::{
    ConnectionRequestSession, ConnectionRequestTerminal, RouterWriteFailure,
};
use skiff_runtime_request::{OutboundResponse, ResponseError};
#[cfg(test)]
use skiff_runtime_transport::protocol::RouterControlEnvelope;
use skiff_runtime_transport::{
    connection_protocol::{decode_connection_response_frame, ConnectionResponseOutcome},
    control_mapper::encode_outbound_control_message,
    protocol::{
        decode_bytecode_request_start_frame, decode_router_bootstrap_frame_header,
        decode_typed_binary_frame, ActorFindResponseFrameHeader,
        ActorGetOrCreateResponseFrameHeader, ActorRemoveResponseFrameHeader,
        ActorReplaceResponseFrameHeader, ActorTaskRuntimeErrorFrameHeader,
        RequestCancelFrameHeader, RouterBootstrapServiceDbFrameHeader, RuntimeErrorFramePayload,
        RuntimeHealthCountersFrameHeader, RuntimeRegisteredFrameHeader,
        TaskCancelResponseFrameHeader, TaskStatusResponseFrameHeader,
        TaskSubmitResponseFrameHeader, TypedEnvelope,
    },
    request_mapper::request_cancel_from_frame_header,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    time::{Duration, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream};
use tracing::{info, warn};

use crate::error::{Result, RuntimeError};

mod handshake;
pub(crate) mod task_submit;

use super::request_supervisor::RouterSessionEpoch;
use handshake::{
    ClientHandshake, ClientHandshakePhase, ClientTerminalKind, ClientTimeoutKind,
    HandshakeDeadlines,
};
use task_submit::{encode_task_submit_wire_message, legacy_task_submit_rejected};

fn handshake_terminal_error(terminal: ClientTerminalKind) -> RuntimeError {
    RuntimeError::Decode(format!(
        "runtime handshake terminal {}: {terminal:?}",
        terminal.description(),
    ))
}

pub(super) async fn run_once(host: super::RuntimeHost) -> Result<()> {
    let (ws, _) = connect_async(&host.router_url)
        .await
        .map_err(|error| RuntimeError::Decode(format!("failed to connect router: {error}")))?;
    info!(
        event = "runtime.router_connected",
        router = %host.router_url
    );
    let router_session_id = format!("skiff-router-session-v1:opaque:{}", uuid::Uuid::new_v4());
    run_connected_session(host, ws, router_session_id).await
}

async fn run_connected_session<S>(
    host: super::RuntimeHost,
    ws: WebSocketStream<S>,
    router_session_id: String,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    run_connected_session_with_bootstrap(host, ws, router_session_id, None).await
}

#[cfg(test)]
#[allow(dead_code)]
pub(super) async fn run_connected_session_with_deadlines<S>(
    host: super::RuntimeHost,
    ws: WebSocketStream<S>,
    router_session_id: String,
    initial_bootstrap: Option<ConnectionBootstrap>,
    deadlines: HandshakeDeadlines,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    run_connected_session_full(host, ws, router_session_id, initial_bootstrap, deadlines).await
}

async fn run_connected_session_with_bootstrap<S>(
    host: super::RuntimeHost,
    ws: WebSocketStream<S>,
    router_session_id: String,
    initial_bootstrap: Option<ConnectionBootstrap>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    run_connected_session_full(
        host,
        ws,
        router_session_id,
        initial_bootstrap,
        HandshakeDeadlines::default(),
    )
    .await
}

async fn run_connected_session_full<S>(
    host: super::RuntimeHost,
    ws: WebSocketStream<S>,
    router_session_id: String,
    initial_bootstrap: Option<ConnectionBootstrap>,
    handshake_deadlines: HandshakeDeadlines,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let router_session_epoch = RouterSessionEpoch::from_connection_id(router_session_id)
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    if !host
        .request_supervisor
        .start_session(router_session_epoch.clone())
    {
        return Err(RuntimeError::Decode(
            "router session epoch is already connected".to_string(),
        ));
    }
    let mut ws = ws;
    let (sender, receiver) = mpsc::unbounded_channel::<super::RouterWriterMessage>();
    let mut receiver = receiver;
    // Test shortcut connections start with the handshake already completed.
    let mut handshake = if initial_bootstrap.is_some() {
        ClientHandshake::registered()
    } else {
        ClientHandshake::new()
    };
    let mut handshake_deadline = if handshake.phase() == ClientHandshakePhase::WaitingBootstrap {
        Some(tokio::time::Instant::now() + handshake_deadlines.bootstrap)
    } else {
        None
    };

    let mut session_guard =
        ConnectedRouterSessionGuard::new(host.clone(), router_session_epoch.clone());
    let session_result = async {
        let mut health_reporter = RuntimeHealthReporter::default();
        let mut bootstrap = initial_bootstrap;
        let mut health_interval = tokio::time::interval(Duration::from_secs(1));
        health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut health_zero_transition_interval = tokio::time::interval(Duration::from_millis(50));
        health_zero_transition_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                message = ws.next() => {
                    if !handle_router_session_message(
                        &host,
                        &mut ws,
                        message,
                        &router_session_epoch,
                        &sender,
                        &mut health_reporter,
                        &mut bootstrap,
                        &mut handshake,
                    )
                    .await?
                    {
                        break;
                    }
                    if handshake.phase() == ClientHandshakePhase::BootstrapReceived {
                        handshake_deadline = Some(
                            tokio::time::Instant::now() + handshake_deadlines.registered,
                        );
                    } else if matches!(
                        handshake.phase(),
                        ClientHandshakePhase::Registered | ClientHandshakePhase::Closed
                    ) {
                        handshake_deadline = None;
                    }
                }
                message = receiver.recv() => {
                    let Some(message) = message else {
                        break;
                    };
                    send_writer_message(&mut ws, message).await?;
                    handshake.on_registration_write_flushed();
                    if handshake.phase() == ClientHandshakePhase::Registered {
                        handshake_deadline = None;
                    }
                }
                _ = health_interval.tick(), if health_reporter.has_registered_runtimes() => {
                    health_reporter.send_periodic(&host, &sender).await?;
                }
                _ = health_zero_transition_interval.tick(), if health_reporter.should_probe_zero_transition() => {
                    health_reporter.send_zero_transition_if_needed(&host, &sender).await?;
                }
                _ = tokio::time::sleep_until(handshake_deadline.unwrap_or_else(tokio::time::Instant::now)), if handshake_deadline.is_some() => {
                    let kind = if handshake.phase() == ClientHandshakePhase::WaitingBootstrap {
                        ClientTimeoutKind::Bootstrap
                    } else {
                        ClientTimeoutKind::Registered
                    };
                    let terminal = handshake.on_timeout(kind);
                    return Err(handshake_terminal_error(terminal));
                }
            }
        }

        Ok(())
    }
    .await;

    let disconnect_result = session_guard.close();
    drop(sender);
    session_result.and(disconnect_result)
}

#[allow(clippy::too_many_arguments)]
async fn handle_router_session_message<S>(
    host: &super::RuntimeHost,
    ws: &mut WebSocketStream<S>,
    message: Option<std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>,
    router_session: &RouterSessionEpoch,
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    health_reporter: &mut RuntimeHealthReporter,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Some(message) = message else {
        return Ok(false);
    };
    let message =
        message.map_err(|error| RuntimeError::Decode(format!("router read failed: {error}")))?;
    match message {
        Message::Text(text) => {
            reject_router_text_message(text.as_str())?;
        }
        Message::Binary(bytes) => {
            let frame_type = router_binary_frame_type(&bytes)?;
            if frame_type == "router.bootstrap" {
                if let Err(terminal) = handshake.on_bootstrap() {
                    return Err(handshake_terminal_error(terminal));
                }
            } else if frame_type != "runtime.registered"
                && matches!(
                    frame_type.as_str(),
                    "runtime.capabilities" | "runtime.health"
                )
            {
                let terminal = handshake.on_direction_violation(&frame_type);
                return Err(handshake_terminal_error(terminal));
            } else if frame_type != "runtime.registered" {
                if let Err(terminal) = handshake.on_business_frame() {
                    return Err(handshake_terminal_error(terminal));
                }
            }
            dispatch_router_binary_frame_with_health(
                host,
                router_session,
                &bytes,
                sender,
                health_reporter,
                bootstrap,
                handshake,
            )
            .await?;
        }
        Message::Ping(_) => {
            ws.flush().await.map_err(|error| {
                RuntimeError::Decode(format!("failed to flush Router ping reply: {error}"))
            })?;
        }
        Message::Pong(_) => {}
        Message::Close(close) => {
            if let Some(frame) = close {
                warn!(
                    event = "runtime.router_close_frame",
                    close_code = %frame.code,
                    close_reason = %frame.reason,
                );
            }
            ws.flush().await.map_err(|error| {
                RuntimeError::Decode(format!("failed to flush Router close reply: {error}"))
            })?;
            return Ok(false);
        }
        Message::Frame(_) => {}
    }
    Ok(true)
}

fn router_binary_frame_type(bytes: &[u8]) -> Result<String> {
    let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    Ok(typed.envelope_type)
}

struct ConnectedRouterSessionGuard {
    host: super::RuntimeHost,
    router_session: RouterSessionEpoch,
    closed: bool,
}

impl ConnectedRouterSessionGuard {
    fn new(host: super::RuntimeHost, router_session: RouterSessionEpoch) -> Self {
        Self {
            host,
            router_session,
            closed: false,
        }
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.host
            .request_supervisor
            .stop_session(&self.router_session);
        if let Ok(session) = ConnectionRequestSession::new(self.router_session.as_str().to_string())
        {
            self.host.connection_requests.disconnect_session(&session);
        }
        self.host.outbound_requests.fail_all(ResponseError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
            status: None,
            details: None,
        });
        Ok(())
    }
}

impl Drop for ConnectedRouterSessionGuard {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Clone)]
pub(crate) struct ConnectionBootstrap {
    pub(crate) resolver: skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver,
    pub(crate) activation: skiff_runtime_transport::protocol::RouterBootstrapActivationFrameHeader,
    pub(crate) max_response_bytes: usize,
}

#[cfg(test)]
fn test_bootstrap_activation(
) -> skiff_runtime_transport::protocol::RouterBootstrapActivationFrameHeader {
    serde_json::from_value(serde_json::json!({
        "profile": "test"
    }))
    .expect("test bootstrap activation must decode")
}

#[cfg(test)]
#[allow(dead_code)]
fn test_connection_bootstrap(name: &str) -> Result<ConnectionBootstrap> {
    let artifact_path = std::env::temp_dir().join(format!(
        "skiff-runtime-test-artifacts-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_path)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    Ok(ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        activation: test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    })
}

fn decode_connection_bootstrap(
    typed: TypedEnvelope,
    payload: &[u8],
) -> Result<(
    ConnectionBootstrap,
    Option<RouterBootstrapServiceDbFrameHeader>,
)> {
    if !payload.is_empty() {
        return Err(RuntimeError::Decode(
            "router.bootstrap binary frame payload must be empty".to_string(),
        ));
    }
    let mut value = typed.rest;
    value.insert("type".to_string(), Value::String(typed.envelope_type));
    let header = decode_router_bootstrap_frame_header(Value::Object(value))
        .map_err(super::transport_error_into_runtime_error)?;
    let resolver = skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver::open(
        &header.artifacts_path,
    )
    .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let max_response_bytes = usize::try_from(header.http.max_response_bytes).map_err(|_| {
        RuntimeError::Decode(
            "router.bootstrap http.maxResponseBytes exceeds Runtime address space".to_string(),
        )
    })?;
    Ok((
        ConnectionBootstrap {
            resolver,
            activation: header.activation,
            max_response_bytes,
        },
        Some(header.service_db),
    ))
}

#[derive(Default)]
struct RuntimeHealthReporter {
    registered_runtime_ids: HashSet<String>,
    last_counters_nonzero: bool,
}

impl RuntimeHealthReporter {
    fn has_registered_runtimes(&self) -> bool {
        !self.registered_runtime_ids.is_empty()
    }

    fn should_probe_zero_transition(&self) -> bool {
        self.has_registered_runtimes() && self.last_counters_nonzero
    }

    async fn record_registered(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
        runtime_id: String,
    ) -> Result<()> {
        self.registered_runtime_ids.insert(runtime_id);
        self.send_current(host, sender).await
    }

    async fn send_periodic(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    ) -> Result<()> {
        self.send_current(host, sender).await
    }

    async fn send_zero_transition_if_needed(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    ) -> Result<bool> {
        if !self.should_probe_zero_transition() {
            return Ok(false);
        }
        let counters = host.runtime_health_counters().await;
        self.send_zero_transition_for_counters(host, sender, counters)
            .await
    }

    async fn send_zero_transition_for_counters(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
        counters: RuntimeHealthCountersFrameHeader,
    ) -> Result<bool> {
        if !self.should_probe_zero_transition() {
            return Ok(false);
        }
        if !runtime_health_counters_all_zero(&counters) {
            self.last_counters_nonzero = true;
            return Ok(false);
        }
        self.send_counters(host, sender, counters).await?;
        Ok(true)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    async fn send_final(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    ) -> Result<()> {
        if !self.has_registered_runtimes() {
            return Ok(());
        }
        self.send_current(host, sender).await
    }

    async fn send_current(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    ) -> Result<()> {
        let counters = host.runtime_health_counters().await;
        self.send_counters(host, sender, counters).await
    }

    async fn send_counters(
        &mut self,
        host: &super::RuntimeHost,
        sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
        counters: RuntimeHealthCountersFrameHeader,
    ) -> Result<()> {
        self.last_counters_nonzero = !runtime_health_counters_all_zero(&counters);
        for runtime_id in self.registered_runtime_ids.iter() {
            host.queue_runtime_health_with_counters(sender, runtime_id, counters.clone())
                .await?;
        }
        Ok(())
    }
}

fn runtime_health_counters_all_zero(counters: &RuntimeHealthCountersFrameHeader) -> bool {
    counters.outbound_requests_pending == 0
        && counters.outbound_stream_leases_active == 0
        && counters.stream_runtime_streams_active == 0
        && counters.flag_backed_cancel_waiters_active == 0
        && counters.task_requests_active == 0
}

#[cfg(test)]
async fn dispatch_router_binary_frame(
    host: &super::RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &mut Option<RouterControlEnvelope>,
    artifact_fingerprint: &mut Option<String>,
) -> Result<()> {
    let _ = (control, artifact_fingerprint);
    let artifact_path = std::env::temp_dir().join("skiff-runtime-test-artifacts");
    std::fs::create_dir_all(&artifact_path)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let mut bootstrap = Some(ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        activation: test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    });
    let mut handshake = ClientHandshake::registered();
    let router_session = RouterSessionEpoch::from_connection_id(
        "skiff-router-session-v1:opaque:test-session".to_string(),
    )
    .unwrap();
    let _ = host
        .request_supervisor
        .start_session(router_session.clone());
    dispatch_router_binary_frame_inner(
        host,
        &router_session,
        bytes,
        sender,
        None,
        &mut bootstrap,
        &mut handshake,
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn dispatch_router_binary_frame_with_http_response_max(
    host: &super::RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    max_response_bytes: usize,
) -> Result<()> {
    let artifact_path = std::env::temp_dir().join("skiff-runtime-test-artifacts");
    std::fs::create_dir_all(&artifact_path)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let mut bootstrap = Some(ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        activation: test_bootstrap_activation(),
        max_response_bytes,
    });
    let mut handshake = ClientHandshake::registered();
    let router_session = RouterSessionEpoch::from_connection_id(
        "skiff-router-session-v1:opaque:test-session".to_string(),
    )
    .unwrap();
    let _ = host
        .request_supervisor
        .start_session(router_session.clone());
    dispatch_router_binary_frame_inner(
        host,
        &router_session,
        bytes,
        sender,
        None,
        &mut bootstrap,
        &mut handshake,
    )
    .await
}

async fn dispatch_router_binary_frame_with_health(
    host: &super::RuntimeHost,
    router_session: &RouterSessionEpoch,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    health_reporter: &mut RuntimeHealthReporter,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
) -> Result<()> {
    dispatch_router_binary_frame_inner(
        host,
        router_session,
        bytes,
        sender,
        Some(health_reporter),
        bootstrap,
        handshake,
    )
    .await
}

async fn dispatch_router_binary_frame_inner(
    host: &super::RuntimeHost,
    router_session: &RouterSessionEpoch,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    health_reporter: Option<&mut RuntimeHealthReporter>,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
) -> Result<()> {
    let (typed, payload) = decode_typed_binary_frame::<TypedEnvelope>(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    match typed.envelope_type.as_str() {
        "router.bootstrap" => {
            if bootstrap.is_some() {
                return Err(RuntimeError::Decode(
                    "router.bootstrap must appear exactly once per connection".to_string(),
                ));
            }
            let (installed, service_db) = decode_connection_bootstrap(typed, &payload)?;
            host.set_db_service_db(service_db);
            // Bootstrap carries only the profile and the artifact root: no
            // committed tuple, no config snapshot, no recovery.
            host.freeze_bootstrap_profile(&installed.activation.profile)
                .map_err(|error| {
                    RuntimeError::Decode(format!(
                        "router bootstrap activation profile check failed: {error:#}"
                    ))
                })?;
            host.set_bootstrap_artifact_root(
                installed.resolver.store().root().display().to_string(),
            );
            host.queue_connection_registration(sender.clone())?;
            handshake.mark_registration_queued();
            *bootstrap = Some(installed);
        }
        "runtime.registered" => {
            let (header, payload) =
                decode_typed_binary_frame::<RuntimeRegisteredFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "runtime.registered binary frame payload must be empty".to_string(),
                ));
            }
            handshake
                .on_registered(&header.runtime_id, &host.base_runtime_id)
                .map_err(handshake_terminal_error)?;
            let mut rest = serde_json::Map::new();
            rest.insert("runtimeId".to_string(), Value::String(header.runtime_id));
            host.log_registered(&rest);
            let runtime_id = rest
                .get("runtimeId")
                .and_then(Value::as_str)
                .expect("runtimeId should be set")
                .to_string();
            if let Some(health_reporter) = health_reporter {
                health_reporter
                    .record_registered(host, sender, runtime_id)
                    .await?;
            }
        }
        "router.control" => {
            return Err(RuntimeError::Decode(
                "router.control artifactRoots/serviceConfig reload is not supported".to_string(),
            ));
        }
        "request.start" => {
            if bootstrap.is_none() {
                return Err(RuntimeError::Decode(
                    "request.start requires router.bootstrap first".to_string(),
                ));
            }
            let (header, payload) = decode_bytecode_request_start_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            let bootstrap = bootstrap.as_ref().expect("bootstrap checked above");
            host.spawn_bytecode_request(router_session, header, payload, bootstrap, sender.clone())
                .await;
        }
        "request.cancel" => {
            let (header, payload) = decode_typed_binary_frame::<RequestCancelFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "request.cancel binary frame payload must be empty".to_string(),
                ));
            }
            host.cancel_request(router_session, request_cancel_from_frame_header(header))
                .await;
        }
        "connection.response" => {
            let (header, payload) = decode_connection_response_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            let request_id = header.request_id.clone();
            let session = ConnectionRequestSession::new(router_session.as_str().to_string())
                .map_err(RuntimeError::Decode)?;
            let terminal = match header.outcome {
                ConnectionResponseOutcome::Success => ConnectionRequestTerminal::Success(payload),
                ConnectionResponseOutcome::DeadlineExceeded => {
                    ConnectionRequestTerminal::DeadlineExceeded
                }
                ConnectionResponseOutcome::ConnectionUnavailable => {
                    ConnectionRequestTerminal::ConnectionUnavailable
                }
                ConnectionResponseOutcome::TransportUnavailable => {
                    ConnectionRequestTerminal::TransportUnavailable
                }
                ConnectionResponseOutcome::ProtocolError => {
                    ConnectionRequestTerminal::ProtocolError
                }
                ConnectionResponseOutcome::ResourceLimit => {
                    ConnectionRequestTerminal::ResourceLimit
                }
                ConnectionResponseOutcome::Remote => {
                    let remote = header
                        .remote
                        .expect("strict connection response decoder requires remote metadata");
                    ConnectionRequestTerminal::Remote {
                        code: remote.code,
                        message: remote.message,
                        data: remote.data_present.then_some(payload),
                    }
                }
            };
            if !host
                .connection_requests
                .complete(&session, &request_id, terminal)
            {
                warn!(
                    event = "runtime.unmatched_connection_response",
                    request_id = %request_id,
                    router_session_id = router_session.as_str(),
                );
            }
        }
        "actor.owner.invoke"
        | "actor.owner.control"
        | "actor.owner.failure"
        | "actor.method.return"
        | "actor.method.error"
        | "actor.method.cancel" => {
            return Err(RuntimeError::Unsupported(
                "legacy actor frames are not supported by bytecode runtime".to_string(),
            ));
        }
        "actor.getOrCreate.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorGetOrCreateResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "actor.getOrCreate.response",
            )?;
        }
        "actor.replace.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorReplaceResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "actor.replace.response",
            )?;
        }
        "actor.find.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorFindResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "actor.find.response",
            )?;
        }
        "actor.remove.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorRemoveResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "actor.remove.response",
            )?;
        }
        "task.submit.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<TaskSubmitResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "task.submit.response",
            )?;
        }
        "actor.getOrCreate.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.getOrCreate.error",
            )?;
        }
        "actor.replace.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.replace.error",
            )?;
        }
        "actor.find.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.find.error",
            )?;
        }
        "actor.remove.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.remove.error",
            )?;
        }
        "task.submit.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "task.submit.error",
            )?;
        }
        "task.status.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<TaskStatusResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "task.status.response",
            )?;
        }
        "task.status.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "task.status.error",
            )?;
        }
        "task.cancel.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<TaskCancelResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "task.cancel.response",
            )?;
        }
        "task.cancel.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorTaskRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "task.cancel.error",
            )?;
        }
        other => {
            warn!(
                event = "runtime.unsupported_router_binary_frame",
                envelope_type = other,
                payload_bytes = payload.len()
            );
        }
    }
    Ok(())
}

fn dispatch_control_response<THeader: Serialize>(
    host: &super::RuntimeHost,
    rpc_id: &str,
    header: &THeader,
    payload: Vec<u8>,
    envelope_type: &'static str,
) -> Result<()> {
    if !payload.is_empty() {
        return Err(RuntimeError::Decode(format!(
            "{envelope_type} binary frame payload must be empty"
        )));
    }
    let response = serde_json::to_vec(header).map_err(RuntimeError::from)?;
    if let Some(sender) = host.outbound_requests.take_terminal_sender(rpc_id) {
        let _ = sender.send(OutboundResponse::End { payload: response });
    } else {
        warn!(
            event = "runtime.unmatched_outbound_control_response",
            envelope_type,
            rpc_id = %rpc_id
        );
    }
    Ok(())
}

fn dispatch_control_error(
    host: &super::RuntimeHost,
    rpc_id: &str,
    payload: Vec<u8>,
    error: RuntimeErrorFramePayload,
    envelope_type: &'static str,
) -> Result<()> {
    if !payload.is_empty() {
        return Err(RuntimeError::Decode(format!(
            "{envelope_type} binary frame payload must be empty"
        )));
    }
    if let Some(sender) = host.outbound_requests.take_terminal_sender(rpc_id) {
        let _ = sender.send(OutboundResponse::Error(response_error_from_frame(error)));
    } else {
        warn!(
            event = "runtime.unmatched_outbound_control_error",
            envelope_type,
            rpc_id = %rpc_id
        );
    }
    Ok(())
}

fn response_error_from_frame(error: RuntimeErrorFramePayload) -> ResponseError {
    ResponseError {
        code: error.code,
        message: error.message,
        status: error.status,
        details: error.details,
    }
}

fn encode_writer_message(message: super::RouterWriterMessage) -> Result<Message> {
    match message {
        super::RouterWriterMessage::Binary(bytes) => Ok(Message::Binary(bytes.into())),
        super::RouterWriterMessage::StreamFrame { .. } => Err(RuntimeError::Decode(
            "server-stream frames require the flush-aware WebSocket writer path".to_string(),
        )),
        super::RouterWriterMessage::TaskSubmit(message) => {
            encode_task_submit_wire_message(message).map(|bytes| Message::Binary(bytes.into()))
        }
        super::RouterWriterMessage::Control(command) => match command {
            skiff_runtime_request::OutboundControlMessage::TaskSubmit { .. } => {
                Err(legacy_task_submit_rejected())
            }
            other => encode_outbound_control_message(other)
                .map_err(super::transport_error_into_runtime_error)
                .map(|bytes| Message::Binary(bytes.into())),
        },
    }
}

async fn send_writer_message<S>(writer: &mut S, message: super::RouterWriterMessage) -> Result<()>
where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if let super::RouterWriterMessage::StreamFrame { bytes, flush_ack } = message {
        return match writer.send(Message::Binary(bytes.into())).await {
            Ok(()) => {
                let _ = flush_ack.send(Ok(()));
                Ok(())
            }
            Err(error) => {
                let message = format!("router write failed: {error}");
                let _ = flush_ack.send(Err(RouterWriteFailure::WebSocketWrite {
                    message: message.clone(),
                }));
                Err(RuntimeError::Decode(message))
            }
        };
    }
    let message = encode_writer_message(message)?;
    writer
        .send(message)
        .await
        .map_err(|error| RuntimeError::Decode(format!("router write failed: {error}")))
}

fn reject_router_text_message(_text: &str) -> Result<()> {
    Err(RuntimeError::Decode(
        "text protocol messages are not supported on runtime WebSocket; use binary runtime frames"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests;

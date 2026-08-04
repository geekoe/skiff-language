use std::{collections::HashSet, future::Future, panic::AssertUnwindSafe, pin::Pin};

use futures_util::{stream::FuturesUnordered, FutureExt, Sink, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use skiff_runtime_capability_context::{
    ActorInvocationCancellation, ActorInvocationError, ActorInvocationOutcome,
    ConnectionRequestSession, ConnectionRequestTerminal,
};
use skiff_runtime_request::{OutboundResponse, ResponseError};
#[cfg(test)]
use skiff_runtime_transport::protocol::RouterControlEnvelope;
use skiff_runtime_transport::{
    actor_method::{decode_actor_method_frame, ActorMethodErrorFramePayload, ActorMethodFrame},
    actor_owner::{
        decode_actor_owner_control_frame, decode_actor_owner_failure_frame,
        decode_actor_owner_invoke_frame, encode_actor_owner_control_ack_frame,
        ActorOwnerControlAckFrameHeader, ActorOwnerControlOperation,
        ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE, ACTOR_OWNER_CONTROL_FRAME_TYPE,
        ACTOR_OWNER_FAILURE_FRAME_TYPE, ACTOR_OWNER_INVOKE_FRAME_TYPE,
    },
    assembly_activation::{
        decode_assembly_activation_frame, AssemblyActivationFrameDirection,
        ASSEMBLY_ACTIVATION_FRAME_TYPE,
    },
    connection_protocol::{decode_connection_response_frame, ConnectionResponseOutcome},
    control_mapper::encode_outbound_control_message,
    protocol::{
        decode_router_bootstrap_frame_header, decode_typed_binary_frame,
        ActorFindResponseFrameHeader, ActorGetOrCreateResponseFrameHeader,
        ActorRemoveResponseFrameHeader, ActorReplaceResponseFrameHeader,
        ActorTaskRuntimeErrorFrameHeader, RequestCancelFrameHeader, RuntimeErrorFramePayload,
        RuntimeHealthCountersFrameHeader, RuntimeRegisteredFrameHeader,
        TaskCancelResponseFrameHeader, TaskStatusResponseFrameHeader,
        TaskSubmitResponseFrameHeader, TypedEnvelope,
    },
    request_mapper::request_cancel_from_frame_header,
    runtime_assembly_request::decode_runtime_assembly_request_start_frame,
    websocket_generation_lifecycle::WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc,
    task::JoinSet,
    time::{Duration, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, WebSocketStream};
use tracing::{info, warn};

use crate::error::{Result, RuntimeError};

mod activation;
mod handshake;
pub(crate) mod task_submit;

use activation::{
    cleanup_session_activation, dispatch_session_activation_frame, router_binary_frame_type,
    terminal_message, SessionActivationState,
};
use handshake::{
    ClientHandshake, ClientHandshakePhase, ClientTerminalKind, ClientTimeoutKind,
    HandshakeDeadlines,
};
use task_submit::{encode_task_submit_wire_message, legacy_task_submit_rejected};

fn handshake_terminal_error(terminal: ClientTerminalKind) -> RuntimeError {
    RuntimeError::Decode(format!(
        "runtime handshake terminal {}: {}",
        terminal.description(),
        format!("{terminal:?}")
    ))
}

const TERMINAL_ABORT_GRACE: Duration = Duration::from_millis(2);

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

    host.websocket_generations.connect(&router_session_id)?;
    if let Err(error) = host.open_actor_instance_session(&router_session_id) {
        let _ = host.websocket_generations.disconnect(&router_session_id);
        return Err(RuntimeError::Decode(error.to_string()));
    }
    let mut session_guard =
        ConnectedRouterSessionGuard::new(host.clone(), router_session_id.clone());
    // Actor owner work is connection-scoped. Poll it as a child of this session instead of
    // detaching Tokio tasks, so every exit path drops all in-flight activation/test leases before
    // session teardown returns.
    let mut child_tasks = RouterSessionChildTasks::default();
    let mut activation_state = SessionActivationState::Idle;
    let mut activation_prepare_tasks = JoinSet::new();

    let session_result = async {
        let mut health_reporter = RuntimeHealthReporter::default();
        let mut bootstrap = initial_bootstrap;
        let mut health_interval = tokio::time::interval(Duration::from_secs(1));
        health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut health_zero_transition_interval = tokio::time::interval(Duration::from_millis(50));
        health_zero_transition_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            activation_state.assert_task_invariant(&activation_prepare_tasks)?;

            // Give an exact Abort that raced prepare completion one bounded read opportunity.
            // Once this single probe is consumed, terminal delivery is forced before the
            // ordinary fair session select can read another frame.
            if activation_state.should_probe_terminal_abort() {
                let inbound = tokio::time::timeout(TERMINAL_ABORT_GRACE, ws.next()).await;
                activation_state.finish_terminal_abort_probe()?;
                if let Ok(message) = inbound {
                    if !handle_router_session_message(
                        &host,
                        &mut ws,
                        message,
                        &router_session_id,
                        &sender,
                        &mut health_reporter,
                        &mut bootstrap,
                        &mut handshake,
                        &mut child_tasks,
                        &mut activation_state,
                        &mut activation_prepare_tasks,
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
                    continue;
                }
            }

            if activation_state.is_terminal_ready() {
                let message = terminal_message(&activation_state)?;
                #[cfg(test)]
                activation::inject_terminal_send_failure(&activation_state)?;
                send_writer_message(&mut ws, message).await?;
                activation_state.mark_terminal_sent()?;
                continue;
            }

            tokio::select! {
                message = ws.next() => {
                    if !handle_router_session_message(
                        &host,
                        &mut ws,
                        message,
                        &router_session_id,
                        &sender,
                        &mut health_reporter,
                        &mut bootstrap,
                        &mut handshake,
                        &mut child_tasks,
                        &mut activation_state,
                        &mut activation_prepare_tasks,
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
                completed = activation_prepare_tasks.join_next(), if activation_state.is_preparing() => {
                    let result = match completed {
                        Some(Ok(Ok(result))) => result,
                        Some(Ok(Err(error))) => return Err(error),
                        Some(Err(error)) => return Err(RuntimeError::Decode(format!(
                            "assembly activation prepare task failed: {error}"
                        ))),
                        None => return Err(RuntimeError::Decode(
                            "pending assembly activation task disappeared".to_string()
                        )),
                    };
                    if !activation_prepare_tasks.is_empty() {
                        return Err(RuntimeError::Decode(
                            "multiple assembly activation prepare tasks were active".to_string()
                        ));
                    }
                    activation_state.complete_prepare(result)?;
                }
                _ = health_interval.tick(), if health_reporter.has_registered_runtimes() => {
                    health_reporter.send_periodic(&host, &sender).await?;
                }
                _ = health_zero_transition_interval.tick(), if health_reporter.should_probe_zero_transition() => {
                    health_reporter.send_zero_transition_if_needed(&host, &sender).await?;
                }
                _ = child_tasks.next(), if !child_tasks.is_empty() => {}
                _ = tokio::time::sleep_until(handshake_deadline.unwrap_or_else(|| tokio::time::Instant::now())), if handshake_deadline.is_some() => {
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

    // Dropping the owned futures synchronously runs their activation and test-derived lease RAII.
    // Do this before closing Actor/test registries, so teardown never races a surviving child.
    drop(child_tasks);
    let activation_cleanup_result =
        cleanup_session_activation(&host, &mut activation_state, &mut activation_prepare_tasks)
            .await;
    let disconnect_result = session_guard.close();
    drop(sender);
    session_result
        .and(activation_cleanup_result)
        .and(disconnect_result)
}

async fn handle_router_session_message<S>(
    host: &super::RuntimeHost,
    ws: &mut WebSocketStream<S>,
    message: Option<std::result::Result<Message, tokio_tungstenite::tungstenite::Error>>,
    router_session_id: &str,
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    health_reporter: &mut RuntimeHealthReporter,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
    child_tasks: &mut RouterSessionChildTasks,
    activation_state: &mut SessionActivationState,
    activation_prepare_tasks: &mut JoinSet<activation::ActivationPrepareTaskResult>,
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
                    "runtime.capabilities" | "runtime.health" | "runtime.register"
                )
            {
                let terminal = handshake.on_direction_violation(&frame_type);
                return Err(handshake_terminal_error(terminal));
            } else if frame_type != "runtime.registered" {
                if let Err(terminal) = handshake.on_business_frame() {
                    return Err(handshake_terminal_error(terminal));
                }
            }
            if frame_type == ASSEMBLY_ACTIVATION_FRAME_TYPE {
                if let Some(reply) = dispatch_session_activation_frame(
                    host,
                    &bytes,
                    bootstrap,
                    activation_state,
                    activation_prepare_tasks,
                )
                .await?
                {
                    send_writer_message(ws, reply).await?;
                }
            } else {
                dispatch_router_binary_frame_with_health(
                    host,
                    router_session_id,
                    &bytes,
                    sender,
                    health_reporter,
                    bootstrap,
                    handshake,
                    child_tasks,
                )
                .await?;
            }
        }
        Message::Ping(_) => {
            ws.flush().await.map_err(|error| {
                RuntimeError::Decode(format!("failed to flush Router ping reply: {error}"))
            })?;
        }
        Message::Pong(_) => {}
        Message::Close(_) => {
            ws.flush().await.map_err(|error| {
                RuntimeError::Decode(format!("failed to flush Router close reply: {error}"))
            })?;
            return Ok(false);
        }
        Message::Frame(_) => {}
    }
    Ok(true)
}

type RouterSessionChildTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[derive(Default)]
struct RouterSessionChildTasks {
    tasks: FuturesUnordered<RouterSessionChildTask>,
}

enum RouterSessionChildTaskDispatch<'a> {
    Owned(&'a mut RouterSessionChildTasks),
    #[cfg(test)]
    Detached,
}

impl RouterSessionChildTaskDispatch<'_> {
    fn submit(self, task: RouterSessionChildTask) {
        match self {
            Self::Owned(tasks) => tasks.push(task),
            #[cfg(test)]
            Self::Detached => {
                tokio::spawn(task);
            }
        }
    }
}

impl RouterSessionChildTasks {
    fn push(&mut self, task: RouterSessionChildTask) {
        self.tasks.push(Box::pin(async move {
            if AssertUnwindSafe(task).catch_unwind().await.is_err() {
                warn!(event = "runtime.router_session_child_panicked");
            }
        }));
    }

    fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    async fn next(&mut self) {
        let _ = self.tasks.next().await;
    }
}

struct ConnectedRouterSessionGuard {
    host: super::RuntimeHost,
    router_session_id: String,
    closed: bool,
}

impl ConnectedRouterSessionGuard {
    fn new(host: super::RuntimeHost, router_session_id: String) -> Self {
        Self {
            host,
            router_session_id,
            closed: false,
        }
    }

    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        if let Ok(session) = ConnectionRequestSession::new(self.router_session_id.clone()) {
            self.host.connection_requests.disconnect_session(&session);
        }
        self.host.outbound_requests.fail_all(ResponseError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
            status: None,
            details: None,
        });
        self.host.actor_method_outbound.fail_all(
            crate::capability_context::actor_method_outbound::ActorInvocationTransportError {
                code: "ConnectionClosed".to_string(),
                message: "router connection closed".to_string(),
            },
        );
        self.host
            .actor_owner_invocations
            .cancel_session(&self.router_session_id);
        // Fence the Actor connection generation first. This wakes session-owned
        // activation tasks and exact-discards provisional instances before any
        // later teardown step can expose a stale parent/test authority window.
        self.host
            .discard_actor_instances_for_session(&self.router_session_id);
        let test_disconnect_result = self
            .host
            .test_http_entries
            .disconnect_session(&self.router_session_id);
        let disconnect_result = self
            .host
            .websocket_generations
            .disconnect(&self.router_session_id);
        test_disconnect_result.and(disconnect_result)
    }
}

impl Drop for ConnectedRouterSessionGuard {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[derive(Clone)]
struct ConnectionBootstrap {
    resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver,
    config_snapshot_store: skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore,
    service_db: skiff_artifact_model::AssemblyActivationServiceDb,
    activation: skiff_runtime_transport::protocol::RouterBootstrapActivationFrameHeader,
    max_response_bytes: usize,
}

#[cfg(test)]
fn test_bootstrap_activation(
) -> skiff_runtime_transport::protocol::RouterBootstrapActivationFrameHeader {
    serde_json::from_value(serde_json::json!({
        "profile": "test",
        "generation": 0,
        "assembly": {
            "assemblyIdentity": format!(
                "skiff-runtime-assembly-v3:sha256:{}",
                "a".repeat(64)
            )
        },
        "configSnapshot": {
            "snapshotId": format!(
                "skiff-runtime-config-snapshot-v1:{}",
                "a".repeat(32)
            )
        }
    }))
    .expect("test bootstrap activation must decode")
}

#[cfg(test)]
fn test_connection_bootstrap(name: &str) -> Result<ConnectionBootstrap> {
    let artifact_path = std::env::temp_dir().join(format!(
        "skiff-runtime-test-artifacts-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&artifact_path)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    Ok(ConnectionBootstrap {
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        config_snapshot_store: skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
            artifact_path.join("runtime-config"),
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        activation: test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    })
}

fn decode_connection_bootstrap(
    typed: TypedEnvelope,
    payload: &[u8],
) -> Result<ConnectionBootstrap> {
    if !payload.is_empty() {
        return Err(RuntimeError::Decode(
            "router.bootstrap binary frame payload must be empty".to_string(),
        ));
    }
    let mut value = typed.rest;
    value.insert("type".to_string(), Value::String(typed.envelope_type));
    let header = decode_router_bootstrap_frame_header(Value::Object(value))
        .map_err(super::transport_error_into_runtime_error)?;
    let resolver = skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
        &header.artifacts_path,
    )
    .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let config_snapshot_store = skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::open(
        std::path::Path::new(&header.artifacts_path).join("runtime-config"),
    )
    .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let service_db = skiff_artifact_model::AssemblyActivationServiceDb {
        mongo_url: header.service_db.mongo_url.clone(),
    };
    Ok(ConnectionBootstrap {
        resolver,
        config_snapshot_store,
        service_db,
        activation: header.activation,
        max_response_bytes: usize::try_from(header.http.max_response_bytes).map_err(|_| {
            RuntimeError::Decode(
                "router.bootstrap http.maxResponseBytes exceeds Runtime address space".to_string(),
            )
        })?,
    })
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
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        config_snapshot_store: skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
            artifact_path.join("runtime-config"),
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        activation: test_bootstrap_activation(),
        max_response_bytes: 67_108_864,
    });
    let mut handshake = ClientHandshake::registered();
    dispatch_router_binary_frame_inner(
        host,
        "skiff-router-session-v1:opaque:test-session",
        bytes,
        sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
}

#[cfg(test)]
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
        resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(
            &artifact_path,
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        config_snapshot_store: skiff_runtime_config_snapshot::RuntimeConfigSnapshotStore::create(
            artifact_path.join("runtime-config"),
        )
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        activation: test_bootstrap_activation(),
        max_response_bytes,
    });
    let mut handshake = ClientHandshake::registered();
    dispatch_router_binary_frame_inner(
        host,
        "skiff-router-session-v1:opaque:test-session",
        bytes,
        sender,
        None,
        &mut bootstrap,
        &mut handshake,
        RouterSessionChildTaskDispatch::Detached,
    )
    .await
}

async fn dispatch_router_binary_frame_with_health(
    host: &super::RuntimeHost,
    router_session_id: &str,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    health_reporter: &mut RuntimeHealthReporter,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
    child_tasks: &mut RouterSessionChildTasks,
) -> Result<()> {
    dispatch_router_binary_frame_inner(
        host,
        router_session_id,
        bytes,
        sender,
        Some(health_reporter),
        bootstrap,
        handshake,
        RouterSessionChildTaskDispatch::Owned(child_tasks),
    )
    .await
}

async fn dispatch_router_binary_frame_inner(
    host: &super::RuntimeHost,
    router_session_id: &str,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    mut health_reporter: Option<&mut RuntimeHealthReporter>,
    bootstrap: &mut Option<ConnectionBootstrap>,
    handshake: &mut ClientHandshake,
    child_tasks: RouterSessionChildTaskDispatch<'_>,
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
            let installed = decode_connection_bootstrap(typed, &payload)?;
            host.recover_durable_committed(
                &installed.activation.profile,
                installed.activation.generation,
                &installed.activation.assembly,
                &installed.activation.config_snapshot,
                &installed.resolver,
                &installed.config_snapshot_store,
                &installed.service_db,
            )
            .await?;
            host.queue_connection_registration(sender.clone())?;
            handshake.mark_registration_queued();
            *bootstrap = Some(installed);
        }
        ASSEMBLY_ACTIVATION_FRAME_TYPE => {
            let bootstrap = bootstrap.as_ref().ok_or_else(|| {
                RuntimeError::Decode(
                    "assembly activation requires router.bootstrap first".to_string(),
                )
            })?;
            let control = decode_assembly_activation_frame(
                AssemblyActivationFrameDirection::RouterToRuntime,
                bytes,
            )
            .map_err(super::transport_error_into_runtime_error)?;
            if let Some(reply) = host
                .apply_bootstrapped_assembly_activation_control(
                    control,
                    &bootstrap.resolver,
                    &bootstrap.config_snapshot_store,
                    Some(&bootstrap.service_db),
                )
                .await
                .map_err(|error| RuntimeError::Decode(error.to_string()))?
            {
                super::RuntimeHost::queue_assembly_activation(sender.clone(), &reply)?;
            }
        }
        WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE => {
            host.websocket_generations
                .dispatch_router_control(router_session_id, bytes, sender)?;
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
            if let Some(health_reporter) = health_reporter.as_deref_mut() {
                health_reporter
                    .record_registered(host, sender, runtime_id)
                    .await?;
            }
        }
        "router.control" => {
            return Err(RuntimeError::Decode(
                "router.control artifactRoots/serviceConfig reload is not supported; use exact assembly activation control"
                    .to_string(),
            ));
        }
        "request.start" => {
            if bootstrap.is_none() {
                return Err(RuntimeError::Decode(
                    "request.start requires router.bootstrap first".to_string(),
                ));
            }
            let (header, payload) = decode_runtime_assembly_request_start_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            let bootstrap = bootstrap.as_ref().expect("bootstrap checked above");
            host.spawn_runtime_assembly_request(
                router_session_id,
                header,
                payload,
                bootstrap.max_response_bytes,
                sender.clone(),
            )
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
            host.cancel_request(request_cancel_from_frame_header(header))
                .await;
        }
        "connection.response" => {
            let (header, payload) = decode_connection_response_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            let request_id = header.request_id.clone();
            let session = ConnectionRequestSession::new(router_session_id.to_string())
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
                    router_session_id
                );
            }
        }
        ACTOR_OWNER_INVOKE_FRAME_TYPE => {
            if bootstrap.is_none() {
                return Err(RuntimeError::Decode(
                    "actor.owner.invoke requires router.bootstrap first".to_string(),
                ));
            }
            let (header, arguments_payload) = decode_actor_owner_invoke_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if header.target_runtime_id != host.base_runtime_id {
                return Err(RuntimeError::Decode(
                    "actor.owner.invoke targets a different Runtime".to_string(),
                ));
            }
            let task = host.begin_actor_owner_invoke(
                router_session_id.to_string(),
                header,
                arguments_payload,
                sender.clone(),
            )?;
            child_tasks.submit(task);
        }
        ACTOR_OWNER_CONTROL_FRAME_TYPE => {
            let task = begin_actor_owner_control(host, router_session_id, bytes, sender)?;
            child_tasks.submit(task);
        }
        ACTOR_OWNER_FAILURE_FRAME_TYPE => {
            let failure = decode_actor_owner_failure_frame(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if !host.actor_method_outbound.complete_failure(
                &failure.invocation_id,
                failure.epoch,
                &failure.actor_implementation_identity,
                crate::capability_context::actor_method_outbound::ActorInvocationTransportError {
                    code: failure.reason.code,
                    message: failure.reason.message,
                },
            ) {
                warn!(
                    event = "runtime.unmatched_actor_owner_failure",
                    invocation_id = %failure.invocation_id
                );
            }
        }
        "actor.method.return" | "actor.method.error" | "actor.method.cancel" => {
            dispatch_actor_method_terminal(host, router_session_id, bytes)?;
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

fn dispatch_actor_method_terminal(
    host: &super::RuntimeHost,
    router_session_id: &str,
    bytes: &[u8],
) -> Result<()> {
    let (invocation_id, outcome) = match decode_actor_method_frame(bytes)
        .map_err(super::transport_error_into_runtime_error)?
    {
        ActorMethodFrame::Return(header, payload) => (
            header.invocation_id,
            ActorInvocationOutcome::Returned(payload),
        ),
        ActorMethodFrame::Error(header) => {
            let outcome = match header.error {
                ActorMethodErrorFramePayload::ActorUpgradingError { retry_after_ms, .. } => {
                    ActorInvocationOutcome::ActorError(ActorInvocationError::ActorUpgrading {
                        retry_after_ms,
                    })
                }
                ActorMethodErrorFramePayload::ActorVersionRejectedError {
                    requested_implementation_identity,
                    accepted_implementation_identity,
                    ..
                } => {
                    ActorInvocationOutcome::ActorError(ActorInvocationError::ActorVersionRejected {
                        requested: requested_implementation_identity,
                        accepted: accepted_implementation_identity,
                    })
                }
                ActorMethodErrorFramePayload::ActorIncarnationReplacedError {
                    actor_ref,
                    current_epoch,
                } => ActorInvocationOutcome::ActorError(
                    ActorInvocationError::ActorIncarnationReplaced {
                        requested_epoch: actor_ref.epoch,
                        current_epoch,
                    },
                ),
            };
            (header.invocation_id, outcome)
        }
        ActorMethodFrame::Cancel(header) => {
            let expected = host
                .actor_method_outbound
                .cancellation_correlation(&header.invocation_id);
            if expected.is_none() {
                host.actor_owner_invocations.cancel_for_session(
                    &header.invocation_id,
                    router_session_id,
                    &header.cancellation_correlation,
                    header.reason.into(),
                );
                return Ok(());
            }
            if expected.as_deref() != Some(header.cancellation_correlation.as_str()) {
                return Ok(());
            }
            let reason = match header.reason {
                skiff_runtime_transport::actor_method::ActorMethodCancelReason::Cancelled => {
                    ActorInvocationCancellation::Cancelled
                }
                skiff_runtime_transport::actor_method::ActorMethodCancelReason::DeadlineExceeded => {
                    ActorInvocationCancellation::DeadlineExceeded
                }
            };
            (
                header.invocation_id,
                ActorInvocationOutcome::Cancelled(reason),
            )
        }
        ActorMethodFrame::Invoke(_, _) => {
            return Err(RuntimeError::Decode(
                "public actor.method.invoke is not a terminal frame".to_string(),
            ))
        }
    };
    if !host.actor_method_outbound.complete(&invocation_id, outcome) {
        warn!(
            event = "runtime.unmatched_actor_method_terminal",
            invocation_id = %invocation_id
        );
    }
    Ok(())
}

fn begin_actor_owner_control(
    host: &super::RuntimeHost,
    router_session_id: &str,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
) -> Result<RouterSessionChildTask> {
    let control = decode_actor_owner_control_frame(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    if control.target_runtime_id != host.base_runtime_id {
        return Err(RuntimeError::Decode(
            "actor.owner.control targets a different Runtime".to_string(),
        ));
    }
    let session_lease = host
        .actor_instance_session_lease(router_session_id)
        .map_err(|error| RuntimeError::Decode(error.to_string()))?;
    // Test-aware create admission is synchronous with the Router frame. This binds the
    // authenticated parent on this exact connection before the session-owned activation can be
    // cancelled, disconnected, or race root finalization.
    let test_effect_execution = match (
        control.test_case_capability.as_deref(),
        control.test_case_parent_request_id.as_deref(),
    ) {
        (Some(capability), Some(parent_request_id))
            if control.operation == ActorOwnerControlOperation::ActivateInitial =>
        {
            Some(host.test_http_entries.begin_actor_method(
                capability,
                parent_request_id,
                router_session_id,
                control.request_id.clone(),
            )?)
        }
        (None, None) => None,
        _ => {
            return Err(RuntimeError::Decode(
                "Actor initial activation test capability and parent request id must be present together"
                    .to_string(),
            ))
        }
    };
    let host = host.clone();
    let router_session_id = router_session_id.to_string();
    let sender = sender.clone();
    Ok(Box::pin(async move {
        let accepted = {
            let activation =
                async {
                    match control.operation {
                ActorOwnerControlOperation::MarkUpgrading => {
                    if super::actor_owner_execution::control_instance_fence(&control).is_ok_and(
                        |fence| host.begin_actor_upgrade_exact(&router_session_id, &fence),
                    ) {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Accepted
                    } else {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(None)
                    }
                }
                ActorOwnerControlOperation::Discard => {
                    if super::actor_owner_execution::control_instance_fence(&control).is_ok_and(
                        |fence| host.discard_upgrading_actor_exact(&router_session_id, &fence),
                    ) {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Accepted
                    } else {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(None)
                    }
                }
                ActorOwnerControlOperation::IdleEvict => {
                    if super::actor_owner_execution::control_instance_fence(&control)
                        .is_ok_and(|fence| host.discard_actor_exact(&router_session_id, &fence))
                    {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Accepted
                    } else {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(None)
                    }
                }
                ActorOwnerControlOperation::Activate => {
                    if host
                        .activate_actor_owner_control(&session_lease, &control, &sender)
                        .await
                    {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Accepted
                    } else {
                        super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(None)
                    }
                }
                ActorOwnerControlOperation::ActivateInitial => {
                    host.activate_actor_owner_initial(
                        &session_lease,
                        &control,
                        &sender,
                        test_effect_execution.as_ref().map(
                            crate::capability_context::ActorMethodTestEffectExecution::context,
                        ),
                    )
                    .await
                }
                }
                };
            tokio::pin!(activation);
            tokio::select! {
                biased;
                _ = session_lease.wait_closed() => {
                    super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(Some(
                        super::actor_owner_execution::control_reason(
                            "ConnectionClosed",
                            "Router session closed during Actor owner control",
                        ),
                    ))
                }
                accepted = &mut activation => accepted,
            }
        };
        let (accepted, reason) = match accepted {
            super::actor_owner_execution::ActorOwnerControlAcceptance::Accepted => (true, None),
            super::actor_owner_execution::ActorOwnerControlAcceptance::Rejected(reason) => {
                (false, reason)
            }
        };
        if let Some(execution) = test_effect_execution.as_ref() {
            execution.revoke_exact();
        }
        let ack = ActorOwnerControlAckFrameHeader {
            schema_version: skiff_runtime_transport::protocol::RUNTIME_FRAME_SCHEMA_VERSION.into(),
            envelope_type: ACTOR_OWNER_CONTROL_ACK_FRAME_TYPE.into(),
            runtime_id: host.base_runtime_id.clone(),
            request_id: control.request_id,
            operation: control.operation,
            accepted,
            reason,
        };
        if let Ok(frame) = encode_actor_owner_control_ack_frame(&ack) {
            let _ = sender.send(super::RouterWriterMessage::Binary(frame));
        }
        // Retain test ownership through ACK encode/send, including rejection and closed-writer
        // tails, so the root cannot finalize while create still appears active to the Router.
        drop(test_effect_execution);
    }))
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

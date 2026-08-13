//! Per-connection session task (C-session §3/§5, C-model-registration §2).
//!
//! One task owns one physical connection: the socket read half, the bounded
//! outbound queue with its writer task, the handshake phase machine, the
//! bound `RuntimeSessionEpoch`, and the close protocol (cancellation token ->
//! reserved terminal delivery -> ACK barrier -> directory deletion ->
//! pre-auth release -> socket abort). Cancellation is observed independently
//! of ordinary frame dequeue.

use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use hyper_util::rt::TokioIo;
use skiff_runtime_transport::protocol::{
    RuntimeCapabilitiesFrameHeader, RuntimeDispatchModeCapability,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::{sleep_until, timeout, Instant};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::routing::DispatchCapabilities;

use super::budget::{OutboundFrameId, OutboundQueue, QueuedFrame, WriterError};
use super::demux::{DemuxEvent, DemuxOutcome};
use super::handshake::{
    CapabilitiesEvent, HandshakePhase, HandshakeState, HealthEvent, TerminalKind, TimeoutKind,
};
use super::identity::{RuntimeConnectionEpoch, RuntimeSessionEpoch};
use super::layer::{
    SessionCloseReason, SessionFrameWriter, SessionLayer, SessionRegistrationFacts,
};

pub type RuntimeSocket = WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>;
pub type RuntimeSocketRead = SplitStream<RuntimeSocket>;
pub type RuntimeSocketWrite = SplitSink<RuntimeSocket, Message>;

/// Completes when the optional oneshot fires; never completes when `None`
/// (used to keep one select branch alive for an absent pending write).
async fn completed(receiver: &mut Option<oneshot::Receiver<()>>) -> bool {
    if let Some(receiver) = receiver {
        let _ = receiver.await;
        true
    } else {
        let () = std::future::pending().await;
        unreachable!("pending never completes")
    }
}

fn debug_frame_type(raw: &[u8]) -> String {
    skiff_runtime_transport::protocol::decode_binary_frame(raw)
        .ok()
        .and_then(|frame| {
            frame
                .header
                .get("type")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "<undecodable>".to_string())
}

pub(crate) async fn run_session_task(
    layer: Arc<SessionLayer>,
    connection_epoch: RuntimeConnectionEpoch,
    socket: RuntimeSocket,
    mut shutdown_rx: watch::Receiver<()>,
    mut cancel_rx: watch::Receiver<Option<SessionCloseReason>>,
) {
    let session_debug = std::env::var("SKIFF_ROUTER_SESSION_DEBUG").is_ok();
    let connection_id = connection_epoch.opaque_connection_id.clone();
    let debug = |message: String| {
        if session_debug {
            eprintln!("[session-debug] {connection_id}: {message}");
        }
    };
    let (write_half, mut read_half) = socket.split();
    let (outbound, outbound_rx) = OutboundQueue::new(layer.budgets);
    let (writer_error_tx, mut writer_error_rx) = mpsc::channel::<WriterError>(1);
    let (writer_closed_tx, writer_closed_rx) = oneshot::channel();
    let mut writer_closed_rx = Some(writer_closed_rx);
    let (writer_close_tx, writer_close_rx) = watch::channel(false);
    let writer_handle = tokio::spawn(run_outbound_writer(
        write_half,
        outbound_rx,
        outbound.clone(),
        writer_error_tx,
        writer_closed_tx,
        layer.writer_delay,
        writer_close_rx,
    ));

    let mut machine = HandshakeState::new();
    let mut bound_session: Option<RuntimeSessionEpoch> = None;
    let mut pre_auth_released = false;
    let mut close_reason: Option<SessionCloseReason> = None;
    let mut phase_started_at = Instant::now();
    let mut ack_started_at: Option<Instant> = None;
    let mut bootstrap_wait: Option<oneshot::Receiver<()>> = None;
    let mut ack_wait: Option<oneshot::Receiver<()>> = None;

    // Issue `router.bootstrap` (M4: built from the frozen config; no epoch
    // tuple). Without it the connection stays in Accepted and fails at the
    // bootstrap deadline (fail-closed).
    if let Some(bootstrap) = layer.bootstrap_bytes() {
        let (written_tx, written_rx) = oneshot::channel();
        match outbound.try_send(OutboundFrameId::Bootstrap, bootstrap, Some(written_tx)) {
            Ok(()) => bootstrap_wait = Some(written_rx),
            Err(_) => {
                machine.on_bootstrap_write_failed();
            }
        }
    }

    loop {
        if machine.is_closed() {
            break;
        }
        let deadline = match machine.phase() {
            HandshakePhase::Accepted => Some(phase_started_at + layer.timing.bootstrap),
            HandshakePhase::BootstrapSent => Some(phase_started_at + layer.timing.capabilities),
            HandshakePhase::CapabilitiesBound => {
                ack_started_at.map(|started| started + layer.timing.ack_write)
            }
            HandshakePhase::Registered | HandshakePhase::Closed => None,
        };

        tokio::select! {
            frame = read_half.next() => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => {
                        let outcome = layer.demux().classify_with_sinks(&bytes, &layer.inbound_sinks());
                        match outcome {
                            DemuxOutcome::Terminal(kind) => {
                                debug(format!(
                                    "inbound frame terminal {}: {:?}",
                                    debug_frame_type(&bytes),
                                    kind
                                ));
                                machine.terminal_with(kind);
                            }
                            DemuxOutcome::Handled(event) => {
                                process_event(
                                    &layer,
                                    &connection_epoch,
                                    &mut machine,
                                    &mut bound_session,
                                    &mut phase_started_at,
                                    &mut ack_started_at,
                                    &mut ack_wait,
                                    &outbound,
                                    event,
                                );
                            }
                        }
                    }
                    Some(Ok(Message::Text(_))) => {
                        debug("inbound text frame terminal MalformedFrame".to_string());
                        machine.terminal_with(TerminalKind::MalformedFrame);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        debug("inbound Close/EOF on_disconnect".to_string());
                        machine.on_disconnect();
                    }
                    Some(Ok(Message::Ping(_)))
                    | Some(Ok(Message::Pong(_)))
                    | Some(Ok(Message::Frame(_))) => {}
                    Some(Err(_)) => {
                        debug("inbound read error on_disconnect".to_string());
                        machine.on_disconnect();
                    }
                }
            }
            _ = cancel_rx.changed() => {
                if let Some(reason) = *cancel_rx.borrow() {
                    debug(format!("cancel_rx request_close: {reason:?}"));
                    close_reason = Some(reason);
                    break;
                }
            }
            _ = shutdown_rx.changed() => {
                debug("shutdown_rx close".to_string());
                close_reason = Some(SessionCloseReason::Shutdown);
                break;
            }
                            error = writer_error_rx.recv() => {
                                match error {
                                    Some(error) => {
                                        debug(format!("writer error {:?}: {error:?}", error.frame_id));
                                        match error.frame_id {
                                            OutboundFrameId::Bootstrap => {
                                                machine.on_bootstrap_write_failed();
                                            }
                                            OutboundFrameId::RegisteredAck => {
                                                machine.on_ack_write_failed();
                                            }
                                            OutboundFrameId::Close | OutboundFrameId::Business => {
                                                machine.on_disconnect();
                                            }
                                        }
                    }
                    None => {
                        machine.on_disconnect();
                    }
                }
            }
            _ = completed(&mut bootstrap_wait) => {
                bootstrap_wait = None;
                if let Err(kind) = machine.on_bootstrap_written() {
                    machine.terminal_with(kind);
                } else {
                    phase_started_at = Instant::now();
                }
            }
            _ = completed(&mut ack_wait) => {
                ack_wait = None;
                if let Err(kind) = machine.on_ack_written() {
                    machine.terminal_with(kind);
                } else {
                    if let Some(session) = &bound_session {
                        if layer.directory_lock().mark_registered(session) {
                            // Cold recovery rebind seam (plan §4.2): the
                            // activation coordinator observes routable
                            // registrations to bind expected replicas.
                            layer.notify_session_registered(session);
                        }
                    }
                    layer.release_pre_auth(&connection_id);
                    pre_auth_released = true;
                }
            }
            _ = sleep_until(deadline.unwrap_or_else(|| Instant::now() + layer.timing.ack_write)),
                if deadline.is_some() =>
            {
                if machine.phase() == HandshakePhase::CapabilitiesBound {
                    debug(format!(
                        "ack deadline expired ({}ms)",
                        layer.timing.ack_write.as_millis()
                    ));
                    machine.on_ack_write_failed();
                } else {
                    let kind = match machine.phase() {
                        HandshakePhase::Accepted => TimeoutKind::Bootstrap,
                        HandshakePhase::BootstrapSent => TimeoutKind::Capabilities,
                        _ => TimeoutKind::Bootstrap,
                    };
                    debug(format!("handshake deadline expired: {kind:?}"));
                    machine.on_timeout(kind);
                }
            }
        }
    }

    debug(format!(
        "session closing phase={:?} terminal={:?} close_reason={:?}",
        machine.phase(),
        machine.terminal(),
        close_reason
    ));
    if let Some(session) = &bound_session {
        layer.unregister_frame_writer(session);
        layer.remove_registration_facts(session);
    }
    close_session(
        &layer,
        &connection_id,
        &mut machine,
        &bound_session,
        &mut pre_auth_released,
        close_reason.unwrap_or(SessionCloseReason::Disconnect),
    )
    .await;
    // Graceful close reply through the independent writer close signal:
    // pending frames are abandoned (writer-queue-full semantics), the writer
    // sends the close frame immediately, and the session aborts it if it does
    // not drain (C-session §3.6/§5.3).
    let _ = writer_close_tx.send(true);
    if let Some(receiver) = writer_closed_rx.as_mut() {
        let _ = timeout(Duration::from_millis(500), receiver).await;
    }
    writer_handle.abort();
    layer.task_finished(&connection_id);
}

#[allow(clippy::too_many_arguments)] // bounded per-connection task plumbing
fn process_event(
    layer: &Arc<SessionLayer>,
    connection_epoch: &RuntimeConnectionEpoch,
    machine: &mut HandshakeState,
    bound_session: &mut Option<RuntimeSessionEpoch>,
    phase_started_at: &mut Instant,
    ack_started_at: &mut Option<Instant>,
    ack_wait: &mut Option<oneshot::Receiver<()>>,
    outbound: &OutboundQueue,
    event: DemuxEvent,
) {
    match event {
        DemuxEvent::Capabilities(header) => match machine.on_capabilities(&header.runtime_id) {
            CapabilitiesEvent::Bound => {
                let session = RuntimeSessionEpoch {
                    replica_id: header.runtime_id.clone(),
                    connection_generation: connection_epoch.generation,
                };
                layer.bind_session(&connection_epoch.opaque_connection_id, session.clone());
                layer.register_frame_writer(
                    &session,
                    Arc::new(QueueFrameWriter {
                        outbound: outbound.clone(),
                    }),
                );
                layer.record_registration_facts(&session, registration_facts(&header));
                // M4: capabilities are the registration. Publish the pending
                // record and write the `runtime.registered` ACK immediately;
                // the session becomes routable once the ACK is written.
                let permits = layer.manifest_kinds();
                let output = layer.registration_sink().handle_capabilities(
                    machine,
                    &mut layer.directory_lock(),
                    &session,
                    &permits,
                );
                match output {
                    super::demux::RegistrationSinkOutput::PendingPublished {
                        cancelled_old,
                        ..
                    } => {
                        if let Some(old) = cancelled_old {
                            layer.request_close(&old, SessionCloseReason::Replaced);
                        }
                        layer.sync_registration_facts(&session);
                        start_ack_write(
                            layer,
                            machine,
                            &session,
                            ack_started_at,
                            ack_wait,
                            outbound,
                        );
                    }
                    super::demux::RegistrationSinkOutput::Idempotent => {}
                    super::demux::RegistrationSinkOutput::Terminal(_) => {}
                }
                *bound_session = Some(session);
                *phase_started_at = Instant::now();
            }
            CapabilitiesEvent::Refreshed => {
                let Some(session) = bound_session.clone() else {
                    machine.terminal_with(TerminalKind::WrongOrder);
                    return;
                };
                layer.record_registration_facts(&session, registration_facts(&header));
            }
            CapabilitiesEvent::Terminal(_) => {}
        },
        DemuxEvent::Health(header) => match machine.on_health(&header.runtime_id) {
            HealthEvent::Observed => {
                if let Some(session) = bound_session {
                    layer.health().record_observation(session, header);
                }
            }
            HealthEvent::Terminal(_) => {}
        },
        DemuxEvent::Sink { family, raw } => {
            let debug = |message: String| {
                if std::env::var("SKIFF_ROUTER_SESSION_DEBUG").is_ok() {
                    eprintln!("[session-debug] sink family={family:?}: {message}");
                }
            };
            let Some(session) = bound_session.clone() else {
                machine.terminal_with(TerminalKind::WrongOrder);
                return;
            };
            let sinks = layer.inbound_sinks();
            match sinks.sink_for(family) {
                Some(sink) => {
                    if let Err(kind) = sink.handle(&session, &raw) {
                        debug(format!(
                            "sink {:?} rejected frame {}: {:?}",
                            family,
                            debug_frame_type(&raw),
                            kind
                        ));
                        machine.terminal_with(kind);
                    }
                }
                None => {
                    debug(format!("no sink for frame {}", debug_frame_type(&raw)));
                    machine.terminal_with(TerminalKind::UnimplementedFamily);
                }
            }
        }
        DemuxEvent::Unimplemented { .. } => {
            if std::env::var("SKIFF_ROUTER_SESSION_DEBUG").is_ok() {
                eprintln!("[session-debug] demux unimplemented family terminal");
            }
            machine.terminal_with(TerminalKind::UnimplementedFamily);
        }
    }
}

/// Projects the validated `runtime.capabilities` header onto the per-session
/// registration facts (integration-contract-v2 §1/§3): dispatch modes plus
/// the loaded build-id set and the lazy-load advertisement.
fn registration_facts(header: &RuntimeCapabilitiesFrameHeader) -> SessionRegistrationFacts {
    SessionRegistrationFacts {
        dispatch: DispatchCapabilities {
            unary: header
                .capabilities
                .dispatch_modes
                .iter()
                .any(|mode| matches!(mode, RuntimeDispatchModeCapability::Unary)),
            server_stream: header
                .capabilities
                .dispatch_modes
                .iter()
                .any(|mode| matches!(mode, RuntimeDispatchModeCapability::ServerStream)),
        },
        registration: super::directory::RegistrationFacts {
            registered_build_ids: header.capabilities.loaded_build_ids.clone(),
            lazy_load: header.capabilities.lazy_load,
            artifact_root: header.capabilities.artifact_root.clone(),
        },
    }
}

/// Queue-backed [`SessionFrameWriter`] registered by the session task. The
/// `OutboundQueue` remains owned by this task; lane ports only enqueue
/// bounded, non-blocking frames through the layer registry.
#[derive(Debug, Clone)]
struct QueueFrameWriter {
    outbound: OutboundQueue,
}

impl SessionFrameWriter for QueueFrameWriter {
    fn enqueue(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.outbound
            .try_send(OutboundFrameId::Business, bytes, None)
            .map_err(|_| "session outbound queue full".to_string())
    }
}

fn start_ack_write(
    layer: &SessionLayer,
    machine: &mut HandshakeState,
    session: &RuntimeSessionEpoch,
    ack_started_at: &mut Option<Instant>,
    ack_wait: &mut Option<oneshot::Receiver<()>>,
    outbound: &OutboundQueue,
) {
    let Some(bytes) = layer.registered_ack_bytes(&session.replica_id) else {
        machine.on_ack_write_failed();
        return;
    };
    let (written_tx, written_rx) = oneshot::channel();
    match outbound.try_send(OutboundFrameId::RegisteredAck, bytes, Some(written_tx)) {
        Ok(()) => {
            *ack_wait = Some(written_rx);
            *ack_started_at = Some(Instant::now());
        }
        Err(_) => {
            machine.on_ack_write_failed();
        }
    }
}

async fn close_session(
    layer: &SessionLayer,
    connection_id: &str,
    machine: &mut HandshakeState,
    bound_session: &Option<RuntimeSessionEpoch>,
    pre_auth_released: &mut bool,
    reason: SessionCloseReason,
) {
    if !machine.is_closed() {
        machine.terminal_with(TerminalKind::Disconnect);
    }
    let _ = reason;
    if let Some(session) = bound_session {
        let close_start = layer.directory_lock().begin_close(session);
        if let Some(close_start) = close_start {
            let mut acks = Vec::with_capacity(close_start.permits.len());
            for consumer in &close_start.permits {
                match layer.deliver_terminal(*consumer, session) {
                    Ok(ack_rx) => acks.push((*consumer, ack_rx)),
                    Err(error) => {
                        layer.fail_stop(format!("reserved terminal slot failed: {error:?}"));
                        acks.clear();
                        break;
                    }
                }
            }
            if !acks.is_empty() {
                let barrier = timeout(layer.timing.close_barrier, async {
                    let results = futures_util::future::join_all(
                        acks.into_iter()
                            .map(|(consumer, ack)| async move { (consumer, ack.await) }),
                    )
                    .await;
                    results
                })
                .await;
                match barrier {
                    Ok(results) => {
                        let mut fail_stop: Option<String> = None;
                        for (consumer, ack) in results {
                            match ack {
                                Ok(Ok(())) => {
                                    let _ = layer.directory_lock().ack_close(session, consumer);
                                }
                                Ok(Err(error)) => {
                                    fail_stop =
                                        Some(format!("consumer {consumer:?} ACK failed: {error}"));
                                }
                                Err(_) => {
                                    fail_stop =
                                        Some(format!("consumer {consumer:?} ACK channel lost"));
                                }
                            }
                        }
                        if layer.directory_lock().record(session).is_some() {
                            fail_stop = Some(format!(
                                "close barrier for {session:?} did not delete the exact session after all ACKs"
                            ));
                        }
                        if let Some(reason) = fail_stop {
                            layer.fail_stop(reason);
                        }
                    }
                    Err(_) => {
                        layer.fail_stop(format!(
                            "close barrier ACK timeout for session {session:?}"
                        ));
                    }
                }
            }
        }
    }
    if !*pre_auth_released {
        layer.release_pre_auth(connection_id);
        *pre_auth_released = true;
    }
}

pub(crate) async fn run_outbound_writer(
    mut write_half: RuntimeSocketWrite,
    mut receiver: mpsc::Receiver<QueuedFrame>,
    queue: OutboundQueue,
    error_tx: mpsc::Sender<WriterError>,
    writer_closed_tx: oneshot::Sender<()>,
    writer_delay: Option<Duration>,
    mut close_rx: watch::Receiver<bool>,
) {
    loop {
        let frame = tokio::select! {
            frame = receiver.recv() => match frame {
                Some(frame) => frame,
                None => break,
            },
            _ = close_rx.changed() => {
                let _ = write_half.send(Message::Close(None)).await;
                let _ = write_half.flush().await;
                break;
            }
        };
        if let Some(delay) = writer_delay {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = close_rx.changed() => {
                    let _ = write_half.send(Message::Close(None)).await;
                    let _ = write_half.flush().await;
                    break;
                }
            }
        }
        if frame.id == OutboundFrameId::Close {
            let _ = write_half.send(Message::Close(None)).await;
            let _ = write_half.flush().await;
            break;
        }
        let result = match write_half
            .send(Message::Binary(frame.bytes.clone().into()))
            .await
        {
            Ok(()) => write_half.flush().await,
            Err(error) => Err(error),
        };
        queue.mark_written(frame.bytes.len());
        match result {
            Ok(()) => {
                if let Some(written_tx) = frame.written_tx {
                    let _ = written_tx.send(());
                }
            }
            Err(error) => {
                let _ = error_tx
                    .send(WriterError {
                        frame_id: frame.id,
                        message: error.to_string(),
                    })
                    .await;
                break;
            }
        }
    }
    let _ = writer_closed_tx.send(());
}

use std::collections::HashSet;

use futures_util::{Sink, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use skiff_runtime_request::{OutboundResponse, ResponseError};
use skiff_runtime_transport::{
    control_mapper::encode_outbound_control_message,
    control_response_mapper::spawn_claim_response_control_payload,
    protocol::{
        decode_typed_binary_frame, ActorFindResponseFrameHeader, ActorPutResponseFrameHeader,
        ActorRemoveResponseFrameHeader, ActorSpawnRuntimeErrorFrameHeader,
        PackageTestStartFrameHeader, RequestCancelFrameHeader, RequestStartFrameHeader,
        ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseErrorFrameHeader,
        ResponseStartFrameHeader, RouterControlEnvelope, RouterControlFrameHeader,
        RuntimeErrorFramePayload, RuntimeRegisteredFrameHeader, SpawnClaimResponseFrameHeader,
        SpawnCompleteResponseFrameHeader, SpawnFailResponseFrameHeader,
        SpawnRenewResponseFrameHeader, SpawnSubmitResponseFrameHeader, TypedEnvelope,
    },
    request_mapper::{request_cancel_from_frame_header, request_envelope_from_start_frame},
    response_mapper::{
        response_chunk_to_outbound, response_end_to_outbound, response_error_to_outbound,
        response_start_to_outbound,
    },
};
use tokio::{
    sync::mpsc,
    time::{sleep, Duration, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::{
    error::{Result, RuntimeError},
    loader::artifact_roots_control_fingerprint,
};

pub(super) async fn run_reconnect_loop(host: super::RuntimeHost) -> Result<()> {
    let mut backoff = Duration::from_millis(250);
    loop {
        match run_once(host.clone()).await {
            Ok(()) => {
                backoff = Duration::from_millis(250);
                warn!(
                    event = "runtime.router_disconnected",
                    reconnect_in_ms = backoff.as_millis() as u64
                );
            }
            Err(error) => {
                warn!(
                    event = "runtime.router_connection_error",
                    error = %error,
                    reconnect_in_ms = backoff.as_millis() as u64
                );
            }
        }
        sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(5));
    }
}

pub(super) async fn run_once(host: super::RuntimeHost) -> Result<()> {
    let (ws, _) = connect_async(&host.router_url)
        .await
        .map_err(|error| RuntimeError::Decode(format!("failed to connect router: {error}")))?;
    info!(
        event = "runtime.router_connected",
        router = %host.router_url
    );
    let (writer, mut reader) = ws.split();
    let (sender, receiver) = mpsc::unbounded_channel::<super::RouterWriterMessage>();

    host.queue_registers(sender.clone())?;
    super::spawn_worker::start_spawn_workers(host.clone(), sender.clone());

    let writer_task = tokio::spawn(run_writer_loop(writer, receiver));

    let mut control: Option<RouterControlEnvelope> = None;
    let mut artifact_fingerprint: Option<String> = None;
    let mut registered_runtime_ids = HashSet::<String>::new();
    let mut reload_interval = tokio::time::interval(Duration::from_secs(1));
    reload_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut health_interval = tokio::time::interval(Duration::from_secs(1));
    health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            message = reader.next() => {
                let Some(message) = message else {
                    break;
                };
                let message = message
                    .map_err(|error| RuntimeError::Decode(format!("router read failed: {error}")))?;
                match message {
                    Message::Text(text) => {
                        reject_router_text_message(text.as_str())?;
                    }
                    Message::Binary(bytes) => {
                        dispatch_router_binary_frame_with_health(
                            &host,
                            &bytes,
                            &sender,
                            &mut control,
                            &mut artifact_fingerprint,
                            &mut registered_runtime_ids,
                        )
                        .await?;
                    }
                    _ => {}
                }
            }
            _ = reload_interval.tick(), if control.as_ref().is_some_and(|control| control.dev_reload.unwrap_or(false)) => {
                let control_ref = control.as_ref().expect("control should be present");
                maybe_reload_dev_artifacts(&host, &sender, control_ref, &mut artifact_fingerprint)
                    .await;
            }
            _ = health_interval.tick(), if !registered_runtime_ids.is_empty() => {
                for runtime_id in registered_runtime_ids.iter() {
                    host.queue_runtime_health(&sender, runtime_id).await?;
                }
            }
        }
    }

    drop(sender);
    let _ = writer_task.await;
    Ok(())
}

#[cfg(test)]
async fn dispatch_router_binary_frame(
    host: &super::RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &mut Option<RouterControlEnvelope>,
    artifact_fingerprint: &mut Option<String>,
) -> Result<()> {
    dispatch_router_binary_frame_inner(
        host,
        bytes,
        sender,
        control,
        artifact_fingerprint,
        None,
    )
    .await
}

async fn dispatch_router_binary_frame_with_health(
    host: &super::RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &mut Option<RouterControlEnvelope>,
    artifact_fingerprint: &mut Option<String>,
    registered_runtime_ids: &mut HashSet<String>,
) -> Result<()> {
    dispatch_router_binary_frame_inner(
        host,
        bytes,
        sender,
        control,
        artifact_fingerprint,
        Some(registered_runtime_ids),
    )
    .await
}

async fn dispatch_router_binary_frame_inner(
    host: &super::RuntimeHost,
    bytes: &[u8],
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &mut Option<RouterControlEnvelope>,
    artifact_fingerprint: &mut Option<String>,
    mut registered_runtime_ids: Option<&mut HashSet<String>>,
) -> Result<()> {
    let (typed, payload) = decode_typed_binary_frame::<TypedEnvelope>(bytes)
        .map_err(super::transport_error_into_runtime_error)?;
    match typed.envelope_type.as_str() {
        "runtime.registered" => {
            let (header, payload) =
                decode_typed_binary_frame::<RuntimeRegisteredFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "runtime.registered binary frame payload must be empty".to_string(),
                ));
            }
            let mut rest = serde_json::Map::new();
            rest.insert("runtimeId".to_string(), Value::String(header.runtime_id));
            host.log_registered(&rest);
            let runtime_id = rest
                .get("runtimeId")
                .and_then(Value::as_str)
                .expect("runtimeId should be set")
                .to_string();
            if let Some(registered_runtime_ids) = registered_runtime_ids.as_deref_mut() {
                registered_runtime_ids.insert(runtime_id.clone());
                host.queue_runtime_health(sender, &runtime_id).await?;
            }
        }
        "router.control" => {
            let (header, payload) = decode_typed_binary_frame::<RouterControlFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "router.control binary frame payload must be empty".to_string(),
                ));
            }
            handle_router_control(
                host,
                router_control_typed_envelope_from_frame_header(header),
                sender,
                control,
                artifact_fingerprint,
            )
            .await?;
        }
        "request.start" => {
            let (header, payload) = decode_typed_binary_frame::<RequestStartFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            let request =
                request_envelope_from_start_frame(header, payload).map_err(RuntimeError::Decode)?;
            host.spawn_request(request, sender.clone()).await;
        }
        "package-test.start" => {
            let (header, payload) = decode_typed_binary_frame::<PackageTestStartFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            host.submit_package_test_start(header, payload, sender.clone());
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
        "response.end" => {
            let (header, payload) = decode_typed_binary_frame::<ResponseEndFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if let Some(sender) = host.outbound_requests.sender(&header.request_id) {
                let _ = sender.send(response_end_to_outbound(&header, payload));
            } else {
                warn!(
                    event = "runtime.unmatched_outbound_response_end",
                    request_id = %header.request_id
                );
            }
        }
        "response.start" => {
            let (header, payload) = decode_typed_binary_frame::<ResponseStartFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "response.start binary frame payload must be empty".to_string(),
                ));
            }
            if let Some(sender) = host.outbound_requests.sender(&header.request_id) {
                let _ = sender.send(response_start_to_outbound(&header));
            } else {
                warn!(
                    event = "runtime.unmatched_outbound_response_start",
                    request_id = %header.request_id
                );
            }
        }
        "response.chunk" => {
            let (header, payload) = decode_typed_binary_frame::<ResponseChunkFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if let Some(sender) = host.outbound_requests.sender(&header.request_id) {
                let _ = sender.send(response_chunk_to_outbound(&header, payload));
            } else {
                warn!(
                    event = "runtime.unmatched_outbound_response_chunk",
                    request_id = %header.request_id,
                    payload_bytes = payload.len()
                );
            }
        }
        "response.error" => {
            let (header, payload) = decode_typed_binary_frame::<ResponseErrorFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            if !payload.is_empty() {
                return Err(RuntimeError::Decode(
                    "response.error binary frame payload must be empty".to_string(),
                ));
            }
            if let Some(sender) = host.outbound_requests.sender(&header.request_id) {
                let _ = sender.send(response_error_to_outbound(&header));
            } else {
                warn!(
                    event = "runtime.unmatched_outbound_response_error",
                    request_id = %header.request_id
                );
            }
        }
        "actor.put.response" => {
            let (header, payload) = decode_typed_binary_frame::<ActorPutResponseFrameHeader>(bytes)
                .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "actor.put.response",
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
        "spawn.submit.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<SpawnSubmitResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "spawn.submit.response",
            )?;
        }
        "spawn.claim.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<SpawnClaimResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            let rpc_id = header.rpc_id.clone();
            dispatch_spawn_claim_response(host, &rpc_id, header, payload)?;
        }
        "spawn.renew.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<SpawnRenewResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "spawn.renew.response",
            )?;
        }
        "spawn.complete.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<SpawnCompleteResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "spawn.complete.response",
            )?;
        }
        "spawn.fail.response" => {
            let (header, payload) =
                decode_typed_binary_frame::<SpawnFailResponseFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_response(
                host,
                &header.rpc_id,
                &header,
                payload,
                "spawn.fail.response",
            )?;
        }
        "actor.put.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.put.error",
            )?;
        }
        "actor.find.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
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
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "actor.remove.error",
            )?;
        }
        "spawn.submit.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "spawn.submit.error",
            )?;
        }
        "spawn.claim.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "spawn.claim.error",
            )?;
        }
        "spawn.renew.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "spawn.renew.error",
            )?;
        }
        "spawn.complete.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "spawn.complete.error",
            )?;
        }
        "spawn.fail.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
                    .map_err(super::transport_error_into_runtime_error)?;
            dispatch_control_error(
                host,
                &header.rpc_id,
                payload,
                header.error,
                "spawn.fail.error",
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

fn dispatch_spawn_claim_response(
    host: &super::RuntimeHost,
    rpc_id: &str,
    header: SpawnClaimResponseFrameHeader,
    payload: Vec<u8>,
) -> Result<()> {
    let payload = spawn_claim_response_control_payload(header, &payload)
        .map_err(super::transport_error_into_runtime_error)?;
    if let Some(sender) = host.outbound_requests.sender(rpc_id) {
        let _ = sender.send(OutboundResponse::End { payload });
    } else {
        warn!(
            event = "runtime.unmatched_outbound_control_response",
            envelope_type = "spawn.claim.response",
            rpc_id = %rpc_id
        );
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
    if let Some(sender) = host.outbound_requests.sender(rpc_id) {
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
    if let Some(sender) = host.outbound_requests.sender(rpc_id) {
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

fn router_control_typed_envelope_from_frame_header(
    header: RouterControlFrameHeader,
) -> TypedEnvelope {
    let mut rest = serde_json::Map::new();
    rest.insert(
        "artifactRoots".to_string(),
        Value::Array(
            header
                .artifact_roots
                .into_iter()
                .map(|root| Value::String(root.to_string_lossy().into_owned()))
                .collect(),
        ),
    );
    if let Some(dev_reload) = header.dev_reload {
        rest.insert("devReload".to_string(), Value::Bool(dev_reload));
    }
    if let Some(mode) = header.mode {
        rest.insert("mode".to_string(), Value::String(mode));
    }
    if let Some(generation) = header.generation {
        rest.insert("generation".to_string(), Value::String(generation));
    }
    if let Some(fingerprint) = header.fingerprint {
        rest.insert("fingerprint".to_string(), Value::String(fingerprint));
    }
    if !header.service_config.is_empty() {
        rest.insert(
            "serviceConfig".to_string(),
            serde_json::to_value(header.service_config).unwrap_or(Value::Null),
        );
    }
    if let Some(telemetry) = header.telemetry {
        rest.insert(
            "telemetry".to_string(),
            serde_json::to_value(telemetry).unwrap_or(Value::Null),
        );
    }
    if let Some(file_backend) = header.file_backend {
        rest.insert(
            "fileBackend".to_string(),
            serde_json::to_value(file_backend).unwrap_or(Value::Null),
        );
    }
    TypedEnvelope {
        envelope_type: "router.control".to_string(),
        rest,
    }
}

async fn run_writer_loop<S>(
    mut writer: S,
    mut receiver: mpsc::UnboundedReceiver<super::RouterWriterMessage>,
) where
    S: Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    while let Some(message) = receiver.recv().await {
        let message = match encode_writer_message(message) {
            Ok(message) => message,
            Err(error) => {
                error!(event = "runtime.encode_writer_message_error", error = %error);
                break;
            }
        };
        if let Err(error) = writer.send(message).await {
            error!(event = "runtime.write_error", error = %error);
            break;
        }
    }
}

fn encode_writer_message(message: super::RouterWriterMessage) -> Result<Message> {
    match message {
        super::RouterWriterMessage::Binary(bytes) => Ok(Message::Binary(bytes.into())),
        super::RouterWriterMessage::Control(command) => encode_outbound_control_message(command)
            .map_err(super::transport_error_into_runtime_error)
            .map(|bytes| Message::Binary(bytes.into())),
    }
}

fn reject_router_text_message(_text: &str) -> Result<()> {
    Err(RuntimeError::Decode(
        "text protocol messages are not supported on runtime WebSocket; use binary runtime frames"
            .to_string(),
    ))
}

async fn handle_router_control(
    host: &super::RuntimeHost,
    typed: TypedEnvelope,
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &mut Option<RouterControlEnvelope>,
    artifact_fingerprint: &mut Option<String>,
) -> Result<()> {
    let next_control: RouterControlEnvelope = serde_json::from_value(Value::Object(typed.rest))?;
    match host
        .reload_from_control(&next_control, sender.clone())
        .await
    {
        Ok(fingerprint) => {
            *artifact_fingerprint = Some(fingerprint);
            *control = Some(next_control);
        }
        Err(error) => {
            warn!(event = "runtime.router_control_load_failed", error = %error);
        }
    }
    Ok(())
}

async fn maybe_reload_dev_artifacts(
    host: &super::RuntimeHost,
    sender: &mpsc::UnboundedSender<super::RouterWriterMessage>,
    control: &RouterControlEnvelope,
    artifact_fingerprint: &mut Option<String>,
) {
    let artifact_roots = match control.ordered_artifact_roots() {
        Ok(artifact_roots) => artifact_roots,
        Err(error) => {
            warn!(
                event = "runtime.artifact_reload_fingerprint_error",
                error = %error
            );
            return;
        }
    };
    match artifact_roots_control_fingerprint(&artifact_roots, control.dev_reload) {
        Ok(fingerprint) if artifact_fingerprint.as_deref() == Some(fingerprint.as_str()) => {}
        Ok(_) => match host.reload_from_control(control, sender.clone()).await {
            Ok(next_fingerprint) => {
                *artifact_fingerprint = Some(next_fingerprint);
            }
            Err(error) => {
                warn!(event = "runtime.artifact_reload_failed", error = %error);
            }
        },
        Err(error) => {
            warn!(
                event = "runtime.artifact_reload_fingerprint_error",
                error = %error
            );
        }
    }
}

#[cfg(test)]
mod tests;

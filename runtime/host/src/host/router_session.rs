use std::collections::HashSet;

use futures_util::{Sink, SinkExt, StreamExt};
use serde::Serialize;
use serde_json::Value;
use skiff_runtime_request::{OutboundResponse, ResponseError};
#[cfg(test)]
use skiff_runtime_transport::protocol::RouterControlEnvelope;
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, AssemblyActivationFrameDirection,
        ASSEMBLY_ACTIVATION_FRAME_TYPE,
    },
    control_mapper::encode_outbound_control_message,
    control_response_mapper::spawn_claim_response_control_payload,
    protocol::{
        decode_router_bootstrap_frame_header, decode_typed_binary_frame,
        ActorFindResponseFrameHeader, ActorGetOrCreateResponseFrameHeader,
        ActorRemoveResponseFrameHeader, ActorReplaceResponseFrameHeader,
        ActorSpawnRuntimeErrorFrameHeader, RequestCancelFrameHeader, ResponseChunkFrameHeader,
        ResponseEndFrameHeader, ResponseErrorFrameHeader, ResponseStartFrameHeader,
        RuntimeErrorFramePayload, RuntimeHealthCountersFrameHeader, RuntimeRegisteredFrameHeader,
        SpawnClaimResponseFrameHeader, SpawnCompleteResponseFrameHeader,
        SpawnFailResponseFrameHeader, SpawnRenewResponseFrameHeader,
        SpawnSubmitResponseFrameHeader, TypedEnvelope,
    },
    request_mapper::request_cancel_from_frame_header,
    response_mapper::{
        response_chunk_to_outbound, response_end_to_outbound, response_error_to_outbound,
        response_start_to_outbound,
    },
    runtime_assembly_request::decode_runtime_assembly_request_start_frame,
    websocket_generation_lifecycle::WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
};
use tokio::{
    sync::mpsc,
    time::{Duration, MissedTickBehavior},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{error, info, warn};

use crate::error::{Result, RuntimeError};

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
    let router_session_id = format!("skiff-router-session-v1:opaque:{}", uuid::Uuid::new_v4());

    host.websocket_generations.connect(&router_session_id)?;
    let writer_task = tokio::spawn(run_writer_loop(writer, receiver));

    let session_result = async {
        let mut health_reporter = RuntimeHealthReporter::default();
        let mut bootstrap = None;
        let mut health_interval = tokio::time::interval(Duration::from_secs(1));
        health_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut health_zero_transition_interval = tokio::time::interval(Duration::from_millis(50));
        health_zero_transition_interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

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
                                &router_session_id,
                                &bytes,
                                &sender,
                                &mut health_reporter,
                                &mut bootstrap,
                            )
                            .await?;
                        }
                        _ => {}
                    }
                }
                _ = health_interval.tick(), if health_reporter.has_registered_runtimes() => {
                    health_reporter.send_periodic(&host, &sender).await?;
                }
                _ = health_zero_transition_interval.tick(), if health_reporter.should_probe_zero_transition() => {
                    health_reporter.send_zero_transition_if_needed(&host, &sender).await?;
                }
            }
        }

        health_reporter.send_final(&host, &sender).await
    }
    .await;

    host.discard_actor_instances_for_session(&router_session_id);
    let disconnect_result = host.websocket_generations.disconnect(&router_session_id);
    drop(sender);
    let _ = writer_task.await;
    session_result.and(disconnect_result)
}

struct ConnectionBootstrap {
    resolver: skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver,
    service_db: skiff_artifact_model::AssemblyActivationServiceDb,
    max_response_bytes: usize,
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
    let service_db = skiff_artifact_model::AssemblyActivationServiceDb {
        mongo_url: header.service_db.mongo_url.clone(),
    };
    Ok(ConnectionBootstrap {
        resolver,
        service_db,
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
        && counters.spawned_tasks_active == 0
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
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        max_response_bytes: 67_108_864,
    });
    dispatch_router_binary_frame_inner(
        host,
        "skiff-router-session-v1:opaque:test-session",
        bytes,
        sender,
        None,
        &mut bootstrap,
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
        service_db: skiff_artifact_model::AssemblyActivationServiceDb {
            mongo_url: "mongodb://127.0.0.1:27017".to_string(),
        },
        max_response_bytes,
    });
    dispatch_router_binary_frame_inner(
        host,
        "skiff-router-session-v1:opaque:test-session",
        bytes,
        sender,
        None,
        &mut bootstrap,
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
) -> Result<()> {
    dispatch_router_binary_frame_inner(
        host,
        router_session_id,
        bytes,
        sender,
        Some(health_reporter),
        bootstrap,
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
            host.recover_durable_committed(&installed.resolver, &installed.service_db)
                .await?;
            host.queue_connection_registration(sender.clone())?;
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
            if bootstrap.is_none() {
                return Err(RuntimeError::Decode(
                    "runtime.registered requires router.bootstrap first".to_string(),
                ));
            }
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
        "actor.getOrCreate.error" => {
            let (header, payload) =
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
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
                decode_typed_binary_frame::<ActorSpawnRuntimeErrorFrameHeader>(bytes)
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

#[cfg(test)]
mod tests;
